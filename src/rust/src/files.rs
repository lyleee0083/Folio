use encoding_rs::GB18030;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use similar::{ChangeTag, TextDiff};
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read, Seek, Write};
use std::os::windows::{
    ffi::OsStrExt,
    fs::{MetadataExt, OpenOptionsExt},
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{
    MoveFileExW, ReplaceFileW, MOVEFILE_WRITE_THROUGH, REPLACE_FILE_FLAGS,
};

pub const FILE_LIMIT: usize = 10 * 1024 * 1024;
pub const LIBRARY_BYTES_LIMIT: usize = 256 * 1024 * 1024;
const LIBRARY_LIMIT: usize = 2000;
const STATE_LIMIT: usize = 16 * 1024 * 1024;
const LEGACY_STATE_LIMIT: usize = LIBRARY_BYTES_LIMIT * 6;
const REPARSE_POINT: u32 = 0x400;
const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
static TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Encoding {
    #[default]
    #[serde(rename = "utf-8")]
    Utf8,
    #[serde(rename = "utf-8-bom")]
    Utf8Bom,
    #[serde(rename = "utf-16le-bom")]
    Utf16Le,
    #[serde(rename = "utf-16be-bom")]
    Utf16Be,
    #[serde(rename = "utf-32le-bom")]
    Utf32Le,
    #[serde(rename = "utf-32be-bom")]
    Utf32Be,
    #[serde(rename = "gb18030")]
    Gb18030,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub source_path: PathBuf,
    #[serde(skip)]
    pub content: String,
    pub text_encoding: Encoding,
    pub reading_progress: f64,
    #[serde(skip)]
    pub checksum: [u8; 32],
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct Options {
    pub font_size: f32,
    pub line_height: f32,
    pub content_width: f32,
    pub auto_width: bool,
    pub is_dark: bool,
    pub font_family: String,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            font_size: 16.,
            line_height: 1.6,
            content_width: 760.,
            auto_width: true,
            is_dark: false,
            font_family: "sans".into(),
        }
    }
}

impl Options {
    pub fn normalize(&mut self) {
        self.font_size = self.font_size.clamp(14., 26.);
        self.line_height = self.line_height.clamp(1.4, 2.2);
        self.content_width = self.content_width.clamp(540., 960.);
        if !matches!(self.font_family.as_str(), "sans" | "serif" | "mono") {
            self.font_family = "sans".into();
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct State {
    pub documents: Vec<Entry>,
    pub active_id: Option<String>,
    pub options: Options,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
    pub sidebar_open: bool,
    pub outline_open: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            documents: vec![],
            active_id: None,
            options: Options::default(),
            width: 1280.,
            height: 840.,
            maximized: false,
            sidebar_open: true,
            outline_open: false,
        }
    }
}

impl State {
    pub fn preferences(&self) -> Self {
        Self {
            options: self.options.clone(),
            width: self.width,
            height: self.height,
            maximized: self.maximized,
            sidebar_open: self.sidebar_open,
            outline_open: self.outline_open,
            ..Self::default()
        }
    }
}

pub struct Store {
    pub root: PathBuf,
}

impl Store {
    pub fn new() -> io::Result<Self> {
        let root = PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default())
            .join("Folio")
            .join("native");
        let root = std::path::absolute(root)?;
        ensure_no_reparse(&root)?;
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    pub fn load(&self) -> State {
        let current = self.root.join("reader-library.json");
        let backup = self.root.join("reader-library.json.bak");
        let state = read_state(&current).or_else(|_| read_state(&backup));
        if let Ok(state) = state {
            return state;
        }
        let legacy = self.root.join("reader-state.json");
        if let Ok(state) = read_state(&legacy) {
            return state.preferences();
        }
        if !current.exists() && !legacy.exists() {
            if let Some(roaming) = self.root.parent().and_then(Path::parent) {
                let old = roaming.join("mdreader").join("native");
                for name in ["reader-library.json", "reader-state.json"] {
                    if let Ok(state) = read_state(&old.join(name)) {
                        return state.preferences();
                    }
                }
            }
        }
        State::default()
    }

