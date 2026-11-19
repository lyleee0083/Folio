use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::{EM_GETFIRSTVISIBLELINE, EM_REPLACESEL, EM_SETSEL};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

type Test<T> = Result<T, Box<dyn std::error::Error>>;

struct Search<'a> {
    process: u32,
    class: &'a str,
    found: HWND,
}

unsafe extern "system" fn examine(hwnd: HWND, parameter: LPARAM) -> BOOL {
    let candidate = &mut *(parameter.0 as *mut Search<'_>);
    let mut pid = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid != candidate.process {
        return BOOL(1);
    }
    let mut class = [0; 128];
    let read = GetClassNameW(hwnd, &mut class).max(0) as usize;
    if String::from_utf16_lossy(&class[..read]) != candidate.class {
        return BOOL(1);
    }
    candidate.found = hwnd;
    BOOL(0)
}

fn window(process: u32, class: &str) -> Option<HWND> {
    let mut search = Search {
        process,
        class,
        found: HWND::default(),
    };
    unsafe {
        let _ = EnumWindows(
            Some(examine),
            LPARAM((&mut search as *mut Search<'_>) as isize),
        );
    }
    (!search.found.0.is_null()).then_some(search.found)
}

struct Process(Child);

impl Process {
    fn start(exe: &Path, root: &Path, profile: &Path, path: Option<&Path>) -> Test<Self> {
        let mut command = Command::new(exe);
        command
            .current_dir(root)
            .env("FOLIO_TEST_USER_DATA", profile)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(file) = path {
            command.arg(file);
        }
        Ok(Self(command.spawn()?))
    }

    fn finish(&mut self) -> Test<()> {
        let status = wait("Folio process did not exit", || {
            self.0.try_wait().ok().flatten()
        })?;
        if !status.success() {
            return Err(io::Error::other(format!("Folio exited with {status}")).into());
        }
        Ok(())
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            if let Some(hwnd) = window(self.0.id(), "FolioNativeWindow") {
                let _ = post(hwnd, WM_CLOSE, 0);
            }
            let until = Instant::now() + Duration::from_millis(500);
            while Instant::now() < until {
                if self.0.try_wait().ok().flatten().is_some() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn wait<T>(message: &str, mut check: impl FnMut() -> Option<T>) -> Test<T> {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(value) = check() {
            return Ok(value);
        }
        if Instant::now() >= until {
            return Err(io::Error::other(message).into());
        }
        std::thread::sleep(Duration::from_millis(35));
    }
}

fn state(profile: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(profile.join("reader-library.json")).ok()?).ok()
}

fn post(hwnd: HWND, message: u32, value: usize) -> Test<()> {
    unsafe { Ok(PostMessageW(hwnd, message, WPARAM(value), LPARAM(0))?) }
}

fn send(hwnd: HWND, message: u32, value: usize, data: isize) -> Test<usize> {
    let mut result = 0;
    let status = unsafe {
        SendMessageTimeoutW(
            hwnd,
            message,
            WPARAM(value),
            LPARAM(data),
            SMTO_ABORTIFHUNG,
            2000,
            Some(&mut result),
        )
    };
    if status.0 == 0 {
        return Err(io::Error::other(format!("Win32 command 0x{message:x} timed out")).into());
    }
    Ok(result)
}

fn read_text(hwnd: HWND) -> Test<String> {
    let len = send(hwnd, WM_GETTEXTLENGTH, 0, 0)?;
    let mut buffer = vec![0u16; len + 1];
    let count = send(hwnd, WM_GETTEXT, buffer.len(), buffer.as_mut_ptr() as isize)?;
    Ok(String::from_utf16(&buffer[..count])?)
}

fn append(hwnd: HWND, text: &str) -> Test<()> {
    let text: Vec<_> = text.encode_utf16().chain([0]).collect();
    send(hwnd, EM_SETSEL, usize::MAX, -1)?;
    send(hwnd, EM_REPLACESEL, 1, text.as_ptr() as isize)?;
    Ok(())
}

fn encode(content: &str) -> Vec<u8> {
    [0xff, 0xfe]
        .into_iter()
        .chain(content.encode_utf16().flat_map(u16::to_le_bytes))
        .collect()
}

fn screenshot(hwnd: HWND, path: &Path) -> Test<()> {
    unsafe {
        let _ = RedrawWindow(
            hwnd,
            None,
            None,
            RDW_UPDATENOW | RDW_INVALIDATE | RDW_ALLCHILDREN,
        );
        std::thread::sleep(Duration::from_millis(120));
        let mut area = RECT::default();
        GetClientRect(hwnd, &mut area)?;
        let mut origin = POINT::default();
        if !ClientToScreen(hwnd, &mut origin).as_bool() {
            return Err(io::Error::other("Cannot locate Folio client area").into());
        }
        let (width, height) = (area.right, area.bottom);
        if width <= 0 || height <= 0 {
            return Err(io::Error::other("Empty screenshot bounds").into());
        }
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, width, height);
        let previous = SelectObject(dc, bitmap);
        let operation = (|| -> Test<()> {
            BitBlt(dc, 0, 0, width, height, screen, origin.x, origin.y, SRCCOPY)?;
            SelectObject(dc, previous);
            let mut info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut pixels = vec![0; width as usize * height as usize * 4];
            if GetDIBits(
                dc,
                bitmap,
                0,
                height as u32,
                Some(pixels.as_mut_ptr().cast()),
                &mut info,
                DIB_RGB_COLORS,
            ) != height
            {
                return Err(io::Error::other("Could not read Folio screenshot pixels").into());
            }
            for pixel in pixels.chunks_exact_mut(4) {
                pixel.swap(0, 2);
                pixel[3] = 255;
            }
            image::save_buffer(
                path,
                &pixels,
                width as u32,
                height as u32,
                image::ColorType::Rgba8,
            )?;
            Ok(())
        })();
        SelectObject(dc, previous);
        let _ = DeleteObject(bitmap);
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        operation
    }
}

fn preview_code_width(window: HWND, preview: HWND, path: &Path) -> Test<i32> {
    // Let the desktop's maximize/restore animation settle before sampling pixels.
    std::thread::sleep(Duration::from_millis(350));
    screenshot(window, path)?;
    let image = image::open(path)?.to_rgb8();
    let mut bounds = RECT::default();
    let mut origin = [POINT::default()];
    unsafe {
        GetClientRect(preview, &mut bounds)?;
        MapWindowPoints(preview, window, &mut origin);
    }
    let left = origin[0].x;
    let right = left + bounds.right;
    let mut first = right;
    let mut last = left;
    for (x, y, pixel) in image.enumerate_pixels() {
        let x = x as i32;
        let y = y as i32;
        if x >= left
            && x < right
            && y >= origin[0].y
            && y < origin[0].y + bounds.bottom
            && pixel.0 == [244, 246, 249]
        {
            first = first.min(x);
            last = last.max(x);
        }
    }
    let margin = (unsafe { GetDpiForWindow(preview) }.max(96) * 50 / 96) as i32;
    if first >= last || first - left > margin || right - last > margin {
        return Err(io::Error::other(format!(
            "Preview code card did not fill the available width: margins {} and {} px",
            first - left,
            right - last
        ))
        .into());
    }
    Ok(last - first + 1)
}

fn execute(
    exe: &Path,
    project: &Path,
    input: &Path,
    output: &Path,
    results: &mut Vec<String>,
) -> Test<()> {
    let profile = input.join("profile");
    let first = input.join("GFM sample.md");
    let second = input.join("已转交 文件.md");
    let source = "---\r\ncategory: native\r\n---\r\n\r\n# 中文 GFM\r\n\r\n- [x] 完成\r\n- [ ] 待办\n\r\n| 左侧 | 右侧 |\r\n| --- | ---: |\r\n| ~~旧稿~~ | 新稿 |\r\n\r\nhttps://example.com\r\n\r\n```rust\r\nfn main() { println!(\"native\"); }\r\n```\r\n\r\n$$x^2 + y^2$$\r\n\r\n<b>literal tag</b>\r\n";
    let bytes = encode(source);
    fs::write(&first, &bytes)?;
    let mut long_sample = String::from("# 长篇中文阅读样本\n\n");
    let paragraph = "这一段是人工生成的排版样本。用它检查中文文字、空行、自动换行和滚动位置。";
    for index in 1..=110 {
        long_sample.push_str(&paragraph.repeat(5));
        long_sample.push_str(&format!("（段落 {index:04}。）\n\n"));
    }
    fs::write(&second, long_sample)?;
    let mut app = Process::start(exe, project, &profile, Some(&first))?;
    let hwnd = wait("Native Folio window did not open", || {
        window(app.0.id(), "FolioNativeWindow")
    })?;
    unsafe {
        let _ = ShowWindow(hwnd, SW_RESTORE);
        let _ = SetForegroundWindow(hwnd);
    }
    wait("Initial source was not imported", || {
        state(&profile).filter(|value| value["Documents"][0]["Name"] == "GFM sample.md")
    })?;
    let editor = unsafe { GetDlgItem(hwnd, 27)? };
    if unsafe { IsWindowVisible(editor) }.as_bool() {
        return Err(io::Error::other("Raw was the default").into());
    }
    screenshot(hwnd, &output.join("preview.png"))?;
    results.push("PASS | Native window and default Preview".into());
    let preview =
        unsafe { FindWindowExW(hwnd, None, windows::core::w!("FolioDocumentPreview"), None)? };
    let mut normal = RECT::default();
    unsafe {
        GetWindowRect(hwnd, &mut normal)?;
        let _ = ShowWindow(hwnd, SW_MAXIMIZE);
    }
    let wide = preview_code_width(hwnd, preview, &output.join("preview-wide.png"))?;
    unsafe {
        let _ = ShowWindow(hwnd, SW_RESTORE);
        SetWindowPos(
            hwnd,
            None,
            0,
            0,
            (normal.right - normal.left) * 3 / 4,
            normal.bottom - normal.top,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        )?;
    }
    let narrow = preview_code_width(hwnd, preview, &output.join("preview-narrow.png"))?;
    send(hwnd, WM_COMMAND, 19, 0)?;
    let no_sidebar = preview_code_width(hwnd, preview, &output.join("preview-no-sidebar.png"))?;
    if wide <= narrow || no_sidebar <= narrow {
        return Err(
            io::Error::other("Preview did not reflow when the window or sidebar changed").into(),
        );
    }
    send(hwnd, WM_COMMAND, 19, 0)?;
    unsafe {
        SetWindowPos(
            hwnd,
            None,
            normal.left,
            normal.top,
            normal.right - normal.left,
            normal.bottom - normal.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )?;
    }
    results.push(
        "PASS | Preview fills the visible pane after maximize, resize, and sidebar toggle".into(),
    );

    post(hwnd, WM_COMMAND, 12)?;
    wait("Raw editor did not appear", || {
        unsafe { IsWindowVisible(editor) }.as_bool().then_some(())
    })?;
    if !read_text(editor)?.contains("中文 GFM") {
        return Err(io::Error::other("Raw source did not preserve Unicode").into());
    }
    append(editor, "\r\nRust editor saved.")?;
    if fs::read(&first)? != bytes {
        return Err(io::Error::other("Raw edited disk before save").into());
    }
    screenshot(hwnd, &output.join("raw.png"))?;
    post(hwnd, WM_SYSCOMMAND, SC_CLOSE as usize)?;
    let prompt = wait("Caption close did not ask to save the draft", || {
        window(app.0.id(), "#32770")
    })?;
    post(prompt, WM_COMMAND, IDCANCEL.0 as usize)?;
    wait("Cancel did not close the save prompt", || {
        (!unsafe { IsWindow(prompt) }.as_bool()).then_some(())
    })?;
    if !read_text(editor)?.ends_with("Rust editor saved.") || fs::read(&first)? != bytes {
        return Err(io::Error::other("Cancel did not preserve the draft in memory").into());
    }
    results.push("PASS | Cancel on caption close keeps the unsaved draft".into());
    post(hwnd, WM_COMMAND, 13)?;
    wait(
        "Save did not write the original file and preserve its bytes",
        || {
            fs::read(&first).ok().filter(|saved| {
                saved.starts_with(&bytes) && saved.ends_with(&encode("Rust editor saved.")[2..])
            })
        },
    )?;
    results.push("PASS | RichEdit draft, explicit save, UTF-16 BOM and mixed endings".into());

    post(hwnd, WM_COMMAND, 12)?;
    wait("Preview did not return after saving", || {
        (!unsafe { IsWindowVisible(editor) }.as_bool()).then_some(())
    })?;
    results.push("PASS | Preview returns after save".into());

    unsafe {
        let _ = ShowWindowAsync(hwnd, SW_HIDE);
    }
    wait("Test could not hide the first Folio window", || {
        (!unsafe { IsWindowVisible(hwnd) }.as_bool()).then_some(())
    })?;
    let mut forwarded = Process::start(exe, project, &profile, Some(&second))?;
    forwarded.finish()?;
    wait("Named pipe did not forward to the same window", || {
        state(&profile).filter(|s| {
            s["Documents"].as_array().is_some_and(|documents| {
                documents.len() == 2
                    && documents
                        .iter()
                        .any(|doc| doc["Id"] == s["ActiveId"] && doc["Name"] == "已转交 文件.md")
            })
        })
    })?;
    if window(app.0.id(), "FolioNativeWindow") != Some(hwnd) {
        return Err(io::Error::other("Instance changed HWND").into());
    }
    wait("Relaunch did not restore the hidden Folio window", || {
        unsafe { IsWindowVisible(hwnd) }.as_bool().then_some(())
    })?;
    results.push(
        "PASS | Existing hidden window is restored and receives the Unicode launch path".into(),
    );
    let filter = unsafe { GetDlgItem(hwnd, 20)? };
    let list = unsafe { GetDlgItem(hwnd, 21)? };
    let query: Vec<_> = "已转交".encode_utf16().chain([0]).collect();
    send(filter, WM_SETTEXT, 0, query.as_ptr() as isize)?;
    if send(list, LB_GETCOUNT, 0, 0)? != 1 {
        return Err(io::Error::other("Document filter did not narrow the library").into());
    }
    send(filter, WM_SETTEXT, 0, [0u16].as_ptr() as isize)?;
    if send(list, LB_GETCOUNT, 0, 0)? != 2 {
        return Err(io::Error::other("Clearing the filter did not restore the library").into());
    }
    results.push("PASS | Unicode document filter and reset".into());
    screenshot(hwnd, &output.join("chinese-prose.png"))?;

    post(hwnd, WM_COMMAND, 16)?;
    wait("Theme was not persisted", || {
        state(&profile).filter(|value| value["Options"]["IsDark"] == true)
    })?;
    screenshot(hwnd, &output.join("dark.png"))?;
    post(hwnd, WM_COMMAND, 12)?;
    wait("Long document did not enter Raw", || {
        unsafe { IsWindowVisible(editor) }.as_bool().then_some(())
    })?;
    if !read_text(editor)?.starts_with("# 长篇中文阅读样本") {
        return Err(io::Error::other("Long Chinese source was not loaded in Raw").into());
    }
    screenshot(hwnd, &output.join("raw-chinese-dark.png"))?;
    results.push("PASS | Long Chinese source in the dark Raw editor".into());
    post(hwnd, WM_COMMAND, 14)?;
    let query_box = unsafe { GetDlgItem(hwnd, 23)? };
    wait("Raw search field did not appear", || {
        unsafe { IsWindowVisible(query_box) }
            .as_bool()
            .then_some(())
    })?;
    let gutter =
        unsafe { FindWindowExW(hwnd, None, windows::core::w!("FolioRawLineNumbers"), None)? };
    let before = output.join("raw-gutter-before.png");
    let after = output.join("raw-gutter-after.png");
    screenshot(gutter, &before)?;
    let query: Vec<_> = "段落 0094。".encode_utf16().chain([0]).collect();
    send(query_box, WM_SETTEXT, 0, query.as_ptr() as isize)?;
    let late_row = wait("Raw search did not scroll to the late paragraph", || {
        send(editor, EM_GETFIRSTVISIBLELINE, 0, 0)
            .ok()
            .filter(|line| *line > 100)
    })?;
    screenshot(gutter, &after)?;
    screenshot(hwnd, &output.join("raw-chinese-scrolled.png"))?;
    let moved_gutter = image::open(after)?.to_rgb8();
    if image::open(before)?.to_rgb8() == moved_gutter {
        return Err(
            io::Error::other("Raw gutter stayed unchanged after the editor scrolled").into(),
        );
    }
    results.push(
        "PASS | Raw gutter updates after a search jumps beyond the first 100 visual rows".into(),
    );
    send(editor, WM_VSCROLL, SB_PAGEUP.0 as usize, 0)?;
    wait("Scrolling up did not move the Raw editor", || {
        send(editor, EM_GETFIRSTVISIBLELINE, 0, 0)
            .ok()
            .filter(|line| *line < late_row)
    })?;
    let page_up = output.join("raw-gutter-page-up.png");
    screenshot(gutter, &page_up)?;
    if image::open(page_up)?.to_rgb8() == moved_gutter {
        return Err(io::Error::other("Raw gutter stayed unchanged after paging up").into());
    }
    results.push("PASS | Raw gutter follows the vertical scrollbar from a late page".into());
    post(hwnd, WM_SYSCOMMAND, SC_CLOSE as usize)?;
    wait("Caption close did not destroy the Folio window", || {
        (!unsafe { IsWindow(hwnd) }.as_bool()).then_some(())
    })?;
    app.finish()?;
    results.push("PASS | Caption close destroys the window and exits the process".into());
    let closed = state(&profile).ok_or_else(|| io::Error::other("No persisted state"))?;
    if closed["Documents"] != serde_json::json!([]) || !closed["ActiveId"].is_null() {
        return Err(io::Error::other("Clean exit left document history").into());
    }
    let mut reopened = Process::start(exe, project, &profile, None)?;
    let next = wait("Folio did not reopen", || {
        window(reopened.0.id(), "FolioNativeWindow")
    })?;
    if state(&profile).is_none_or(|value| {
        value["Options"]["IsDark"] != true || value["Documents"] != serde_json::json!([])
    }) {
        return Err(
            io::Error::other("Reopen lost the selected theme or restored old documents").into(),
        );
    }
    post(next, WM_CLOSE, 0)?;
    reopened.finish()?;
    results.push("PASS | Reopen preserves preferences and starts with an empty library".into());
    Ok(())
}

fn run() -> Test<()> {
    let mut args = std::env::args_os().skip(1).map(PathBuf::from);
    let exe = fs::canonicalize(
        args.next()
            .ok_or_else(|| io::Error::other("FolioVerify requires the Folio executable path"))?,
    )?;
    let project = fs::canonicalize(
        args.next()
            .ok_or_else(|| io::Error::other("FolioVerify requires the project path"))?,
    )?;
    if !exe.starts_with(&project) || !project.join("src/rust/Cargo.toml").is_file() {
        return Err(io::Error::other("FolioVerify must run from the Folio project").into());
    }
    let id = format!(
        "{}-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
        std::process::id()
    );
    let input = project
        .join("work/2026-11-18-rust-rewrite/verification")
        .join(&id);
    let output = project
        .join("outputs/2026-11-18-rust-rewrite/runs")
        .join(&id);
    fs::create_dir_all(&input)?;
    fs::create_dir_all(&output)?;
    let mut results = Vec::new();
    let result = execute(&exe, &project, &input, &output, &mut results);
    let hash = Sha256::digest(fs::read(&exe)?)
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<String>();
    results.push(format!("Executable SHA-256 {hash}"));
    match &result {
        Ok(()) => results.push("Result SUCCESS".into()),
        Err(error) => {
            results.push("Result FAILURE".into());
            results.push(error.to_string());
        }
    }
    let evidence = results.join("\n") + "\n";
    fs::write(output.join("result.txt"), &evidence)?;
    print!("{evidence}Evidence {}\n", output.display());
    result
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
