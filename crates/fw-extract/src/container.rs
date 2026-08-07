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
    /// GNU/POSIX tar archive. Roland ships TR-6S/TR-8S/T-8 updates as tar
    /// despite the `.bin` extension (members live under a `./_tmp/` prefix).
    Tar,
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
            Container::Tar => Some("tar"),
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
            Container::Tar => "GNU/POSIX tar archive (Roland updater)",
            Container::Unknown => "unrecognized container",
        }
    }
}

/// Sniff a container type from the first bytes of a file (plus the trailing
/// bytes for `.dmg`, whose `koly` trailer lives at the end).
pub fn sniff(path: &Path) -> Result<Container> {
    let mut f = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    // Read a full tar header block: the ustar magic sits at offset 257, so 16
    // bytes is not enough to recognize tar. The leading magic checks below only
    // look at the first few bytes, so a larger buffer is harmless for them.
    let mut head = [0u8; 512];
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

    // tar has no magic at offset 0, so require BOTH the ustar magic at 257 and a
    // valid header checksum to avoid false positives on arbitrary blobs.
    if is_tar_header(head) {
        return Ok(Container::Tar);
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
    match c {
        Container::Tar => {
            println!("unpack:    `fw-extract unpack` (native tar reader — no external tool needed)");
            if let Err(e) = list_tar_members(package) {
                println!("members:   (could not list: {e})");
            }
        }
        _ => match c.suggested_tool() {
            Some(tool) => println!("unpack:    `fw-extract unpack` will shell out to `{tool}`"),
            None => println!(
                "unpack:    unknown container — try `fw-analyze inspect --scan` or \
                 `binwalk` to look for embedded archives"
            ),
        },
    }
    Ok(())
}

/// Print the members of a tar archive with their sizes (identify convenience).
fn list_tar_members(package: &Path) -> Result<()> {
    let f = File::open(package).with_context(|| format!("opening {}", package.display()))?;
    let mut archive = tar::Archive::new(f);
    println!("members:");
    for entry in archive.entries()? {
        let entry = entry?;
        let size = entry.header().size()?;
        let path = entry.path()?;
        println!("  {size:>10}  {}", path.display());
    }
    Ok(())
}

/// Validate a leading 512-byte tar header block.
///
/// tar has no magic at offset 0, so we require two independent signals:
///   1. the POSIX `ustar\0` magic (or the GNU `ustar ` variant) at offset 257;
///   2. a header checksum (octal ASCII at bytes 148..156) matching the recomputed
///      checksum — the sum of all 512 header bytes with the checksum field itself
///      treated as ASCII spaces.
fn is_tar_header(block: &[u8]) -> bool {
    if block.len() < 512 {
        return false;
    }
    let magic = &block[257..263];
    // POSIX: "ustar\0"; GNU: "ustar " (trailing space, version "  ").
    if magic != b"ustar\0" && magic != b"ustar " {
        return false;
    }
    let stored = match parse_octal(&block[148..156]) {
        Some(v) => v,
        None => return false,
    };
    let mut sum: u64 = 0;
    for (i, &b) in block[..512].iter().enumerate() {
        // The checksum field is treated as 8 ASCII spaces when computing.
        sum += if (148..156).contains(&i) { 0x20 } else { b as u64 };
    }
    sum == stored
}

/// Parse a NUL/space-padded octal ASCII field (as used by tar header numbers).
fn parse_octal(field: &[u8]) -> Option<u64> {
    let mut value = 0u64;
    let mut saw_digit = false;
    for &b in field {
        match b {
            b'0'..=b'7' => {
                value = value * 8 + u64::from(b - b'0');
                saw_digit = true;
            }
            // Leading spaces are padding; a trailing NUL/space terminates.
            b' ' if !saw_digit => continue,
            0 | b' ' => break,
            _ => return None,
        }
    }
    saw_digit.then_some(value)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Build a tiny, synthetic tar archive in memory containing one member.
    /// SYNTHETIC ONLY — never real Roland firmware.
    fn synthetic_tar(member: &str, data: &[u8]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, member, data)
            .expect("append synthetic member");
        builder.into_inner().expect("finish synthetic tar")
    }

    fn write_temp(bytes: &[u8]) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().expect("temp file");
        f.write_all(bytes).expect("write temp");
        f.flush().expect("flush temp");
        f
    }

    #[test]
    fn sniff_detects_synthetic_tar() {
        let tar = synthetic_tar("./_tmp/fake_up.bin", b"not real firmware");
        let f = write_temp(&tar);
        assert_eq!(sniff(f.path()).unwrap(), Container::Tar);
    }

    #[test]
    fn sniff_rejects_ustar_magic_with_bad_checksum() {
        // A 1024-byte buffer with the ustar magic planted at 257 but a checksum
        // field that does not match the recomputed value -> must be Unknown.
        let mut buf = vec![0x41u8; 1024]; // fill with 'A' so the byte sum is nonzero
        buf[257..263].copy_from_slice(b"ustar\0");
        // Stored checksum claims octal 0, but the real sum is clearly nonzero.
        buf[148..156].copy_from_slice(b"000000\0 ");
        let f = write_temp(&buf);
        assert_eq!(sniff(f.path()).unwrap(), Container::Unknown);
    }

    #[test]
    fn parse_octal_handles_padding() {
        assert_eq!(parse_octal(b"000644\0 "), Some(0o644));
        assert_eq!(parse_octal(b"   644  "), Some(0o644));
        assert_eq!(parse_octal(b"\0\0\0\0\0\0\0\0"), None);
    }
}
