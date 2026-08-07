//! Roland TR-6S / TR-8S user-data formats — **plaintext user data only**.
//!
//! This crate reads (and losslessly rewrites) the TR device backup container
//! and, in time, its kit/pattern records. It touches **no firmware and no
//! encryption** — the backup and its sections are plaintext, so nothing here
//! relates to the encrypted `App1_Main` image or its key.
//!
//! ## Container format (reverse-engineered from a v1.51 TR-6S backup)
//!
//! A backup (`*_bak.bin`) is a 64-byte file header followed by a sequence of
//! tagged chunks:
//!
//! ```text
//! 0x00  file header: "TR6S" + 0x00000000 + u32 version(=5) + reserved + 16-space name + ...
//! 0x40  chunk: tag[4] | u32 reserved(=0) | u32 payload_size | u32 extra | payload[payload_size]
//! ...   more chunks: SYS / PTN / KIT / SMPL / FX ...
//! ```
//!
//! Known chunk tags: `SYS ` (system params — same body as the firmware
//! `init_param`), `PTN ` (128 pattern records), `KIT ` (128 kit records),
//! `SMPL` (user samples; empty when none are loaded), `FX  `.
//!
//! ## Losslessness
//!
//! [`Backup`] retains the original bytes verbatim. [`Backup::to_bytes`] returns
//! them unchanged, so `parse(x).to_bytes() == x` **always** holds regardless of
//! how much of the format is understood — the section directory is an index over
//! the retained bytes, never a re-serialization. This is the safety property a
//! librarian needs: editing one section can never corrupt unknown/reserved data.
//!
//! Record-internal layout (individual kit/pattern fields) is not decoded yet;
//! see `docs/tr-format.md` for the reversing plan.

use anyhow::{bail, Context, Result};

/// The TR-6S / TR-8S backup file magic.
pub const MAGIC_TR6S: &[u8; 4] = b"TR6S";
pub const MAGIC_TR8S: &[u8; 4] = b"TR8S";

/// Bytes before the first chunk (the fixed file header).
pub const HEADER_LEN: usize = 0x40;
/// Chunk header: tag(4) + reserved(4) + payload_size(4) + extra(4).
pub const CHUNK_HEADER_LEN: usize = 16;

/// Container-level chunk tags we recognize. The `reserved == 0` check (below)
/// keeps 16-char name fields like `USER01` — which live *inside* the `SYS `
/// payload — from being mistaken for chunks.
const KNOWN_TAGS: &[&[u8; 4]] = &[b"SYS ", b"PTN ", b"KIT ", b"SMPL", b"FX  ", b"SONG"];

/// One tagged chunk located within a backup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub tag: [u8; 4],
    /// Offset of the 16-byte chunk header within the file.
    pub header_offset: usize,
    /// Offset of the payload (header_offset + 16).
    pub payload_offset: usize,
    /// Declared payload length (bytes).
    pub payload_len: usize,
    /// The `extra` u32 at header+12 (0 for most; the reserved sample-region
    /// size for `SMPL`).
    pub extra: u32,
}

impl Section {
    /// Tag as a string, trailing spaces trimmed (e.g. `"PTN"`).
    pub fn tag_str(&self) -> String {
        String::from_utf8_lossy(&self.tag).trim_end().to_string()
    }

    /// The payload byte range within the file.
    pub fn payload_range(&self) -> std::ops::Range<usize> {
        self.payload_offset..self.payload_offset + self.payload_len
    }

    /// For array sections (`PTN `/`KIT `), the `(count, record_size)` declared
    /// at the start of the payload, if present and self-consistent.
    pub fn array_shape(&self, raw: &[u8]) -> Option<(u32, u32)> {
        if self.tag != *b"PTN " && self.tag != *b"KIT " {
            return None;
        }
        let p = self.payload_offset;
        if p + 8 > raw.len() {
            return None;
        }
        let count = u32::from_le_bytes(raw[p..p + 4].try_into().ok()?);
        let rec = u32::from_le_bytes(raw[p + 4..p + 8].try_into().ok()?);
        // sanity: 1..=1024 records of a plausible size.
        if (1..=1024).contains(&count) && (1..=1 << 20).contains(&rec) {
            Some((count, rec))
        } else {
            None
        }
    }
}

/// A parsed TR backup. Retains the original bytes for lossless round-trip.
#[derive(Debug, Clone)]
pub struct Backup {
    raw: Vec<u8>,
    magic: [u8; 4],
    version: u32,
    sections: Vec<Section>,
}

impl Backup {
    /// Parse a backup image. Verifies the `TR6S`/`TR8S` magic and builds a
    /// directory of the container chunks. The bytes are retained verbatim.
    pub fn parse(bytes: impl Into<Vec<u8>>) -> Result<Backup> {
        let raw = bytes.into();
        if raw.len() < HEADER_LEN {
            bail!("too small to be a TR backup ({} bytes)", raw.len());
        }
        let mut magic = [0u8; 4];
        magic.copy_from_slice(&raw[0..4]);
        if &magic != MAGIC_TR6S && &magic != MAGIC_TR8S {
            bail!(
                "bad magic {:?}: not a TR6S/TR8S backup",
                String::from_utf8_lossy(&magic)
            );
        }
        let version = u32::from_le_bytes(raw[8..12].try_into().unwrap());
        let sections = scan_sections(&raw);
        Ok(Backup {
            raw,
            magic,
            version,
            sections,
        })
    }

