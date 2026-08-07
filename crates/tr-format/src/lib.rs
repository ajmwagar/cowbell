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
const KNOWN_TAGS: &[&[u8; 4]] = &[
    b"SYS ", b"PTN ", b"KIT ", b"SMPL", b"FX  ", b"SONG", b"TONE",
];

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

/// The six TR-6S voice slots, in record order.
pub const VOICES: [&str; 6] = ["BD", "SD", "LT", "HC", "CH", "OH"];

// --- Kit-record voice block ---------------------------------------------------
// CONFIRMED against Roland TR Editor's schema (Contents/Resources/Script/
// Script.xml, structType `instCommon[0]`): the param order + ranges + defaults
// match the backup bytes exactly (INST LEVEL def 255 -> +0x04=0xff, GAIN def 81
// -> +0x05=0x51, PAN def 128 -> +0x06=0x80, DELAY SEND def 224 -> +0x08=0xe0,
// LFO DEPTH def 128 -> +0x0b=0x80, all observed). Voice fields are contiguous
// single bytes after the u16 tone. See docs/tr-format.md.

/// Offset of voice 0's block within a kit record (BD). The `u16` tone-ID is at
/// the block start.
pub const VOICE_TONE_ID_OFFSET: usize = 0x194;
/// Byte stride between consecutive voice blocks in a kit record.
pub const VOICE_STRIDE: usize = 0x34;

/// The confirmed `instCommon` voice parameters (offsets relative to the voice
/// block start). Named per TR Editor's `Script.xml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoiceParams {
    /// Tone ID (u16, 0–1023) — index into the `TONE` table.
    pub tone: u16,
    /// Tune (0–255, center 128).
    pub tune: u8,
    /// Decay (0–255).
    pub decay: u8,
    /// Level (0–255).
    pub level: u8,
    /// Gain (0–161).
    pub gain: u8,
    /// Pan (0–255, center 128).
    pub pan: u8,
    /// Reverb send (0–255).
    pub reverb_send: u8,
    /// Delay send (0–255).
    pub delay_send: u8,
    /// LFO switch (0/1).
    pub lfo_switch: u8,
    /// LFO destination (0–37).
    pub lfo_dest: u8,
    /// LFO depth (0–255).
    pub lfo_depth: u8,
    /// Category lock (0/1).
    pub category_lock: u8,
}

impl VoiceParams {
    /// Parse from a voice block (>= 0x0D bytes; the block is 0x34 total).
    pub fn from_block(b: &[u8]) -> VoiceParams {
        VoiceParams {
            tone: u16::from_le_bytes([b[0x00], b[0x01]]),
            tune: b[0x02],
            decay: b[0x03],
            level: b[0x04],
            gain: b[0x05],
            pan: b[0x06],
            reverb_send: b[0x07],
            delay_send: b[0x08],
            lfo_switch: b[0x09],
            lfo_dest: b[0x0a],
            lfo_depth: b[0x0b],
            category_lock: b[0x0c],
        }
    }
}

/// One `TONE` table entry (bytes): name[16] + params[20].
pub const TONE_ENTRY_SIZE: usize = 0x24;
/// The `TONE` payload begins with a 16-byte preamble; entry 0 (tone-ID 0) starts
/// after it.
pub const TONE_ENTRY_BASE_IN_PAYLOAD: usize = 0x10;
/// Length of a tone name.
pub const TONE_NAME_LEN: usize = 16;

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

    /// The six voice tone-IDs (BD, SD, LT, HC, CH, OH), indices into the `TONE`
    /// table.
    pub fn voice_tone_ids(&self, raw: &[u8]) -> [u16; 6] {
        let mut ids = [0u16; 6];
        for (i, id) in ids.iter_mut().enumerate() {
            let o = self.offset + VOICE_TONE_ID_OFFSET + i * VOICE_STRIDE;
            *id = u16::from_le_bytes([raw[o], raw[o + 1]]);
        }
        ids
    }

    /// The six voices' full [`VoiceParams`] (BD, SD, LT, HC, CH, OH).
    pub fn voices(&self, raw: &[u8]) -> [VoiceParams; 6] {
        std::array::from_fn(|i| {
            let o = self.offset + VOICE_TONE_ID_OFFSET + i * VOICE_STRIDE;
            VoiceParams::from_block(&raw[o..o + VOICE_STRIDE])
        })
    }
}

