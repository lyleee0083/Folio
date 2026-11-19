use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::Command;

fn main() -> io::Result<()> {
    println!("cargo:rerun-if-changed=assets/app.ico");
    println!("cargo:rerun-if-changed=assets/folio.manifest");
    println!("cargo:rerun-if-env-changed=FOLIO_WINDRES");
    let target = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
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
    fs::write(&resource, records)?;
    let linked = if target == "gnu" {
        let tool = env::var_os("FOLIO_WINDRES").unwrap_or_else(|| "windres.exe".into());
        let file = out.join("folio-resources.o");
        let status = Command::new(tool)
            .args(["-J", "res", "-O", "coff", "-F", "pe-x86-64", "-i"])
            .arg(&resource)
            .arg("-o")
            .arg(&file)
            .status()?;
        if !status.success() {
            return Err(io::Error::other(
                "windres could not compile Folio resources",
            ));
        }
        file
    } else {
        resource
    };
    println!("cargo:rustc-link-arg-bins={}", linked.display());
    Ok(())
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