    pub fn save(&self, state: &State, closing: bool) -> io::Result<()> {
        let bytes = if closing {
            serde_json::to_vec(&state.preferences())?
        } else {
            serde_json::to_vec(state)?
        };
        if bytes.len() > STATE_LIMIT {
            return Err(io::Error::other("阅读状态超过大小限制"));
        }
        atomic_write(&self.root.join("reader-library.json"), &bytes)
    }

    // Older versions wrote full document bodies to this backup. Rewrite valid
    // backups as preferences once, before starting the regular save timer.
    pub fn migrate_backup(&self) -> io::Result<()> {
        let backup = self.root.join("reader-library.json.bak");
        if let Ok(state) = read_state(&backup) {
            let bytes = serde_json::to_vec(&state.preferences())?;
            atomic_write(&backup, &bytes)?;
        }
        Ok(())
    }
}

fn read_state(path: &Path) -> io::Result<State> {
    ensure_no_reparse(path)?;
    let file = File::open(path)?;
    if file.metadata()?.len() > LEGACY_STATE_LIMIT as u64 {
        return Err(io::Error::other("阅读状态超过大小限制"));
    }
    use std::io::BufRead;
    let mut reader = BufReader::new(file.take(LEGACY_STATE_LIMIT as u64 + 1));
    if reader.fill_buf()?.starts_with(&[0xef, 0xbb, 0xbf]) {
        reader.consume(3);
    }
    let mut state: State = serde_json::from_reader(reader)?;
    state.documents.truncate(LIBRARY_LIMIT);
    state.options.normalize();
    Ok(state)
}

pub fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain([0]).collect()
}