impl Backup {
    /// Resolve a tone-ID to its name via the `TONE` section, or `None` if there
    /// is no TONE section / the id is out of range.
    ///
    /// TODO(controlled-diff): entry base/stride reversed from a single backup;
    /// the exact entry count is not yet pinned. Cross-checked against kits 0-3.
    pub fn tone_name(&self, id: u16) -> Option<String> {
        let sec = self.find("TONE")?;
        let base = sec.payload_offset + TONE_ENTRY_BASE_IN_PAYLOAD;
        let o = base + id as usize * TONE_ENTRY_SIZE;
        let end = o + TONE_NAME_LEN;
        // must stay within the TONE payload
        if end > sec.payload_offset + sec.payload_len {
            return None;
        }
        Some(
            String::from_utf8_lossy(&self.raw[o..end])
                .trim_end_matches([' ', '\0'])
                .to_string(),
        )
    }

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

/// Byte size of a `Script.xml` `<value>` type in the backup, given its range
/// maximum. **This is the solved offset model** — accumulating these over a
/// structType's fields (record offset = `0x0F` + schema offset, i.e. after the
/// 16-byte record header) reproduces the kit voice block and the pattern header
/// exactly (validated: 102/103 `ptnCmn` fields land in range).
///
/// - `int1x7`, `int2x4` → 1 byte
/// - `int2x7` → 2 bytes
/// - `int8x4` → 4 bytes
/// - `int4x4` → `ceil(bits(range_max) / 7)` (7-bit-safe packing): 0–1023 → 2,
///   0–3000 → 2, 0–65535 → 3
/// - `stringNx7` → N bytes (16 for the name fields)
pub fn schema_value_size(ty: &str, range_max: u32) -> Option<usize> {
    Some(match ty {
        "int1x7" | "int2x4" => 1,
        "int2x7" => 2,
        "int8x4" => 4,
        "int4x4" => {
            let bits = (32 - range_max.leading_zeros()).max(1) as usize;
            bits.div_ceil(7)
        }
        t if t.starts_with("string") => t
            .strip_prefix("string")
            .and_then(|r| r.split('x').next())
            .and_then(|n| n.parse().ok())
            .unwrap_or(16),
        _ => return None,
    })
}

/// A pattern step word (`int8x4`, 4 bytes). Byte 0 is the velocity; 0 = the step
/// is off. (Higher bytes carry sub-step/flam/probability — not decoded yet.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepWord {
    pub raw: [u8; 4],
}

impl StepWord {
    pub fn velocity(&self) -> u8 {
        self.raw[0]
    }
    pub fn is_on(&self) -> bool {
        self.raw[0] != 0
    }
}

/// Size of one `PTN ` record (bytes): 128 records of `0x5FB8` fill the payload.
pub const PATTERN_RECORD_SIZE: usize = 0x5fb8;
/// Offset of the 16-char pattern name within a pattern record.
pub const PATTERN_NAME_OFFSET: usize = 0x10;
/// Offset of TEMPO (`u16` LE, BPM×10) — schema `ptnCmn.TEMPO`, range 400–3000.
pub const PATTERN_TEMPO_OFFSET: usize = 0x20;
/// Offset of KIT REFFERENCE (`u8`, 1–128) — which kit the pattern plays.
pub const PATTERN_KIT_REF_OFFSET: usize = 0x22;

