//! Container helpers for the Roland `App1_Main` update image.
//!
//! An `App1_Main` payload is a 96-byte plaintext header, then the encrypted
//! body, then a 64-byte plaintext trailer. The body is what the ECB/relocation
//! analyses want; passing `--app1` extracts its `(offset, len)` so callers need
//! not hand-compute `--offset 96 --len <payloadlen>` per file. See
//! `docs/firmware-format.md`.

/// Byte offset of the encrypted payload within an `App1_Main` image.
pub const APP1_PAYLOAD_OFFSET: usize = 0x60;
/// Offset of the little-endian `u32` payload length in the header.
pub const APP1_PAYLOAD_LEN_FIELD: usize = 0x44;

/// If `data` is an `App1_Main` image, the `(offset, len)` of its encrypted
/// payload, clamped to the file. `None` if the magic or declared length does
/// not check out (so callers can fall back to whole-file analysis).
pub fn app1_payload_range(data: &[u8]) -> Option<(usize, usize)> {
    if data.len() < APP1_PAYLOAD_OFFSET || &data[0..9] != b"App1_Main" {
        return None;
    }
    let f = APP1_PAYLOAD_LEN_FIELD;
    let len = u32::from_le_bytes(data[f..f + 4].try_into().ok()?) as usize;
    // The declared payload must fit after the header.
    if len == 0 || APP1_PAYLOAD_OFFSET + len > data.len() {
        return None;
    }
    Some((APP1_PAYLOAD_OFFSET, len))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(magic: &[u8], payload_len: u32, total: usize) -> Vec<u8> {
        let mut v = vec![0u8; total];
        v[..magic.len()].copy_from_slice(magic);
        v[APP1_PAYLOAD_LEN_FIELD..APP1_PAYLOAD_LEN_FIELD + 4]
            .copy_from_slice(&payload_len.to_le_bytes());
        v
    }

    #[test]
    fn extracts_the_payload_range() {
        // header(0x60) + payload(0x100) + trailer(0x40)
        let v = img(b"App1_Main", 0x100, 0x60 + 0x100 + 0x40);
        assert_eq!(app1_payload_range(&v), Some((0x60, 0x100)));
    }

    #[test]
    fn rejects_non_app1_and_bogus_lengths() {
        assert_eq!(app1_payload_range(b"not an image at all______"), None);
        // right magic, but the declared length overruns the file.
        let v = img(b"App1_Main", 0xFFFF, 0x60 + 0x10);
        assert_eq!(app1_payload_range(&v), None);
        // right magic, zero length.
        let v = img(b"App1_Main", 0, 0x200);
        assert_eq!(app1_payload_range(&v), None);
    }
}