pub fn checksum(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

pub fn ensure_no_reparse(path: &Path) -> io::Result<()> {
    for part in path.ancestors() {
        match fs::symlink_metadata(part) {
            Ok(info) if info.file_attributes() & REPARSE_POINT != 0 => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "路径含符号链接或重解析点",
                ))
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub fn open_bounded(path: &Path, maximum: usize) -> io::Result<File> {
    ensure_no_reparse(path)?;
    let file = File::open(path)?;
    if file.metadata()?.len() > maximum as u64 {
        return Err(io::Error::other("文件超过大小限制"));
    }
    Ok(file)
}

pub fn read_bounded(path: &Path, maximum: usize) -> io::Result<Vec<u8>> {
    let mut result = Vec::new();
    open_bounded(path, maximum)?
        .take(maximum as u64 + 1)
        .read_to_end(&mut result)?;
    if result.len() > maximum {
        return Err(io::Error::other("文件超过大小限制"));
    }
    Ok(result)
}

pub fn open(path: &Path) -> io::Result<Entry> {
    let absolute = std::path::absolute(path)?;
    let bytes = read_bounded(&absolute, FILE_LIMIT)?;
    let (content, text_encoding) = decode(&bytes)?;
    let hash = checksum(&bytes);
    Ok(Entry {
        id: path_key(&absolute),
        name: absolute
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        source_path: absolute,
        content,
        text_encoding,
        reading_progress: 0.,
        checksum: hash,
    })
}

pub fn import(paths: &[PathBuf], existing: &[Entry]) -> Vec<Entry> {
    let mut pending: Vec<_> = paths.iter().rev().cloned().collect();
    let mut visited = HashSet::new();
    let known: HashMap<_, _> = existing
        .iter()
        .map(|item| (path_key(&item.source_path), item.content.len()))
        .collect();
    let mut result = Vec::new();
    let mut size = existing
        .iter()
        .map(|item| item.content.len())
        .sum::<usize>();
    let mut new = existing.len();
    while let Some(path) = pending.pop() {
        let Ok(path) = std::path::absolute(path) else {
            continue;
        };
        if !visited.insert(path_key(&path)) || ensure_no_reparse(&path).is_err() {
            continue;
        }
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        if metadata.is_dir() {
            if let Ok(entries) = fs::read_dir(&path) {
                let mut children: Vec<_> = entries
                    .flatten()
                    .filter(|item| {
                        !matches!(
                            item.file_name().to_string_lossy().to_lowercase().as_str(),
                            "work" | "runtime" | "待删除" | ".git" | "node_modules"
                        )
                    })
                    .map(|item| item.path())
                    .collect();
                children.sort();
                pending.extend(children.into_iter().rev());
            }
            continue;
        }
        if !is_markdown(&path) || metadata.len() > FILE_LIMIT as u64 {
            continue;
        }
        let previous = known.get(&path_key(&path));
        if previous.is_none() && new >= LIBRARY_LIMIT {
            continue;
        }
        let retained = size - previous.copied().unwrap_or_default();
        if let Ok(entry) = open(&path) {
            let updated = retained + entry.content.len();
            if updated > LIBRARY_BYTES_LIMIT {
                continue;
            }
            size = updated;
            if previous.is_none() {
                new += 1;
            }
            result.push(entry);
        }
    }
    result
}

pub fn is_markdown(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(OsStr::to_str)
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "md" | "markdown" | "mdown" | "txt"
    )
}

pub fn path_key(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_uppercase()
}

pub fn decode(bytes: &[u8]) -> io::Result<(String, Encoding)> {
    let malformed = || io::Error::new(io::ErrorKind::InvalidData, "文件编码包含无效字符");
    for (tag, encoding, step, little) in [
        (&[0xff, 0xfe, 0, 0][..], Encoding::Utf32Le, 4, true),
        (&[0, 0, 0xfe, 0xff][..], Encoding::Utf32Be, 4, false),
        (&[0xff, 0xfe][..], Encoding::Utf16Le, 2, true),
        (&[0xfe, 0xff][..], Encoding::Utf16Be, 2, false),
    ] {
        if let Some(raw) = bytes.strip_prefix(tag) {
            if raw.len() % step != 0 {
                return Err(malformed());
            }
            let value = if step == 2 {
                let utf16: Vec<_> = raw
                    .chunks_exact(2)
                    .map(|word| {
                        if little {
                            u16::from_le_bytes([word[0], word[1]])
                        } else {
                            u16::from_be_bytes([word[0], word[1]])
                        }
                    })
                    .collect();
                String::from_utf16(&utf16).map_err(|_| malformed())?
            } else {
                raw.chunks_exact(4)
                    .map(|word| {
                        let array = [word[0], word[1], word[2], word[3]];
                        char::from_u32(if little {
                            u32::from_le_bytes(array)
                        } else {
                            u32::from_be_bytes(array)
                        })
                        .ok_or_else(malformed)
                    })
                    .collect::<io::Result<String>>()?
            };
            return Ok((value, encoding));
        }
    }
    if let Some(raw) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        return Ok((
            std::str::from_utf8(raw).map_err(|_| malformed())?.into(),
            Encoding::Utf8Bom,
        ));
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Ok((text.into(), Encoding::Utf8));
    }
    GB18030
        .decode_without_bom_handling_and_without_replacement(bytes)
        .map(|value| (value.into_owned(), Encoding::Gb18030))
        .ok_or_else(malformed)
}

pub fn encode(content: &str, encoding: Encoding) -> io::Result<Vec<u8>> {
    let mut bytes = match encoding {
        Encoding::Utf8 => Vec::new(),
        Encoding::Utf8Bom => vec![0xef, 0xbb, 0xbf],
        Encoding::Utf16Le => vec![0xff, 0xfe],
        Encoding::Utf16Be => vec![0xfe, 0xff],
        Encoding::Utf32Le => vec![0xff, 0xfe, 0, 0],
        Encoding::Utf32Be => vec![0, 0, 0xfe, 0xff],
        Encoding::Gb18030 => {
            let (value, _, bad) = GB18030.encode(content);
            if bad {
                return Err(io::Error::other("无法用原编码保存这些字符"));
            }
            return Ok(value.into_owned());
        }
    };
    match encoding {
        Encoding::Utf8 | Encoding::Utf8Bom => bytes.extend_from_slice(content.as_bytes()),
        Encoding::Utf16Le | Encoding::Utf16Be => {
            for code in content.encode_utf16() {
                bytes.extend(if encoding == Encoding::Utf16Le {
                    code.to_le_bytes()
                } else {
                    code.to_be_bytes()
                });
            }
        }
        _ => {
            for c in content.chars() {
                let code = c as u32;
                bytes.extend(if encoding == Encoding::Utf32Le {
                    code.to_le_bytes()
                } else {
                    code.to_be_bytes()
                });
            }
        }
    }
    Ok(bytes)
}

