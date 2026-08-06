//! Canonical hex + ASCII dump, `hexdump -C` style.

use anyhow::{Context, Result};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub fn run(image: &Path, offset: u64, len: Option<u64>) -> Result<()> {
    let mut f = File::open(image).with_context(|| format!("opening {}", image.display()))?;
    let total = f.metadata()?.len();

    anyhow::ensure!(
        offset <= total,
        "offset 0x{offset:x} is past end of file (0x{total:x})"
    );
    let to_read = match len {
        Some(n) => n.min(total - offset),
        None => total - offset,
    };

    f.seek(SeekFrom::Start(offset))?;
    let mut remaining = to_read;
    let mut buf = [0u8; 16];
    let mut addr = offset;

    while remaining > 0 {
        let want = (remaining.min(16)) as usize;
        let got = read_up_to(&mut f, &mut buf[..want])?;
        if got == 0 {
            break;
        }
        print_line(addr, &buf[..got]);
        addr += got as u64;
        remaining -= got as u64;
    }
    Ok(())
}

fn read_up_to<R: Read>(r: &mut R, buf: &mut [u8]) -> Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

fn print_line(addr: u64, bytes: &[u8]) {
    print!("{addr:08x}  ");
    for i in 0..16 {
        if i == 8 {
            print!(" ");
        }
        if i < bytes.len() {
            print!("{:02x} ", bytes[i]);
        } else {
            print!("   ");
        }
    }
    print!(" |");
    for &b in bytes {
        let c = if (0x20..=0x7e).contains(&b) {
            b as char
        } else {
            '.'
        };
        print!("{c}");
    }
    println!("|");
}
