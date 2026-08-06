//! Installer container sniffing.

use anyhow::{Context, Result};
use std::fs::File;
use std::io::Read;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    /// DOS/PE — self-extracting .exe (InstallShield / NSIS / 7z SFX).
    PeExe,
    /// OLE/CFB compound file — .msi installer.
    Msi,
    /// Apple Disk Image — .dmg.
    Dmg,
    /// XAR archive — macOS flat .pkg.
    Pkg,
    /// Plain zip.
    Zip,
    /// 7-Zip archive.
    SevenZip,
    /// Nothing recognized.
    Unknown,
}

impl Container {
    /// The external tool we'd reach for to unpack this, if any.
    pub fn suggested_tool(self) -> Option<&'static str> {
        match self {
            Container::PeExe | Container::Zip | Container::SevenZip | Container::Dmg => Some("7z"),
            Container::Msi => Some("msiextract"),
            Container::Pkg => Some("xar"),
            Container::Unknown => None,
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Container::PeExe => "Windows PE executable (self-extracting installer)",
            Container::Msi => "MSI installer (OLE/CFB compound file)",
            Container::Dmg => "Apple Disk Image (.dmg)",
            Container::Pkg => "macOS flat package (.pkg, XAR archive)",
            Container::Zip => "ZIP archive",
            Container::SevenZip => "7-Zip archive",
            Container::Unknown => "unrecognized container",
        }
    }
}

/// Sniff a container type from the first bytes of a file (plus the trailing
/// bytes for `.dmg`, whose `koly` trailer lives at the end).
pub fn sniff(path: &Path) -> Result<Container> {
    let mut f = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut head = [0u8; 16];
    let n = read_up_to(&mut f, &mut head)?;
    let head = &head[..n];

    if head.starts_with(b"MZ") {
        return Ok(Container::PeExe);
    }
    if head.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) {
        return Ok(Container::Msi);
    }
    if head.starts_with(b"xar!") {
        return Ok(Container::Pkg);
    }
    if head.starts_with(b"7z\xbc\xaf\x27\x1c") {
        return Ok(Container::SevenZip);
    }
    if head.starts_with(b"PK\x03\x04") {
        return Ok(Container::Zip);
    }

    // .dmg (UDIF) carries a 512-byte "koly" trailer at end-of-file.
    let len = f.metadata()?.len();
    if len >= 512 {
        use std::io::{Seek, SeekFrom};
        f.seek(SeekFrom::Start(len - 512))?;
        let mut koly = [0u8; 4];
        if read_up_to(&mut f, &mut koly)? == 4 && &koly == b"koly" {
            return Ok(Container::Dmg);
        }
    }

    Ok(Container::Unknown)
}

pub fn run_identify(package: &Path) -> Result<()> {
    let c = sniff(package)?;
    println!("file:      {}", package.display());
    println!("container: {}", c.describe());
    match c.suggested_tool() {
        Some(tool) => println!("unpack:    `fw-extract unpack` will shell out to `{tool}`"),
        None => println!(
            "unpack:    unknown container — try `fw-analyze inspect --scan` or \
             `binwalk` to look for embedded archives"
        ),
    }
    Ok(())
}

fn read_up_to<R: Read>(r: &mut R, buf: &mut [u8]) -> Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..])? {
            0 => break,
            k => filled += k,
        }
    }
    Ok(filled)
}