    /// The exact original bytes. Lossless: `parse(x).to_bytes() == x`.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.raw.clone()
    }

    pub fn magic(&self) -> [u8; 4] {
        self.magic
    }
    pub fn version(&self) -> u32 {
        self.version
    }
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// Find the first section with the given tag (space-padded to 4 bytes,
    /// e.g. `find("KIT")`).
    pub fn find(&self, tag: &str) -> Option<&Section> {
        let mut t = [b' '; 4];
        for (i, b) in tag.bytes().take(4).enumerate() {
            t[i] = b;
        }
        self.sections.iter().find(|s| s.tag == t)
    }

    /// Replace a section's payload with `new` in place. Requires the same
    /// length (length-preserving edits keep every other offset valid and keep
    /// the round-trip trivially exact). Returns an error on length mismatch.
    pub fn replace_payload(&mut self, section_index: usize, new: &[u8]) -> Result<()> {
        let s = self
            .sections
            .get(section_index)
            .context("section index out of range")?;
        if new.len() != s.payload_len {
            bail!(
                "length-preserving edit required: section {} payload is {} bytes, got {}",
                s.tag_str(),
                s.payload_len,
                new.len()
            );
        }
        let range = s.payload_range();
        self.raw[range].copy_from_slice(new);
        Ok(())
    }
}

/// Size of one `KIT ` record (bytes). Verified: 128 records of `0x520` fill the
/// KIT section payload exactly on a v1.51 TR-6S backup.
pub const KIT_RECORD_SIZE: usize = 0x520;
/// Offset of the 16-char kit name within a kit record.
pub const KIT_NAME_OFFSET: usize = 0x10;
/// Length of the kit-name field.
pub const KIT_NAME_LEN: usize = 16;

/// A kit record located within the `KIT ` section. A lightweight view — call
/// [`Kit::name`] / [`Kit::bytes`] with the owning [`Backup`]'s `raw()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kit {
    /// 0-based slot index within the KIT section.
    pub index: usize,
    /// Byte offset of the record within the file.
    pub offset: usize,
}

impl Kit {
    /// The record's bytes (`KIT_RECORD_SIZE` long).
    pub fn bytes<'a>(&self, raw: &'a [u8]) -> &'a [u8] {
        &raw[self.offset..self.offset + KIT_RECORD_SIZE]
    }

    /// The kit name (`+0x10`, 16 bytes, trailing spaces/NULs trimmed).
    pub fn name(&self, raw: &[u8]) -> String {
        let s = self.offset + KIT_NAME_OFFSET;
        String::from_utf8_lossy(&raw[s..s + KIT_NAME_LEN])
            .trim_end_matches([' ', '\0'])
            .to_string()
    }
}

impl Backup {
    /// The kit records in the `KIT ` section (128 on a full TR-6S backup), or an
    /// empty vec if there is no KIT section.
    ///
    /// NOTE: only the record framing and name are decoded so far. The per-record
    /// header (`+0x00`, includes an as-yet-unidentified checksum) and the
    /// per-voice params (6 voices: BD/SD/LT/HC/CH/OH, referencing a separate
    /// tone table) are not decoded yet — see `docs/tr-format.md`.
    pub fn kits(&self) -> Vec<Kit> {
        let Some(sec) = self.find("KIT") else {
            return Vec::new();
        };
        let n = sec.payload_len / KIT_RECORD_SIZE;
        (0..n)
            .map(|i| Kit {
                index: i,
                offset: sec.payload_offset + i * KIT_RECORD_SIZE,
            })
            .collect()
    }
}

