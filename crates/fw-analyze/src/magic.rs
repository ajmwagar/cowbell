//! Header / magic-byte inspection.
//!
//! A small hand-maintained table of signatures we actually expect to run into
//! while poking at Roland installer payloads and the raw NOR flash image. This
//! is not a substitute for binwalk's database — it is the short list of things
//! worth eyeballing at offset 0, plus an optional full-file scan.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

struct Signature {
    magic: &'static [u8],
    name: &'static str,
}

/// Signatures relevant to firmware/installer archaeology. Kept short on purpose.
const SIGNATURES: &[Signature] = &[
    Signature {
        magic: b"\x7fELF",
        name: "ELF executable",
    },
    Signature {
        magic: b"MZ",
        name: "DOS/PE executable (Windows .exe/InstallShield)",
    },
    Signature {
        magic: b"PK\x03\x04",
        name: "ZIP archive (also .apk/.jar/embedded)",
    },
    Signature {
        magic: b"\x1f\x8b",
        name: "gzip stream",
    },
    Signature {
        magic: b"BZh",
        name: "bzip2 stream",
    },
    Signature {
        magic: b"\xfd7zXZ\x00",
        name: "xz stream",
    },
    Signature {
        magic: b"7z\xbc\xaf\x27\x1c",
        name: "7-Zip archive",
    },
    Signature {
        magic: b"Rar!\x1a\x07",
        name: "RAR archive",
    },
    Signature {
        magic: b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1",
        name: "MS CFB (MSI installer / OLE)",
    },
    Signature {
        magic: b"xar!",
        name: "XAR archive (macOS .pkg)",
    },
    Signature {
        magic: b"ustar",
        name: "tar archive (ustar, at offset 257)",
    },
    Signature {
        magic: b"hsqs",
        name: "SquashFS (little-endian)",
    },
    Signature {
        magic: b"sqsh",
        name: "SquashFS (big-endian)",
    },
    Signature {
        magic: b"UBI#",
        name: "UBI volume",
    },
    Signature {
        magic: b"\x27\x05\x19\x56",
        name: "U-Boot uImage",
    },
    Signature {
        magic: b"MThd",
        name: "Standard MIDI file",
    },
    Signature {
        magic: b"RIFF",
        name: "RIFF container (WAV/AVI)",
    },
];

pub fn run(image: &Path, scan: bool) -> Result<()> {
    let data = fs::read(image).with_context(|| format!("reading {}", image.display()))?;

    println!("# inspect {} ({} bytes)", image.display(), data.len());

    // Always dump the first 16 bytes as a header preview.
    let head = &data[..data.len().min(16)];
    print!("head:");
    for b in head {
        print!(" {b:02x}");
    }
    println!();

    // Match at offset 0.
    let mut matched_at_zero = false;
    for sig in SIGNATURES {
        if data.starts_with(sig.magic) {
            println!("offset 0x00000000: {}", sig.name);
            matched_at_zero = true;
        }
    }
    if !matched_at_zero {
        println!("offset 0x00000000: no known magic (raw/opaque payload?)");
    }

    if scan {
        println!("# full-file magic scan:");
        let mut hits = 0usize;
        for sig in SIGNATURES {
            for off in find_all(&data, sig.magic) {
                // Skip the offset-0 hit we already reported.
                if off == 0 {
                    continue;
                }
                println!("offset 0x{off:08x}: {}", sig.name);
                hits += 1;
            }
        }
        if hits == 0 {
            println!("(no embedded magics found)");
        }
    }
    Ok(())
}

/// All start offsets where `needle` occurs in `hay`.
fn find_all(hay: &[u8], needle: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    if needle.is_empty() || needle.len() > hay.len() {
        return out;
    }
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        if &hay[i..i + needle.len()] == needle {
            out.push(i);
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}