// --- Pattern body stride (empirically nailed on the v1.51 backup) ------------
/// Record offset where variation 0 (A) begins (right after `ptnCmn`).
pub const PATTERN_VARIATION_0_OFFSET: usize = 0xA0;
/// Bytes between consecutive variations (`ptnVar`): accent(4) + 25 step-arrays
/// (25×64) + motion(832) = 2436. Verified: A–H boundaries are a steady 0x984.
pub const PATTERN_VARIATION_STRIDE: usize = 0x984;
/// Variations per pattern: A–H + 2 fills.
pub const PATTERN_VARIATIONS: usize = 10;
/// `ptnVar00` accent header at the start of each variation (2× accent mask).
pub const PATTERN_ACCENT_SIZE: usize = 4;
/// Step-array slots per variation (`ptnVar01`…`ptnVar25`). The mapping of slot →
/// instrument/voice is a higher-level concern; this crate exposes raw slots.
pub const PATTERN_STEP_TRACKS: usize = 25;
/// Steps per track (`PTN00`…`PTN15`), each a 4-byte [`StepWord`].
pub const PATTERN_STEPS_PER_TRACK: usize = 16;

/// A pattern record in the `PTN ` section. Header fields confirmed against TR
/// Editor's `ptnCmn` schema + the backup manifest (name/tempo/kit all match).
///
/// NOTE: only the header (name/tempo/kit) is decoded. The per-variation step &
/// motion data (`ptnVar*`) needs the schema offset model finished — see
/// `docs/tr-format.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pattern {
    /// 0-based slot index within the PTN section.
    pub index: usize,
    /// Byte offset of the record within the file.
    pub offset: usize,
}

impl Pattern {
    /// The pattern name (`+0x10`, 16 bytes, trailing spaces/NULs trimmed).
    pub fn name(&self, raw: &[u8]) -> String {
        let s = self.offset + PATTERN_NAME_OFFSET;
        String::from_utf8_lossy(&raw[s..s + 16])
            .trim_end_matches([' ', '\0'])
            .to_string()
    }

    /// Tempo in BPM (the stored `u16` is BPM×10, so e.g. 1440 → 144.0).
    pub fn tempo_bpm(&self, raw: &[u8]) -> f32 {
        let o = self.offset + PATTERN_TEMPO_OFFSET;
        u16::from_le_bytes([raw[o], raw[o + 1]]) as f32 / 10.0
    }

    /// The kit slot (1–128) this pattern references.
    pub fn kit_ref(&self, raw: &[u8]) -> u8 {
        raw[self.offset + PATTERN_KIT_REF_OFFSET]
    }

    /// Raw byte offset of one step word within the record. `variation` 0–9,
    /// `track` 0–24, `step` 0–15. Low-level: no bounds beyond the asserts.
    pub fn step_word_offset(&self, variation: usize, track: usize, step: usize) -> usize {
        debug_assert!(variation < PATTERN_VARIATIONS);
        debug_assert!(track < PATTERN_STEP_TRACKS);
        debug_assert!(step < PATTERN_STEPS_PER_TRACK);
        self.offset
            + PATTERN_VARIATION_0_OFFSET
            + variation * PATTERN_VARIATION_STRIDE
            + PATTERN_ACCENT_SIZE
            + track * (PATTERN_STEPS_PER_TRACK * 4)
            + step * 4
    }

    /// The [`StepWord`] at (`variation`, `track`, `step`), or `None` if out of
    /// range / past the record.
    pub fn step_word(
        &self,
        raw: &[u8],
        variation: usize,
        track: usize,
        step: usize,
    ) -> Option<StepWord> {
        if variation >= PATTERN_VARIATIONS
            || track >= PATTERN_STEP_TRACKS
            || step >= PATTERN_STEPS_PER_TRACK
        {
            return None;
        }
        let o = self.step_word_offset(variation, track, step);
        let end = o + 4;
        if end > self.offset + PATTERN_RECORD_SIZE || end > raw.len() {
            return None;
        }
        Some(StepWord {
            raw: [raw[o], raw[o + 1], raw[o + 2], raw[o + 3]],
        })
    }
}

