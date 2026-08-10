//! LZSS decompression for Roland's `init_param` firmware component.
//!
//! `dd001_init_param.bin` (and its TR-8S sibling) is **not** encrypted — it is
//! LZSS-*compressed*. It carries three byte-identical compressed sections, each
//! of which decompresses to a **full factory-default backup image** (`TR6S`
//! magic, the same `SYS`/`PTN`/`KIT`/`TONE` chunk layout as an SD-card backup).
//! That makes it a free second corpus of factory defaults for `tr-format`.
//!
//! The variant is classic Okumura `lzss.c`, with the parameters Roland uses:
//!
//! - 4096-byte ring buffer, **pre-filled with `0x00`** (not the textbook `0x20`);
//! - initial write position `N − F = 0xFEE`;
//! - one control byte per 8 tokens, **LSB first**, bit `1` = literal, `0` = match;
//! - a match token is 2 bytes: `offset = b0 | ((b1 & 0xF0) << 4)` (12-bit),
//!   `length = (b1 & 0x0F) + THRESHOLD + 1`.
//!
//! This is firmware tooling, so it lives here rather than in `tr-format` (which
//! stays firmware-free). See `docs/firmware-format.md`.

use anyhow::{bail, ensure, Result};

const RING: usize = 4096;
/// Longest encodable match (`F`).
const MAX_MATCH: usize = 18;
/// Matches shorter than `THRESHOLD + 1` are stored as literals.
const THRESHOLD: usize = 2;
/// Initial ring write position (`RING − MAX_MATCH`).
const INIT_POS: usize = RING - MAX_MATCH;

/// Decompress a complete Okumura-LZSS stream (Roland parameters). Decodes the
/// whole of `data` until it is exhausted.
pub fn decompress(data: &[u8]) -> Vec<u8> {
    let mut ring = [0u8; RING];
    let mut r = INIT_POS;
    let mut out = Vec::new();
    let mut i = 0usize;
    // `flags` holds the current control byte in its low 8 bits; bit 8 is a
    // sentinel that falls off after 8 shifts, signalling "reload".
    let mut flags: u32 = 0;
    while i < data.len() {
        flags >>= 1;
        if flags & 0x100 == 0 {
            if i >= data.len() {
                break;
            }
            flags = data[i] as u32 | 0xFF00;
            i += 1;
        }
        if flags & 1 != 0 {
            // literal
            if i >= data.len() {
                break;
            }
            let c = data[i];
            i += 1;
            out.push(c);
            ring[r] = c;
            r = (r + 1) % RING;
        } else {
            // back-reference
            if i + 1 >= data.len() {
                break;
            }
            let b0 = data[i] as usize;
            let b1 = data[i + 1] as usize;
            i += 2;
            let pos = b0 | ((b1 & 0xF0) << 4);
            let length = (b1 & 0x0F) + THRESHOLD + 1;
            for k in 0..length {
                let c = ring[(pos + k) % RING];
                out.push(c);
                ring[r] = c;
                r = (r + 1) % RING;
            }
        }
    }
    out
}

/// Offset of the first (of three redundant) compressed sections in an
/// `init_param` file.
pub const INIT_PARAM_STREAM_OFFSET: usize = 0x30;

/// Parse an `INIT`-magic `init_param` file and decompress its first section to
/// the factory-default backup image. Verifies the magic and that the decoded
/// length matches the header's declared size.
pub fn decompress_init_param(raw: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        raw.len() >= INIT_PARAM_STREAM_OFFSET && &raw[0..4] == b"INIT",
        "not an init_param file (missing INIT magic)"
    );
    let comp_len = u32::from_le_bytes(raw[0x08..0x0C].try_into().unwrap()) as usize;
    let decomp_len = u32::from_le_bytes(raw[0x14..0x18].try_into().unwrap()) as usize;
    let end = INIT_PARAM_STREAM_OFFSET + comp_len;
    ensure!(
        comp_len > 0 && end <= raw.len(),
        "declared compressed size {comp_len:#x} overruns the file"
    );
    let out = decompress(&raw[INIT_PARAM_STREAM_OFFSET..end]);
    if out.len() != decomp_len {
        bail!(
            "decompressed {} bytes, header declared {decomp_len} — wrong LZSS variant or offset",
            out.len()
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_literals_then_a_back_reference() {
        // flag 0x07 = literal, literal, literal, MATCH (LSB-first).
        // 'A','B','C' land at ring 0xFEE..0xFF1; the match (pos 0xFEE, len 3)
        // copies them back -> "ABCABC".
        let stream = [0x07, b'A', b'B', b'C', 0xEE, 0xF0];
        assert_eq!(decompress(&stream), b"ABCABC");
    }

    #[test]
    fn all_literal_stream_round_trips() {
        // A control byte of 0xFF marks 8 following bytes as literals.
        let mut stream = vec![0xFF];
        stream.extend_from_slice(b"ROLANDXX");
        assert_eq!(decompress(&stream), b"ROLANDXX");
    }

    #[test]
    fn init_param_header_is_parsed_and_decompressed() {
        // Synthetic INIT file: 5 literals "HELLO" at the 0x30 stream offset.
        let payload = {
            let mut s = vec![0x1Fu8]; // flag: 5 literal bits set
            s.extend_from_slice(b"HELLO");
            s
        };
        let mut raw = vec![0u8; INIT_PARAM_STREAM_OFFSET];
        raw[0..4].copy_from_slice(b"INIT");
        raw[0x08..0x0C].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        raw[0x14..0x18].copy_from_slice(&5u32.to_le_bytes());
        raw.extend_from_slice(&payload);

        assert_eq!(decompress_init_param(&raw).unwrap(), b"HELLO");

        // wrong magic and a size mismatch are rejected.
        let mut bad = raw.clone();
        bad[0] = b'X';
        assert!(decompress_init_param(&bad).is_err());
        let mut mismatch = raw.clone();
        mismatch[0x14..0x18].copy_from_slice(&99u32.to_le_bytes());
        assert!(decompress_init_param(&mismatch).is_err());
    }
}