pub fn normalize_source(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

pub fn restore_endings(original: &str, editor: &str) -> String {
    let normalized = normalize_source(original);
    if normalized == editor {
        return original.to_owned();
    }
    let mut endings = Vec::new();
    let mut bytes = original.bytes().peekable();
    while let Some(byte) = bytes.next() {
        match byte {
            b'\r' if bytes.peek() == Some(&b'\n') => {
                bytes.next();
                endings.push("\r\n");
            }
            b'\r' => endings.push("\r"),
            b'\n' => endings.push("\n"),
            _ => {}
        }
    }
    let fallback = endings.first().copied().unwrap_or("\r\n");
    let edit = TextDiff::configure()
        .timeout(Duration::from_millis(400))
        .diff_lines(normalized.as_str(), editor);
    let mut restored = String::with_capacity(editor.len());
    let mut original_line = 0;
    for change in edit.iter_all_changes() {
        if let Some(line) = change.old_index() {
            original_line = line;
        }
        if change.tag() == ChangeTag::Delete {
            continue;
        }
        let line = change.value();
        if let Some(body) = line.strip_suffix('\n') {
            restored.push_str(body);
            restored.push_str(endings.get(original_line).copied().unwrap_or(fallback));
        } else {
            restored.push_str(line);
        }
    }
    restored
}

pub fn save_document(entry: &mut Entry, editor: &str) -> io::Result<()> {
    let content = restore_endings(&entry.content, editor);
    let bytes = encode(&content, entry.text_encoding)?;
    if bytes.len() > FILE_LIMIT {
        return Err(io::Error::other("保存后的文件将超过 10 MiB"));
    }
    replace_document(&entry.source_path, &bytes, entry.checksum)?;
    entry.checksum = checksum(&bytes);
    entry.content = content;
    Ok(())
}

fn sibling_file(path: &Path, suffix: &str) -> io::Result<(PathBuf, File)> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("无效的保存位置"))?;
    let name: String = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .chars()
        .take(75)
        .collect();
    loop {
        let sibling = parent.join(format!(
            ".folio-{}-{}-{name}.{suffix}",
            std::process::id(),
            TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&sibling)
        {
            Ok(file) => return Ok((sibling, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

fn write_document_bytes(file: &mut File, bytes: &[u8]) -> io::Result<()> {
    file.rewind()?;
    file.write_all(bytes)?;
    file.set_len(bytes.len() as u64)?;
    file.sync_all()
}

fn replace_document(path: &Path, bytes: &[u8], expected: [u8; 32]) -> io::Result<()> {
    ensure_no_reparse(path)?;
    // Deny any other write, read or rename from the checksum check through
    // the final flush. All writes go to the exact file object we checked.
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .custom_flags(OPEN_REPARSE_POINT)
        .open(path)?;
    if file.metadata()?.file_attributes() & REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "无效的保存位置",
        ));
    }
    let mut original = Vec::new();
    (&mut file)
        .take(FILE_LIMIT as u64 + 1)
        .read_to_end(&mut original)?;
    if original.len() > FILE_LIMIT || checksum(&original) != expected {
        return Err(io::Error::other(
            "原文件已在 Folio 外部修改，当前编辑内容仍在 Raw 中。",
        ));
    }

    let (backup_path, mut backup) = sibling_file(path, "recovery.bak")?;
    let staged = backup.write_all(&original).and_then(|_| backup.sync_all());
    drop(backup);
    if let Err(error) = staged {
        let _ = fs::remove_file(&backup_path);
        return Err(error);
    }

    if let Err(error) = write_document_bytes(&mut file, bytes) {
        if let Err(recovery_error) = write_document_bytes(&mut file, &original) {
            return Err(io::Error::other(format!(
                "保存失败，请使用以下备份恢复原文件: {error}; {recovery_error}. {}",
                backup_path.display()
            )));
        }
        let _ = fs::remove_file(&backup_path);
        return Err(error);
    }
    let _ = fs::remove_file(backup_path);
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    ensure_no_reparse(path)?;
    let (temporary, mut file) = sibling_file(path, "tmp")?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        ensure_no_reparse(path)?;
        let destination = wide(path);
        let source = wide(&temporary);
        let result = unsafe {
            if path.exists() {
                ReplaceFileW(
                    PCWSTR(destination.as_ptr()),
                    PCWSTR(source.as_ptr()),
                    PCWSTR::null(),
                    REPLACE_FILE_FLAGS(0),
                    None,
                    None,
                )
            } else {
                MoveFileExW(
                    PCWSTR(source.as_ptr()),
                    PCWSTR(destination.as_ptr()),
                    MOVEFILE_WRITE_THROUGH,
                )
            }
        };
        result.map_err(|err| io::Error::other(err.to_string()))
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work/file-tests");
            fs::create_dir_all(&base).unwrap();
            loop {
                let path = base.join(format!(
                    "{}-{}",
                    std::process::id(),
                    TEMP_ID.fetch_add(1, Ordering::Relaxed)
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("{error}"),
                }
            }
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn preserves_encoding_and_endings_when_saving() {
        let dir = TestDirectory::new();
        let path = dir.0.join("original.md");
        fs::write(&path, b"\xef\xbb\xbf# Original\r\n\r\nBody\n").unwrap();
        let mut entry = open(&path).unwrap();
        save_document(&mut entry, "# Original\n\nEdited\n").unwrap();
        assert_eq!(
            fs::read(&path).unwrap(),
            b"\xef\xbb\xbf# Original\r\n\r\nEdited\n"
        );
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    }

    #[test]
    fn detects_a_replaced_file_and_a_busy_file() {
        let dir = TestDirectory::new();
        let path = dir.0.join("original.md");
        fs::write(&path, b"original").unwrap();
        let mut entry = open(&path).unwrap();
        atomic_write(&path, b"external").unwrap();
        assert!(save_document(&mut entry, "local").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"external");
        assert_eq!(entry.content, "original");

        let mut entry = open(&path).unwrap();
        let locked = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .unwrap();
        assert!(save_document(&mut entry, "local").is_err());
        drop(locked);
        assert_eq!(fs::read(&path).unwrap(), b"external");
        save_document(&mut entry, "local").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"local");
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    }

    #[test]
    fn loads_legacy_paths_and_removes_cached_bodies_from_state() {
        let dir = TestDirectory::new();
        let current = dir.0.join("reader-library.json");
        let backup = dir.0.join("reader-library.json.bak");
        let legacy = br#"{"Documents":[{"Id":"draft","Name":"draft.md","SourcePath":"C:/draft.md","Content":"cached markdown"}],"ActiveId":"draft"}"#;
        fs::write(&current, legacy).unwrap();
        fs::write(&backup, legacy).unwrap();
        let store = Store {
            root: dir.0.clone(),
        };
        let mut state = store.load();
        assert_eq!(state.documents[0].id, "draft");
        assert_eq!(state.documents[0].source_path, PathBuf::from("C:/draft.md"));
        assert!(state.documents[0].content.is_empty());
        state.documents[0].content = "in-memory markdown".into();
        store.save(&state, false).unwrap();
        store.migrate_backup().unwrap();
        let saved = fs::read_to_string(&current).unwrap();
        assert!(saved.contains("draft.md"));
        let json: serde_json::Value = serde_json::from_str(&saved).unwrap();
        assert!(json["Documents"][0].get("Content").is_none());
        assert!(!saved.contains("in-memory markdown"));
        assert!(!fs::read_to_string(&backup).unwrap().contains("markdown"));
        store.save(&state, true).unwrap();
        assert!(store.load().documents.is_empty());
    }
}