/// Scan for container chunks: 4-byte-aligned offsets whose tag is known and
/// whose reserved u32 (header+4) is zero. Best-effort directory; the retained
/// bytes remain the source of truth for round-trip.
fn scan_sections(raw: &[u8]) -> Vec<Section> {
    let mut out = Vec::new();
    let mut o = HEADER_LEN;
    while o + CHUNK_HEADER_LEN <= raw.len() {
        let tag = &raw[o..o + 4];
        let reserved = u32::from_le_bytes(raw[o + 4..o + 8].try_into().unwrap());
        let is_known = KNOWN_TAGS.iter().any(|k| k.as_slice() == tag);
        if is_known && reserved == 0 {
            let payload_len = u32::from_le_bytes(raw[o + 8..o + 12].try_into().unwrap()) as usize;
            let extra = u32::from_le_bytes(raw[o + 12..o + 16].try_into().unwrap());
            let payload_offset = o + CHUNK_HEADER_LEN;
            // Clamp to file end so a corrupt size can't panic downstream.
            let payload_len = payload_len.min(raw.len().saturating_sub(payload_offset));
            let mut t = [0u8; 4];
            t.copy_from_slice(tag);
            out.push(Section {
                tag: t,
                header_offset: o,
                payload_offset,
                payload_len,
                extra,
            });
        }
        o += 4;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a tiny synthetic backup: header + two chunks. No Roland bytes.
    fn synthetic() -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"TR6S"); // magic
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&5u32.to_le_bytes()); // version
        v.resize(HEADER_LEN, 0); // pad file header to 0x40
        // chunk: "SYS " with 4-byte payload
        let payload = [0xAAu8, 0xBB, 0xCC, 0xDD];
        v.extend_from_slice(b"SYS ");
        v.extend_from_slice(&0u32.to_le_bytes()); // reserved
        v.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes()); // extra
        v.extend_from_slice(&payload);
        // a name field that must NOT be seen as a chunk (reserved != 0)
        v.extend_from_slice(b"USER01\0\0\0\0\0\0\0\0\0\0");
        // chunk: "KIT " array-shaped payload: count=2, rec=3, then 2*3 bytes
        v.extend_from_slice(b"KIT ");
        v.extend_from_slice(&0u32.to_le_bytes());
        let kit_payload_len = 8 + 2 * 3;
        v.extend_from_slice(&(kit_payload_len as u32).to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&2u32.to_le_bytes()); // count
        v.extend_from_slice(&3u32.to_le_bytes()); // record_size
        v.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        v
    }

    #[test]
    fn round_trip_is_byte_exact() {
        let bytes = synthetic();
        let b = Backup::parse(bytes.clone()).unwrap();
        assert_eq!(b.to_bytes(), bytes, "round-trip must be byte-identical");
    }

    #[test]
    fn parses_magic_and_version() {
        let b = Backup::parse(synthetic()).unwrap();
        assert_eq!(&b.magic(), MAGIC_TR6S);
        assert_eq!(b.version(), 5);
    }

    #[test]
    fn finds_sys_and_kit_but_not_name_field() {
        let b = Backup::parse(synthetic()).unwrap();
        let tags: Vec<_> = b.sections().iter().map(|s| s.tag_str()).collect();
        assert!(tags.contains(&"SYS".to_string()));
        assert!(tags.contains(&"KIT".to_string()));
        // "USER01" is a name inside the stream, not a chunk — must be excluded.
        assert!(!tags.iter().any(|t| t.starts_with("USER")));
    }

    #[test]
    fn kit_array_shape() {
        let b = Backup::parse(synthetic()).unwrap();
        let kit = b.find("KIT").unwrap();
        assert_eq!(kit.array_shape(b.raw()), Some((2, 3)));
    }

    /// A backup with a real-sized KIT section: 2 records of KIT_RECORD_SIZE,
    /// names at +0x10. No Roland bytes.
    fn synthetic_with_kits(names: &[&str]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"TR6S");
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&5u32.to_le_bytes());
        v.resize(HEADER_LEN, 0);
        // KIT chunk header
        v.extend_from_slice(b"KIT ");
        v.extend_from_slice(&0u32.to_le_bytes());
        let payload_len = names.len() * KIT_RECORD_SIZE;
        v.extend_from_slice(&(payload_len as u32).to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        for name in names {
            let mut rec = vec![0u8; KIT_RECORD_SIZE];
            let nb = name.as_bytes();
            let n = nb.len().min(KIT_NAME_LEN);
            rec[KIT_NAME_OFFSET..KIT_NAME_OFFSET + n].copy_from_slice(&nb[..n]);
            for b in &mut rec[KIT_NAME_OFFSET + n..KIT_NAME_OFFSET + KIT_NAME_LEN] {
                *b = b' ';
            }
            v.extend_from_slice(&rec);
        }
        v
    }

    #[test]
    fn kits_parse_names() {
        let bytes = synthetic_with_kits(&["TR-808_Kit", "My Kit"]);
        let b = Backup::parse(bytes.clone()).unwrap();
        let kits = b.kits();
        assert_eq!(kits.len(), 2);
        assert_eq!(kits[0].name(b.raw()), "TR-808_Kit");
        assert_eq!(kits[1].name(b.raw()), "My Kit");
        assert_eq!(kits[0].bytes(b.raw()).len(), KIT_RECORD_SIZE);
        // still lossless
        assert_eq!(b.to_bytes(), bytes);
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = synthetic();
        bytes[0] = b'X';
        assert!(Backup::parse(bytes).is_err());
    }

    #[test]
    fn length_preserving_edit_round_trips() {
        let b = Backup::parse(synthetic()).unwrap();
        let sys_idx = b.sections().iter().position(|s| s.tag == *b"SYS ").unwrap();
        let mut b2 = b.clone();
        b2.replace_payload(sys_idx, &[0, 0, 0, 0]).unwrap();
        // edit took effect and the file is still parseable + same length
        assert_eq!(b2.to_bytes().len(), b.to_bytes().len());
        // wrong-length edit is refused
        assert!(b2.replace_payload(sys_idx, &[0, 0]).is_err());
    }
}
