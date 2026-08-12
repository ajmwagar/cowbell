//! The device's SysEx **value** wire encoding.
//!
//! A schema type `int{N}x{B}` is `N` digits of `B` bits (the same naming that
//! sizes fields in `tr-format`). On the 7-bit-safe SysEx wire each digit travels
//! as **one byte, most-significant digit first**:
//!
//! - `int1x7` → 1 byte (a plain 7-bit value, `0..=127`)
//! - `int2x4` → 2 bytes (two 4-bit nibbles → `0..=255`)
//! - `int4x4` → 4 bytes (four nibbles → `0..=65535`, used `0..=1023`)
//! - `int8x4` → 8 bytes (eight nibbles → 32-bit)
//!
//! This is **confirmed on real capture data**: in a kit-instrument write,
//! `LEVEL = 255` rides as `0f 0f`, `PAN = 128` as `08 00`, `GAIN ≈ 81` as
//! `05 03`. Note this is a *different* packing from the backup **file**, where
//! the same field is bit-packed into `⌈N·B/8⌉` bytes — the device spreads one
//! digit per byte so every byte stays `≤ 0x7f`.

/// Wire byte-width of a schema value type. Returns `None` for an unrecognized
/// type name.
pub fn wire_len(type_name: &str) -> Option<usize> {
    Some(match type_name {
        "int1x7" => 1,
        "int2x7" => 2,
        "int2x4" => 2,
        "int4x4" => 4,
        "int8x4" => 8,
        _ => return None,
    })
}

/// Decode a nibble-per-byte value (`int{N}x4`) from `bytes` (MSB digit first).
/// Only the low nibble of each byte is significant.
pub fn decode_nibbles(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .fold(0u32, |acc, &b| (acc << 4) | (b & 0x0f) as u32)
}

/// Encode `value` as `n` nibble-per-byte wire bytes (MSB digit first) — the
/// inverse of [`decode_nibbles`]. High nibbles are always clear, so the result
/// is 7-bit-safe.
pub fn encode_nibbles(value: u32, n: usize) -> Vec<u8> {
    (0..n)
        .rev()
        .map(|i| ((value >> (4 * i)) & 0x0f) as u8)
        .collect()
}

/// Decode a 7-bit-digit value (`int{N}x7`) from `bytes` (MSB digit first).
pub fn decode_7bit_digits(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .fold(0u32, |acc, &b| (acc << 7) | (b & 0x7f) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nibble_round_trip_and_real_values() {
        // Real "KiNK 1" kit-instrument-0 wire bytes: level 0f 0f, pan 08 00.
        assert_eq!(decode_nibbles(&[0x0f, 0x0f]), 255);
        assert_eq!(decode_nibbles(&[0x08, 0x00]), 128);
        assert_eq!(decode_nibbles(&[0x05, 0x03]), 83); // gain
        assert_eq!(decode_nibbles(&[0x00, 0x00, 0x0c, 0x01]), 193); // int4x4 tone

        for v in [0u32, 1, 127, 128, 255, 1023, 65535] {
            let n = if v > 255 { 4 } else { 2 };
            let w = encode_nibbles(v, n);
            assert!(w.iter().all(|&b| b <= 0x0f), "7-bit-safe");
            assert_eq!(decode_nibbles(&w), v);
        }
    }

    #[test]
    fn wire_len_matches_the_type_names() {
        assert_eq!(wire_len("int1x7"), Some(1));
        assert_eq!(wire_len("int2x4"), Some(2));
        assert_eq!(wire_len("int4x4"), Some(4));
        assert_eq!(wire_len("int8x4"), Some(8));
        assert_eq!(wire_len("bogus"), None);
    }
}