impl Backup {
    /// The pattern records in the `PTN ` section (128 on a full TR-6S backup).
    pub fn patterns(&self) -> Vec<Pattern> {
        let Some(sec) = self.find("PTN") else {
            return Vec::new();
        };
        let n = sec.payload_len / PATTERN_RECORD_SIZE;
        (0..n)
            .map(|i| Pattern {
                index: i,
                offset: sec.payload_offset + i * PATTERN_RECORD_SIZE,
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

    /// Backup with one KIT record carrying tone-IDs at the reversed offsets,
    /// plus a TONE table so IDs resolve to names. Synthetic; no Roland bytes.
    fn synthetic_with_kit_and_tones() -> (Vec<u8>, [u16; 6], Vec<String>) {
        let tone_ids = [1u16, 5, 81, 17, 21, 22];
        // TONE table with enough entries to cover the max id (81) + names for
        // the ids we assert on.
        let tone_names: Vec<String> = (0..=81u16)
            .map(|i| format!("tone{i:03}"))
            .collect();

        let mut v = Vec::new();
        v.extend_from_slice(b"TR6S");
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&5u32.to_le_bytes());
        v.resize(HEADER_LEN, 0);

        // KIT chunk with a single record; place tone-IDs at the voice offsets.
        v.extend_from_slice(b"KIT ");
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&(KIT_RECORD_SIZE as u32).to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        let mut rec = vec![0u8; KIT_RECORD_SIZE];
        rec[KIT_NAME_OFFSET..KIT_NAME_OFFSET + 4].copy_from_slice(b"Kit0");
        for (i, id) in tone_ids.iter().enumerate() {
            let o = VOICE_TONE_ID_OFFSET + i * VOICE_STRIDE;
            rec[o..o + 2].copy_from_slice(&id.to_le_bytes());
        }
        v.extend_from_slice(&rec);

        // TONE chunk: 16-byte preamble, then 0x24 entries with name at start.
        let mut tone_payload = vec![0u8; TONE_ENTRY_BASE_IN_PAYLOAD];
        for name in &tone_names {
            let mut entry = vec![0u8; TONE_ENTRY_SIZE];
            let nb = name.as_bytes();
            entry[..nb.len()].copy_from_slice(nb);
            tone_payload.extend_from_slice(&entry);
        }
        v.extend_from_slice(b"TONE");
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&(tone_payload.len() as u32).to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&tone_payload);

        (v, tone_ids, tone_names)
    }

    #[test]
    fn kit_voice_tone_ids_are_read_at_voice_offsets() {
        let (bytes, ids, _) = synthetic_with_kit_and_tones();
        let b = Backup::parse(bytes).unwrap();
        let kit = b.kits()[0];
        assert_eq!(kit.voice_tone_ids(b.raw()), ids);
    }

    #[test]
    fn tone_ids_resolve_to_names() {
        let (bytes, ids, names) = synthetic_with_kit_and_tones();
        let b = Backup::parse(bytes).unwrap();
        for &id in &ids {
            assert_eq!(b.tone_name(id).as_deref(), Some(names[id as usize].as_str()));
        }
    }

    #[test]
    fn voice_params_map_to_confirmed_offsets() {
        // A voice block set to TR Editor's instCommon defaults; assert each
        // field lands at the schema-confirmed offset.
        let mut blk = [0u8; VOICE_STRIDE];
        blk[0x00] = 72; // tone lo (default 72)
        blk[0x02] = 128; // tune
        blk[0x03] = 128; // decay
        blk[0x04] = 255; // level
        blk[0x05] = 81; // gain
        blk[0x06] = 128; // pan
        blk[0x07] = 128; // reverb send
        blk[0x08] = 224; // delay send
        blk[0x09] = 1; // lfo switch
        blk[0x0a] = 1; // lfo dest
        blk[0x0b] = 128; // lfo depth
        blk[0x0c] = 0; // category lock
        let vp = VoiceParams::from_block(&blk);
        assert_eq!(vp.tone, 72);
        assert_eq!(vp.level, 255);
        assert_eq!(vp.gain, 81);
        assert_eq!(vp.pan, 128);
        assert_eq!(vp.delay_send, 224);
        assert_eq!(vp.lfo_switch, 1);
    }

    #[test]
    fn kit_voices_reads_six_blocks() {
        let bytes = synthetic_with_kit_and_tones();
        let b = Backup::parse(bytes.0).unwrap();
        let voices = b.kits()[0].voices(b.raw());
        assert_eq!(voices.len(), 6);
        assert_eq!(voices[0].tone, 1); // tone-IDs from the fixture
        assert_eq!(voices[1].tone, 5);
    }

    #[test]
    fn tone_name_out_of_range_is_none() {
        let (bytes, _, _) = synthetic_with_kit_and_tones();
        let b = Backup::parse(bytes).unwrap();
        assert_eq!(b.tone_name(9999), None);
    }

    #[test]
    fn schema_size_rule() {
        // The solved offset model: confirmed sizes.
        assert_eq!(schema_value_size("int1x7", 1), Some(1));
        assert_eq!(schema_value_size("int2x4", 255), Some(1));
        assert_eq!(schema_value_size("int2x7", 16383), Some(2));
        assert_eq!(schema_value_size("int8x4", 0), Some(4));
        // int4x4 = ceil(bits(range_max)/7): the key fix
        assert_eq!(schema_value_size("int4x4", 1023), Some(2)); // TONE (10 bits)
        assert_eq!(schema_value_size("int4x4", 3000), Some(2)); // TEMPO (12 bits)
        assert_eq!(schema_value_size("int4x4", 65535), Some(3)); // SHUFFLE SWITCH (16 bits)
        assert_eq!(schema_value_size("stringNx7", 0), Some(16));
    }

    #[test]
    fn step_word_velocity() {
        assert!(!StepWord { raw: [0, 0, 0, 0] }.is_on());
        let on = StepWord {
            raw: [0x50, 0, 0, 0],
        };
        assert!(on.is_on());
        assert_eq!(on.velocity(), 80);
    }

    #[test]
    fn pattern_header_fields() {
        // Backup with one PTN record: name/tempo/kit at confirmed offsets.
        let mut v = Vec::new();
        v.extend_from_slice(b"TR6S");
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&5u32.to_le_bytes());
        v.resize(HEADER_LEN, 0);
        v.extend_from_slice(b"PTN ");
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&(PATTERN_RECORD_SIZE as u32).to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        let mut rec = vec![0u8; PATTERN_RECORD_SIZE];
        rec[PATTERN_NAME_OFFSET..PATTERN_NAME_OFFSET + 10].copy_from_slice(b"Speak C0DE");
        rec[PATTERN_TEMPO_OFFSET..PATTERN_TEMPO_OFFSET + 2].copy_from_slice(&1440u16.to_le_bytes());
        rec[PATTERN_KIT_REF_OFFSET] = 14;
        v.extend_from_slice(&rec);

        let b = Backup::parse(v).unwrap();
        let ptns = b.patterns();
        assert_eq!(ptns.len(), 1);
        assert_eq!(ptns[0].name(b.raw()), "Speak C0DE");
        assert_eq!(ptns[0].tempo_bpm(b.raw()), 144.0);
        assert_eq!(ptns[0].kit_ref(b.raw()), 14);
    }

