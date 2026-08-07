//! Checksum / CRC computation and brute-forcing.
//!
//! When you find a 2- or 4-byte field that looks like it validates an image,
//! you rarely know which algorithm produced it. This module computes a spread
//! of common checksums over a region and, given an expected value, tells you
//! which (if any) matches — the fast way to identify an unknown integrity field.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// A named checksum over a byte slice, producing a u32 (narrower results are
/// zero-extended so everything shares one type).
struct Algo {
    name: &'static str,
    compute: fn(&[u8]) -> u32,
}

const ALGOS: &[Algo] = &[
    Algo {
        name: "sum8",
        compute: sum8,
    },
    Algo {
        name: "sum16-le",
        compute: sum16,
    },
    Algo {
        name: "sum32-le",
        compute: sum32,
    },
    Algo {
        name: "xor8",
        compute: xor8,
    },
    Algo {
        name: "crc16-ccitt (0x1021, init 0xFFFF)",
        compute: crc16_ccitt,
    },
    Algo {
        name: "crc16-modbus (0xA001, init 0xFFFF)",
        compute: crc16_modbus,
    },
    Algo {
        name: "crc32 (0xEDB88320, IEEE)",
        compute: crc32_ieee,
    },
];

pub fn run(image: &Path, offset: u64, len: Option<u64>, expect: Option<u32>) -> Result<()> {
    let data = fs::read(image).with_context(|| format!("reading {}", image.display()))?;
    let start = offset as usize;
    anyhow::ensure!(start <= data.len(), "offset past end of file");
    let end = match len {
        Some(n) => (start + n as usize).min(data.len()),
        None => data.len(),
    };
    let region = &data[start..end];

    println!(
        "# checksum over {} bytes [0x{:x}..0x{:x}) of {}",
        region.len(),
        start,
        end,
        image.display()
    );

    for algo in ALGOS {
        let v = (algo.compute)(region);
        let flag = match expect {
            Some(e) if e == v => "  <== MATCH",
            _ => "",
        };
        println!("{:<34} 0x{:08x}{}", algo.name, v, flag);
    }

    if let Some(e) = expect {
        let any = ALGOS.iter().any(|a| (a.compute)(region) == e);
        if !any {
            println!(
                "# no listed algorithm reproduces 0x{e:08x} over this region — \
                 try a different offset/length or endianness."
            );
        }
    }
    Ok(())
}

fn sum8(d: &[u8]) -> u32 {
    d.iter().fold(0u8, |a, &b| a.wrapping_add(b)) as u32
}

fn sum16(d: &[u8]) -> u32 {
    (d.iter().fold(0u16, |a, &b| a.wrapping_add(b as u16))) as u32
}

fn sum32(d: &[u8]) -> u32 {
    d.iter().fold(0u32, |a, &b| a.wrapping_add(b as u32))
}

fn xor8(d: &[u8]) -> u32 {
    d.iter().fold(0u8, |a, &b| a ^ b) as u32
}

fn crc16_ccitt(d: &[u8]) -> u32 {
    let mut crc: u16 = 0xFFFF;
    for &b in d {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc as u32
}

fn crc16_modbus(d: &[u8]) -> u32 {
    let mut crc: u16 = 0xFFFF;
    for &b in d {
        crc ^= b as u16;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xA001;
            } else {
                crc >>= 1;
            }
        }
    }
    crc as u32
}

fn crc32_ieee(d: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in d {
        crc ^= b as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    // "123456789" is the canonical CRC check vector.
    const CHECK: &[u8] = b"123456789";

    #[test]
    fn crc32_check_vector() {
        assert_eq!(crc32_ieee(CHECK), 0xCBF4_3926);
    }

    #[test]
    fn crc16_ccitt_check_vector() {
        assert_eq!(crc16_ccitt(CHECK), 0x29B1);
    }

    #[test]
    fn crc16_modbus_check_vector() {
        assert_eq!(crc16_modbus(CHECK), 0x4B37);
    }

    #[test]
    fn trivial_sums() {
        assert_eq!(sum8(&[1, 2, 3]), 6);
        assert_eq!(xor8(&[0xff, 0x0f]), 0xf0);
    }
}
