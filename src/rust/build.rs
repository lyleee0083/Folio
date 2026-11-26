use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;

fn main() -> io::Result<()> {
    println!("cargo:rerun-if-changed=assets/app.ico");
    println!("cargo:rerun-if-changed=assets/folio.manifest");
    let out = PathBuf::from(
        env::var_os("OUT_DIR").ok_or_else(|| io::Error::other("OUT_DIR is missing"))?,
    );
    let resource = out.join("folio.res");
    let contents = fs::read("assets/app.ico")?;
    let mut records = Vec::new();
    entry(&mut records, 0, 0, &[]);
    let icon_atlas = icons(&contents, &mut records)?;
    entry(&mut records, 14, 1, &icon_atlas);
    // Windows assembly identities require four version components.
    let manifest = fs::read_to_string("assets/folio.manifest")?.replace(
        "@FOLIO_ASSEMBLY_VERSION@",
        concat!(env!("CARGO_PKG_VERSION"), ".0"),
    );
    entry(&mut records, 24, 1, manifest.as_bytes());
    entry(&mut records, 16, 1, &version_info());
    fs::write(&resource, records)?;
    println!("cargo:rustc-link-arg-bins={}", resource.display());
    Ok(())
}

fn version_info() -> Vec<u8> {
    let major: u16 = env!("CARGO_PKG_VERSION_MAJOR").parse().expect("Invalid major version");
    let minor: u16 = env!("CARGO_PKG_VERSION_MINOR").parse().expect("Invalid minor version");
    let patch: u16 = env!("CARGO_PKG_VERSION_PATCH").parse().expect("Invalid patch version");
    let version_ms = (u32::from(major) << 16) | u32::from(minor);
    let version_ls = u32::from(patch) << 16;

    // VS_FIXEDFILEINFO stores file and product versions as pairs of DWORDs.
    let fixed: Vec<u8> = [
        0xFEEF04BD, 0x00010000, version_ms, version_ls, version_ms, version_ls,
        0x3F, 0, 0x00040004, 1, 0, 0, 0,
    ]
    .into_iter()
    .flat_map(u32::to_le_bytes)
    .collect();

    let version: Vec<u8> = env!("CARGO_PKG_VERSION")
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect();
    let mut strings = Vec::new();
    for key in ["FileVersion", "ProductVersion"] {
        version_block(&mut strings, key, true, &version, &[]);
    }
    let mut table = Vec::new();
    version_block(&mut table, "040904B0", true, &[], &strings);

    let mut translation = Vec::new();
    // English (United States), Unicode.
    version_block(&mut translation, "Translation", false, &[0x09, 0x04, 0xB0, 0x04], &[]);
    let mut children = Vec::new();
    version_block(&mut children, "StringFileInfo", true, &[], &table);
    version_block(&mut children, "VarFileInfo", true, &[], &translation);

    let mut root = Vec::new();
    version_block(&mut root, "VS_VERSION_INFO", false, &fixed, &children);
    root
}

fn version_block(output: &mut Vec<u8>, key: &str, is_text: bool, value: &[u8], children: &[u8]) {
    output.resize((output.len() + 3) & !3, 0);
    let start = output.len();
    let value_length = if is_text { value.len() / 2 } else { value.len() };
    output.extend(0u16.to_le_bytes());
    output.extend((value_length as u16).to_le_bytes());
    output.extend(u16::from(is_text).to_le_bytes());
    output.extend(key.encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
    output.resize((output.len() + 3) & !3, 0);
    output.extend(value);
    if !children.is_empty() {
        output.resize((output.len() + 3) & !3, 0);
        output.extend(children);
    }
    let length = (output.len() - start) as u16;
    output[start..start + 2].copy_from_slice(&length.to_le_bytes());
}

fn icons(contents: &[u8], output: &mut Vec<u8>) -> io::Result<Vec<u8>> {
    let bad = || io::Error::new(io::ErrorKind::InvalidData, "Invalid ICO file");
    let header = contents.get(..6).ok_or_else(bad)?;
    if header[..4] != [0, 0, 1, 0] {
        return Err(bad());
    }
    let count = u16::from_le_bytes([header[4], header[5]]) as usize;
    let table = contents.get(6..6 + count * 16).ok_or_else(bad)?;
    let mut group = header.to_vec();
    for (index, row) in table.chunks_exact(16).enumerate() {
        let len = u32::from_le_bytes([row[8], row[9], row[10], row[11]]) as usize;
        let start = u32::from_le_bytes([row[12], row[13], row[14], row[15]]) as usize;
        let icon = contents
            .get(start..start.checked_add(len).ok_or_else(bad)?)
            .ok_or_else(bad)?;
        let id = index as u16 + 1;
        group.extend_from_slice(&row[..12]);
        group.extend(id.to_le_bytes());
        entry(output, 3, id, icon);
    }
    Ok(group)
}

fn entry(output: &mut Vec<u8>, kind: u16, id: u16, data: &[u8]) {
    output.extend((data.len() as u32).to_le_bytes());
    output.extend(32_u32.to_le_bytes());
    output.extend(u16::MAX.to_le_bytes());
    output.extend(kind.to_le_bytes());
    output.extend(u16::MAX.to_le_bytes());
    output.extend(id.to_le_bytes());
    output.extend([0; 16]);
    output.extend_from_slice(data);
    output.resize((output.len() + 3) & !3, 0);
}