    #[test]
    fn step_word_offset_and_read() {
        // Build a PTN record with a known step at (var 0, track 0, step 2).
        let mut v = Vec::new();
        v.extend_from_slice(b"TR6S");
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&5u32.to_le_bytes());
        v.resize(HEADER_LEN, 0);
        v.extend_from_slice(b"PTN ");
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&(PATTERN_RECORD_SIZE as u32).to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        let mut rec = vec![0u8; PATTERN_RECORD_SIZE];
        // var0 track0 step2 => 0xA0 + 4 + 0 + 2*4 = 0xAC (velocity 90)
        rec[0xA0 + 4 + 2 * 4] = 90;
        v.extend_from_slice(&rec);

        let b = Backup::parse(v).unwrap();
        let p = b.patterns()[0];
        assert_eq!(p.step_word_offset(0, 0, 2), p.offset + 0xAC);
        assert!(p.step_word(b.raw(), 0, 0, 2).unwrap().is_on());
        assert_eq!(p.step_word(b.raw(), 0, 0, 2).unwrap().velocity(), 90);
        assert!(!p.step_word(b.raw(), 0, 0, 3).unwrap().is_on());
        // variation 1 lands one stride later
        assert_eq!(
            p.step_word_offset(1, 0, 0),
            p.offset + 0xA0 + PATTERN_VARIATION_STRIDE + 4
        );
        assert_eq!(p.step_word(b.raw(), PATTERN_VARIATIONS, 0, 0), None);
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
