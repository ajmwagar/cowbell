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
//! Record internals are decoded incrementally: kit framing, names, voice blocks
//! and tone IDs; pattern headers, the variation/array-slot map, the step word
//! (velocity, sub step, ALTERNATE) and the motion lanes (tune/decay/ctrl and the
//! delay/reverb/MFX planes). Still open: per-step probability and part of the
//! motion flags byte — see `docs/tr-format.md`.

pub mod fx;
pub mod sys;

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

    /// Mutable access to the raw bytes, for low-level edits (e.g. a builder
    /// writing step velocities). `to_bytes()` reflects whatever is written here.
    ///
    /// There is **no per-record checksum**: the `+0x08` field is zero for every
    /// record except record 0 of a section, so length-preserving edits to user
    /// slots (records ≥ 1) need no recomputation. Record 0 carries a
    /// section-level token (unresolved algorithm); leave it untouched. See
    /// `docs/tr-format.md`. Device acceptance of a modified backup is still
    /// pending a hardware test.
    pub fn raw_mut(&mut self) -> &mut [u8] {
        &mut self.raw
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

/// A fixed-size record block with a **byte-exact Roland ↔ typed** mapping.
///
/// [`from_block`](RolandBlock::from_block) decodes a struct of typed fields from
/// a byte block; [`write_to`](RolandBlock::write_to) is its exact inverse,
/// writing those fields back **in place and touching only the decoded bytes** —
/// unknown/reserved bytes in the block are preserved, upholding the crate's
/// losslessness contract. [`LEN`](RolandBlock::LEN) is the decoded span (the
/// minimum block size the two operate within).
///
/// The types stay byte-faithful on purpose (raw `u8` fields, not enums): a rich
/// enum would have to carry an `Unknown(u8)` for every out-of-range or reserved
/// value to avoid losing it on write. Semantic views (enums, ranges) live one
/// layer up (`tr-studio`) or as accessor methods (`type_name()` etc.).
pub trait RolandBlock: Sized {
    /// The decoded span in bytes: `from_block` reads and `write_to` fills the
    /// range `0..LEN`.
    const LEN: usize;
    /// Decode from a block (must be at least [`LEN`](Self::LEN) bytes).
    fn from_block(bytes: &[u8]) -> Self;
    /// Write these fields back in place — the exact inverse of `from_block`,
    /// touching only the decoded bytes within `0..LEN`.
    fn write_to(&self, bytes: &mut [u8]);
}

/// The confirmed `instCommon` voice parameters (offsets relative to the voice
/// block start). Named per TR Editor's `Script.xml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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

impl RolandBlock for VoiceParams {
    /// 13 decoded bytes (`+0x00..=0x0C`); the voice block is `0x34` total, the
    /// rest being still-unknown reserve preserved by `write_to`.
    const LEN: usize = 0x0d;

    fn from_block(b: &[u8]) -> VoiceParams {
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

    fn write_to(&self, block: &mut [u8]) {
        block[0x00..0x02].copy_from_slice(&self.tone.to_le_bytes());
        block[0x02] = self.tune;
        block[0x03] = self.decay;
        block[0x04] = self.level;
        block[0x05] = self.gain;
        block[0x06] = self.pan;
        block[0x07] = self.reverb_send;
        block[0x08] = self.delay_send;
        block[0x09] = self.lfo_switch;
        block[0x0a] = self.lfo_dest;
        block[0x0b] = self.lfo_depth;
        block[0x0c] = self.category_lock;
    }
}

/// Write an ASCII name into a fixed-length field, space-padded — the inverse of
/// the readers' `trim_end_matches([' ', '\0'])`. Truncates to `len` bytes.
/// Returns false (writing nothing) if the field is out of range. Length-
/// preserving: exactly `len` bytes are written.
pub(crate) fn write_name_field(raw: &mut [u8], off: usize, len: usize, name: &str) -> bool {
    if off + len > raw.len() {
        return false;
    }
    let bytes = name.as_bytes();
    let n = bytes.len().min(len);
    raw[off..off + n].copy_from_slice(&bytes[..n]);
    for b in &mut raw[off + n..off + len] {
        *b = b' ';
    }
    true
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

    /// Byte offset of voice `i`'s block within the file.
    fn voice_offset(&self, voice: usize) -> usize {
        self.offset + VOICE_TONE_ID_OFFSET + voice * VOICE_STRIDE
    }

    /// Set the kit name (`+0x10`, 16 bytes, space-padded). The inverse of
    /// [`Kit::name`]. Returns false if out of range.
    pub fn set_name(&self, raw: &mut [u8], name: &str) -> bool {
        write_name_field(raw, self.offset + KIT_NAME_OFFSET, KIT_NAME_LEN, name)
    }

    /// Set voice `voice`'s tone-ID (`u16` LE at the voice block start). `voice`
    /// 0–5 (BD…OH). Returns false if out of range.
    pub fn set_voice_tone(&self, raw: &mut [u8], voice: usize, tone: u16) -> bool {
        if voice >= VOICES.len() {
            return false;
        }
        let o = self.voice_offset(voice);
        if o + 2 > raw.len() {
            return false;
        }
        raw[o..o + 2].copy_from_slice(&tone.to_le_bytes());
        true
    }

    /// Write a voice's full [`VoiceParams`] back (the inverse of one entry of
    /// [`Kit::voices`]). `voice` 0–5. Touches only the decoded param bytes; the
    /// rest of the `0x34` voice block is preserved. Returns false if out of
    /// range.
    pub fn set_voice_params(&self, raw: &mut [u8], voice: usize, params: &VoiceParams) -> bool {
        if voice >= VOICES.len() {
            return false;
        }
        let o = self.voice_offset(voice);
        if o + VOICE_STRIDE > raw.len() {
            return false;
        }
        params.write_to(&mut raw[o..o + VOICE_STRIDE]);
        true
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

/// A step's sub-step (retrigger) mode — TR Editor's `subStep` combo, whose
/// string table is `1/2,1/3,1/4,FLAM`.
///
/// The stored field is the **number of hits**, not the combo index: `2`/`3`/`4`
/// retrigger the step that many times within its slot, and `1` is the flam
/// (a grace note, spaced by the pattern's `FLAM SPACING`). See
/// [`StepWord::sub_step`] and `docs/tr-format.md` for the evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SubStep {
    /// Grace note before the beat; spacing comes from `ptnCmn.FLAM SPACING`.
    Flam,
    /// `1/2` — two hits in the step.
    Half,
    /// `1/3` — three hits in the step.
    Third,
    /// `1/4` — four hits in the step.
    Quarter,
}

impl SubStep {
    /// The stored encoding (1–4).
    pub fn to_raw(self) -> u8 {
        match self {
            SubStep::Flam => 1,
            SubStep::Half => 2,
            SubStep::Third => 3,
            SubStep::Quarter => 4,
        }
    }

    /// Decode the stored field; `None` for 0 (no sub-step) or an unknown value.
    pub fn from_raw(v: u8) -> Option<SubStep> {
        Some(match v {
            1 => SubStep::Flam,
            2 => SubStep::Half,
            3 => SubStep::Third,
            4 => SubStep::Quarter,
            _ => return None,
        })
    }

    /// The label TR Editor shows (`FLAM`, `1/2`, `1/3`, `1/4`).
    pub fn label(self) -> &'static str {
        match self {
            SubStep::Flam => "FLAM",
            SubStep::Half => "1/2",
            SubStep::Third => "1/3",
            SubStep::Quarter => "1/4",
        }
    }
}

/// Bit mask of the sub-step field within step-word byte 1.
pub const STEP_SUB_STEP_MASK: u8 = 0x07;
/// Bit mask of the ALTERNATE flag within step-word byte 1.
pub const STEP_ALTERNATE_MASK: u8 = 0x80;

/// A pattern step word (`int8x4`, 4 bytes), decoded.
///
/// | Byte | Bits | Field |
/// | ---- | ---- | ----- |
/// | 0 | 0–7 | **velocity** (1–127; 0 = step off) |
/// | 1 | 0–2 | **sub step** — 0 = none, else [`SubStep`] |
/// | 1 | 3–6 | unknown (always 0 on the v1.51 reference backup) |
/// | 1 | 7 | **ALTERNATE** flag |
/// | 2–3 | — | unknown (always 0 on the v1.51 reference backup) |
///
/// TR Editor exposes exactly four per-step attributes — velocity, probability,
/// sub step, alternate — so the unknown bits are where per-step **probability**
/// (0–10) lives. It reads 0 for every step of every factory pattern in the
/// reference backup, so its bit position is unconfirmed; per-step probability
/// appears to be a later-firmware feature. The setters below preserve those
/// bits, so editing a step can never destroy them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct StepWord {
    pub raw: [u8; 4],
}

impl StepWord {
    /// Step velocity (0 = the step is off).
    pub fn velocity(&self) -> u8 {
        self.raw[0]
    }

    /// Whether the step triggers at all.
    pub fn is_on(&self) -> bool {
        self.raw[0] != 0
    }

    /// The step's sub-step mode, or `None` if it plays a single hit.
    pub fn sub_step(&self) -> Option<SubStep> {
        SubStep::from_raw(self.raw[1] & STEP_SUB_STEP_MASK)
    }

    /// Whether the step is flagged ALTERNATE (plays the voice's alternate tone).
    pub fn is_alternate(&self) -> bool {
        self.raw[1] & STEP_ALTERNATE_MASK != 0
    }

    /// Set the velocity (0 turns the step off).
    pub fn set_velocity(&mut self, velocity: u8) {
        self.raw[0] = velocity;
    }

    /// Set (or clear, with `None`) the sub-step mode. Preserves every other bit.
    pub fn set_sub_step(&mut self, sub: Option<SubStep>) {
        let bits = sub.map_or(0, SubStep::to_raw);
        self.raw[1] = (self.raw[1] & !STEP_SUB_STEP_MASK) | bits;
    }

    /// Set the ALTERNATE flag. Preserves every other bit.
    pub fn set_alternate(&mut self, on: bool) {
        if on {
            self.raw[1] |= STEP_ALTERNATE_MASK;
        } else {
            self.raw[1] &= !STEP_ALTERNATE_MASK;
        }
    }

    /// The bits this crate does not understand yet (byte 1 bits 3–6, bytes 2–3).
    /// Zero on every step of the reference backup; non-zero means a step carries
    /// something we would drop if we re-encoded from the typed view alone.
    pub fn unknown_bits(&self) -> u32 {
        u32::from(self.raw[1] & !(STEP_SUB_STEP_MASK | STEP_ALTERNATE_MASK))
            | u32::from(self.raw[2]) << 8
            | u32::from(self.raw[3]) << 16
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
/// 64-byte array slots per variation (`ptnVar01`…`ptnVar25`). Only the first
/// [`PATTERN_STEP_TRACKS`] hold step words — see [`track_role`].
pub const PATTERN_ARRAY_SLOTS: usize = 25;
/// Slots that hold [`StepWord`]s: `ptnVar01`…`ptnVar11` (INST01–INST11) plus
/// `ptnVar12` (TRIG). Named from TR Editor's `editor_pattern_inst` panels.
pub const PATTERN_STEP_TRACKS: usize = 12;
/// Steps per track (`PTN00`…`PTN15`), each a 4-byte [`StepWord`].
pub const PATTERN_STEPS_PER_TRACK: usize = 16;

/// The 11 instrument tracks in `ptnVar01`…`ptnVar11`, in slot order. This is the
/// **TR-8S** panel layout; a TR-6S stores its six voices in slots 0–5 and leaves
/// 6–10 empty, so on a TR-6S backup slot 3/4/5 are its HC/CH/OH — use [`VOICES`]
/// there. Verified on the reference backup: slots 6–10 are all zero and slot 4
/// (the TR-6S closed hat) is by far the busiest track.
pub const INST_TRACKS: [&str; 11] = [
    "BD", "SD", "LT", "MT", "HT", "RS", "HC", "CH", "OH", "CC", "RC",
];
/// Slot index of the TRIG (trigger-out) track, `ptnVar12`.
pub const PATTERN_TRIGGER_TRACK: usize = 11;

/// What a `ptnVar` array slot (0-based, i.e. `ptnVar{n+1}`) actually holds.
/// From TR Editor's `Script.xml` field names, corroborated by the reference
/// backup: the motion slots for unused voices are zero and the step slots are
/// the ones that read as musical patterns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TrackRole {
    /// `ptnVar01`…`ptnVar11` — `INSTnn PTNnn` step words for instrument `0..11`.
    Inst(usize),
    /// `ptnVar12` — `TRIG PTNnn` step words (trigger out).
    Trigger,
    /// `ptnVar13`…`ptnVar23` — `INSTnn PRMnn`, per-step motion for instrument
    /// `0..11`. Not step words.
    Motion(usize),
    /// `ptnVar24`/`ptnVar25` — `OTH0`/`OTH1 PRMnn` motion planes.
    OtherMotion(usize),
}

/// The role of array slot `track` (0-based), or `None` past the last slot.
pub fn track_role(track: usize) -> Option<TrackRole> {
    Some(match track {
        0..=10 => TrackRole::Inst(track),
        11 => TrackRole::Trigger,
        12..=22 => TrackRole::Motion(track - 12),
        23 | 24 => TrackRole::OtherMotion(track - 23),
        _ => return None,
    })
}

// --- Motion (`PRM`) arrays ----------------------------------------------------
// Mapped from TR Editor's `motionPrm` dataTable, whose `<order>` field is the
// byte index of a parameter within the 4-byte PRM word. VELOCITY and
// PROBABILITY carry `order -1` — they live in the [`StepWord`], not here.
// See docs/tr-format.md.

/// Parameter lanes in a [`MotionWord`]; the 4th byte is [`MotionWord::flags`].
pub const MOTION_LANES: usize = 3;

/// Lane names for instrument motion slots (`INSTnn PRM`), by lane index.
pub const MOTION_LANES_INST: [&str; MOTION_LANES] = ["TUNE", "DECAY", "CTRL"];
/// Lane names for `OTH0` (`ptnVar24`) — the delay plane.
pub const MOTION_LANES_DELAY: [&str; MOTION_LANES] =
    ["DELAY FEEDBACK", "DELAY LEVEL", "DELAY TIME"];
/// Lane names for `OTH1` (`ptnVar25`) — the reverb + master-FX plane.
pub const MOTION_LANES_REVERB_MFX: [&str; MOTION_LANES] = ["REVERB LEVEL", "MFX SW", "MFX DEPTH"];

/// The parameter recorded by `lane` of a motion word in array slot `track`, or
/// `None` if the slot is not a motion slot / the lane is out of range.
pub fn motion_lane_name(track: usize, lane: usize) -> Option<&'static str> {
    let names = match track_role(track)? {
        TrackRole::Motion(_) => &MOTION_LANES_INST,
        TrackRole::OtherMotion(0) => &MOTION_LANES_DELAY,
        TrackRole::OtherMotion(_) => &MOTION_LANES_REVERB_MFX,
        _ => return None,
    };
    names.get(lane).copied()
}

/// A motion word (`INSTnn PRMnn` / `OTHn PRMnn`, `int8x4`, 4 bytes).
///
/// | Byte | Field |
/// | ---- | ----- |
/// | 0 | lane 0 value — see [`motion_lane_name`] |
/// | 1 | lane 1 value |
/// | 2 | lane 2 value |
/// | 3 | flags — bit 7 = lane 0 recorded, bit 6 = lane 1 recorded, rest unknown |
///
/// A lane's value of 0 is ambiguous on its own (0 is a legal parameter value),
/// which is what the flag bits are for. Bits 7 and 6 are confirmed: across all
/// 8,933 live motion words in the reference backup, a non-zero lane 0 always has
/// bit 7 set and a non-zero lane 1 always has bit 6 set, with **zero**
/// violations. **Lane 2's flag is not resolved** — no single bit implies it
/// across slots, so [`MotionWord::lane_recorded`] returns `None` for it rather
/// than guessing. Tune and Ctrl are bipolar with centre 128 (`<offset>128`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MotionWord {
    pub raw: [u8; 4],
}

impl MotionWord {
    /// The raw byte of a lane (0–2), or 0 if out of range. Does not consult the
    /// flags — see [`MotionWord::lane`].
    pub fn lane_raw(&self, lane: usize) -> u8 {
        if lane < MOTION_LANES {
            self.raw[lane]
        } else {
            0
        }
    }

    /// The lane's value if the word records one, using the confirmed flag bits.
    /// Returns `None` when the flag is clear, and — for lane 2, whose flag bit
    /// is unknown — falls back to "non-zero means recorded", which under-reports
    /// a genuine recorded 0.
    pub fn lane(&self, lane: usize) -> Option<u8> {
        match self.lane_recorded(lane) {
            Some(true) => Some(self.lane_raw(lane)),
            Some(false) => None,
            None => (self.lane_raw(lane) != 0).then(|| self.lane_raw(lane)),
        }
    }

    /// Whether the lane records a value: `Some(true)`/`Some(false)` for the
    /// confirmed lanes 0 and 1, `None` (undetermined) for lane 2 and beyond.
    pub fn lane_recorded(&self, lane: usize) -> Option<bool> {
        match lane {
            0 => Some(self.raw[3] & 0x80 != 0),
            1 => Some(self.raw[3] & 0x40 != 0),
            _ => None,
        }
    }

    /// The flags byte (byte 3). Never 0 on a live word in the reference backup.
    pub fn flags(&self) -> u8 {
        self.raw[3]
    }

    /// Whether the word is entirely empty (no motion recorded at this step).
    pub fn is_empty(&self) -> bool {
        self.raw == [0; 4]
    }
}

/// A pattern record in the `PTN ` section. Header fields confirmed against TR
/// Editor's `ptnCmn` schema + the backup manifest (name/tempo/kit all match);
/// the body is addressed by array slot via [`track_role`] — [`StepWord`]s in
/// slots 0–11, [`MotionWord`]s in slots 12–24. See `docs/tr-format.md`.
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

    /// Set the pattern name (`+0x10`, 16 bytes, space-padded) — inverse of
    /// [`Pattern::name`]. Returns false if out of range.
    pub fn set_name(&self, raw: &mut [u8], name: &str) -> bool {
        write_name_field(raw, self.offset + PATTERN_NAME_OFFSET, 16, name)
    }

    /// Set the tempo in BPM (stored as `u16` LE `BPM×10`) — inverse of
    /// [`Pattern::tempo_bpm`]. Clamps to the schema range 40.0–300.0 BPM. Returns
    /// false if out of range.
    pub fn set_tempo_bpm(&self, raw: &mut [u8], bpm: f32) -> bool {
        let o = self.offset + PATTERN_TEMPO_OFFSET;
        if o + 2 > raw.len() {
            return false;
        }
        // `ptnCmn.TEMPO` range is 400–3000 (BPM×10).
        let stored = (bpm * 10.0).round().clamp(400.0, 3000.0) as u16;
        raw[o..o + 2].copy_from_slice(&stored.to_le_bytes());
        true
    }

    /// Set the referenced kit slot (1–128) — inverse of [`Pattern::kit_ref`].
    /// Returns false if out of range.
    pub fn set_kit_ref(&self, raw: &mut [u8], kit: u8) -> bool {
        let o = self.offset + PATTERN_KIT_REF_OFFSET;
        if o >= raw.len() {
            return false;
        }
        raw[o] = kit;
        true
    }

    /// Raw byte offset of one 4-byte word within the record. `variation` 0–9,
    /// `track` (array slot) 0–24, `step` 0–15. Low-level and role-agnostic: it
    /// addresses motion slots as readily as step slots — see [`track_role`].
    pub fn step_word_offset(&self, variation: usize, track: usize, step: usize) -> usize {
        debug_assert!(variation < PATTERN_VARIATIONS);
        debug_assert!(track < PATTERN_ARRAY_SLOTS);
        debug_assert!(step < PATTERN_STEPS_PER_TRACK);
        self.offset
            + PATTERN_VARIATION_0_OFFSET
            + variation * PATTERN_VARIATION_STRIDE
            + PATTERN_ACCENT_SIZE
            + track * (PATTERN_STEPS_PER_TRACK * 4)
            + step * 4
    }

    /// The [`StepWord`] at (`variation`, `track`, `step`), or `None` if out of
    /// range / past the record. `track` must be a step slot (`< 12`); motion
    /// slots are not step words and are rejected.
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

    /// Low-level write: set a step's velocity byte (0 = off). Returns false if
    /// out of range. Does not touch the record checksum — see [`Backup::raw_mut`].
    pub fn set_step_velocity(
        &self,
        raw: &mut [u8],
        variation: usize,
        track: usize,
        step: usize,
        velocity: u8,
    ) -> bool {
        if variation >= PATTERN_VARIATIONS
            || track >= PATTERN_STEP_TRACKS
            || step >= PATTERN_STEPS_PER_TRACK
        {
            return false;
        }
        let o = self.step_word_offset(variation, track, step);
        if o + 4 > self.offset + PATTERN_RECORD_SIZE || o >= raw.len() {
            return false;
        }
        raw[o] = velocity;
        true
    }

    /// Low-level write of a whole [`StepWord`]: velocity, sub step, alternate,
    /// and whatever unknown bits the word carries. Read-modify-write via
    /// [`Pattern::step_word`] and the `StepWord` setters to keep the bits this
    /// crate does not understand intact. Returns false if out of range.
    pub fn set_step_word(
        &self,
        raw: &mut [u8],
        variation: usize,
        track: usize,
        step: usize,
        word: StepWord,
    ) -> bool {
        if variation >= PATTERN_VARIATIONS
            || track >= PATTERN_STEP_TRACKS
            || step >= PATTERN_STEPS_PER_TRACK
        {
            return false;
        }
        let o = self.step_word_offset(variation, track, step);
        if o + 4 > self.offset + PATTERN_RECORD_SIZE || o + 4 > raw.len() {
            return false;
        }
        raw[o..o + 4].copy_from_slice(&word.raw);
        true
    }

    /// The [`MotionWord`] at (`variation`, `track`, `step`), or `None` if
    /// `track` is not a motion slot (12–24) or the read is out of range.
    pub fn motion_word(
        &self,
        raw: &[u8],
        variation: usize,
        track: usize,
        step: usize,
    ) -> Option<MotionWord> {
        if variation >= PATTERN_VARIATIONS
            || step >= PATTERN_STEPS_PER_TRACK
            || !matches!(
                track_role(track),
                Some(TrackRole::Motion(_) | TrackRole::OtherMotion(_))
            )
        {
            return None;
        }
        let o = self.step_word_offset(variation, track, step);
        let end = o + 4;
        if end > self.offset + PATTERN_RECORD_SIZE || end > raw.len() {
            return None;
        }
        Some(MotionWord {
            raw: [raw[o], raw[o + 1], raw[o + 2], raw[o + 3]],
        })
    }

    /// Low-level write of a whole [`MotionWord`]. The flags byte carries bits
    /// this crate has not decoded, so read-modify-write rather than composing a
    /// word from scratch. Returns false if out of range.
    pub fn set_motion_word(
        &self,
        raw: &mut [u8],
        variation: usize,
        track: usize,
        step: usize,
        word: MotionWord,
    ) -> bool {
        if self.motion_word(raw, variation, track, step).is_none() {
            return false;
        }
        let o = self.step_word_offset(variation, track, step);
        raw[o..o + 4].copy_from_slice(&word.raw);
        true
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
        let tone_names: Vec<String> = (0..=81u16).map(|i| format!("tone{i:03}")).collect();

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
            assert_eq!(
                b.tone_name(id).as_deref(),
                Some(names[id as usize].as_str())
            );
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
    fn step_word_sub_step_and_alternate() {
        // Byte 1 low bits = sub step (hit count), bit 7 = ALTERNATE. These are
        // the eight byte-1 values that occur on the reference backup.
        let w = |b1| StepWord {
            raw: [0x50, b1, 0, 0],
        };
        assert_eq!(w(0x00).sub_step(), None);
        assert_eq!(w(0x01).sub_step(), Some(SubStep::Flam));
        assert_eq!(w(0x02).sub_step(), Some(SubStep::Half));
        assert_eq!(w(0x03).sub_step(), Some(SubStep::Third));
        assert_eq!(w(0x04).sub_step(), Some(SubStep::Quarter));
        for b1 in [0x00, 0x01, 0x02, 0x03, 0x04] {
            assert!(!w(b1).is_alternate());
        }
        for b1 in [0x80, 0x81, 0x82] {
            assert!(w(b1).is_alternate());
        }
        assert_eq!(w(0x82).sub_step(), Some(SubStep::Half));
        assert_eq!(SubStep::Quarter.label(), "1/4");
        assert_eq!(SubStep::Flam.to_raw(), 1);
    }

    #[test]
    fn step_word_setters_preserve_unknown_bits() {
        // Bits we have not decoded (byte 1 bits 3-6, bytes 2-3) must survive an
        // edit — the librarian's losslessness contract, at step granularity.
        let mut w = StepWord {
            raw: [0x50, 0b0111_1000, 0xAB, 0xCD],
        };
        let unknown = w.unknown_bits();
        w.set_velocity(100);
        w.set_sub_step(Some(SubStep::Third));
        w.set_alternate(true);
        assert_eq!(w.velocity(), 100);
        assert_eq!(w.sub_step(), Some(SubStep::Third));
        assert!(w.is_alternate());
        assert_eq!(w.unknown_bits(), unknown);

        w.set_sub_step(None);
        w.set_alternate(false);
        assert_eq!(w.sub_step(), None);
        assert!(!w.is_alternate());
        assert_eq!(w.unknown_bits(), unknown);
        assert_eq!(w.raw[1], 0b0111_1000);
        assert_eq!(
            StepWord {
                raw: [0x50, 0, 0, 0]
            }
            .unknown_bits(),
            0
        );
    }

    #[test]
    fn motion_lane_names_follow_the_slot_role() {
        // INST motion slots: TUNE/DECAY/CTRL, from motionPrm <order>.
        assert_eq!(motion_lane_name(12, 0), Some("TUNE"));
        assert_eq!(motion_lane_name(12, 1), Some("DECAY"));
        assert_eq!(motion_lane_name(12, 2), Some("CTRL"));
        assert_eq!(motion_lane_name(22, 0), Some("TUNE"));
        // OTH0 = the delay plane, OTH1 = reverb + master FX.
        assert_eq!(motion_lane_name(23, 0), Some("DELAY FEEDBACK"));
        assert_eq!(motion_lane_name(23, 2), Some("DELAY TIME"));
        assert_eq!(motion_lane_name(24, 0), Some("REVERB LEVEL"));
        assert_eq!(motion_lane_name(24, 1), Some("MFX SW"));
        assert_eq!(motion_lane_name(24, 2), Some("MFX DEPTH"));
        // Step slots and the flags byte are not motion lanes.
        assert_eq!(motion_lane_name(0, 0), None);
        assert_eq!(motion_lane_name(11, 0), None);
        assert_eq!(motion_lane_name(12, MOTION_LANES), None);
    }

    #[test]
    fn motion_word_lanes_and_flags() {
        // Both confirmed flags set, all three lanes carrying values.
        let w = MotionWord {
            raw: [124, 116, 88, 0b1100_0010],
        };
        assert_eq!(w.lane_recorded(0), Some(true));
        assert_eq!(w.lane_recorded(1), Some(true));
        assert_eq!(w.lane_recorded(2), None); // flag bit unresolved
        assert_eq!(w.lane(0), Some(124));
        assert_eq!(w.lane(1), Some(116));
        assert_eq!(w.lane(2), Some(88)); // falls back to non-zero
        assert_eq!(w.flags(), 0b1100_0010);
        assert!(!w.is_empty());

        // Flags clear: a 0 byte means "not recorded", not "recorded as 0".
        let w = MotionWord {
            raw: [0, 0, 0, 0b0000_0010],
        };
        assert_eq!(w.lane(0), None);
        assert_eq!(w.lane(1), None);
        assert_eq!(w.lane(2), None);
        assert_eq!(w.lane_raw(0), 0);
        assert_eq!(w.lane_raw(MOTION_LANES), 0);

        // A recorded 0 on a confirmed lane is distinguishable from "not set".
        let w = MotionWord {
            raw: [0, 0, 0, 0b1000_0000],
        };
        assert_eq!(w.lane(0), Some(0));
        assert_eq!(w.lane(1), None);
        assert!(MotionWord { raw: [0; 4] }.is_empty());
    }

    #[test]
    fn motion_word_reads_only_from_motion_slots() {
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
        // var0, slot 12 (BD motion), step 1 => 0xA0 + 4 + 12*64 + 4
        let o = 0xA0 + 4 + 12 * 64 + 4;
        rec[o..o + 4].copy_from_slice(&[124, 116, 88, 0b1100_0010]);
        v.extend_from_slice(&rec);

        let mut b = Backup::parse(v).unwrap();
        let p = b.patterns()[0];
        let w = p.motion_word(b.raw(), 0, 12, 1).unwrap();
        assert_eq!(w.lane(0), Some(124));
        assert_eq!(motion_lane_name(12, 0), Some("TUNE"));
        // Step slots are rejected, and step_word rejects motion slots.
        assert_eq!(p.motion_word(b.raw(), 0, 0, 1), None);
        assert_eq!(p.motion_word(b.raw(), 0, PATTERN_ARRAY_SLOTS, 1), None);
        assert_eq!(p.step_word(b.raw(), 0, 12, 1), None);

        // Round-trip a write through the same offset.
        let mut w2 = w;
        w2.raw[0] = 200;
        assert!(p.set_motion_word(b.raw_mut(), 0, 12, 1, w2));
        assert_eq!(p.motion_word(b.raw(), 0, 12, 1).unwrap().lane(0), Some(200));
        assert!(!p.set_motion_word(b.raw_mut(), 0, 0, 1, w2));
    }

    #[test]
    fn array_slot_roles() {
        assert_eq!(track_role(0), Some(TrackRole::Inst(0)));
        assert_eq!(track_role(10), Some(TrackRole::Inst(10)));
        assert_eq!(track_role(PATTERN_TRIGGER_TRACK), Some(TrackRole::Trigger));
        assert_eq!(track_role(12), Some(TrackRole::Motion(0)));
        assert_eq!(track_role(22), Some(TrackRole::Motion(10)));
        assert_eq!(track_role(23), Some(TrackRole::OtherMotion(0)));
        assert_eq!(track_role(24), Some(TrackRole::OtherMotion(1)));
        assert_eq!(track_role(PATTERN_ARRAY_SLOTS), None);
        // The step slots are exactly the Inst + Trigger ones.
        assert_eq!(PATTERN_STEP_TRACKS, INST_TRACKS.len() + 1);
        // The variation stride is fully accounted for: accent + 25 arrays + the
        // 832-byte ptnVar26 reserve block.
        assert_eq!(
            PATTERN_ACCENT_SIZE + PATTERN_ARRAY_SLOTS * PATTERN_STEPS_PER_TRACK * 4 + 832,
            PATTERN_VARIATION_STRIDE
        );
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

    // ---- typed write/edit API (cowbell-j6b.9) --------------------------------

    /// Assert that `before`/`after` (equal length) differ **only** inside the
    /// byte window `[off, off + len)` — the write API's losslessness contract:
    /// an edit never moves a byte outside the field it targets. (It need not
    /// change every byte in the window — e.g. writing a tempo only flips the
    /// bytes that actually differ.)
    fn assert_changed_within(before: &[u8], after: &[u8], off: usize, len: usize) {
        assert_eq!(before.len(), after.len());
        for i in 0..before.len() {
            if before[i] != after[i] {
                assert!(
                    (off..off + len).contains(&i),
                    "byte 0x{i:x} changed outside the field [0x{off:x}, 0x{:x})",
                    off + len
                );
            }
        }
        assert!(
            (off..off + len).any(|i| before[i] != after[i]),
            "no byte changed in the field"
        );
    }

    #[test]
    fn kit_setters_write_only_their_fields() {
        let (bytes, _, _) = synthetic_with_kit_and_tones();
        let mut b = Backup::parse(bytes).unwrap();
        let orig = b.to_bytes();
        let kit = b.kits()[0];
        let base = kit.offset;

        // name
        assert!(kit.set_name(b.raw_mut(), "MyKit"));
        assert_eq!(kit.name(b.raw()), "MyKit");
        assert_changed_within(&orig, b.raw(), base + KIT_NAME_OFFSET, KIT_NAME_LEN);

        // voice tone (u16 LE at the voice block start)
        let before = b.to_bytes();
        assert!(kit.set_voice_tone(b.raw_mut(), 2, 0x0321));
        assert_eq!(kit.voice_tone_ids(b.raw())[2], 0x0321);
        assert_changed_within(
            &before,
            b.raw(),
            base + VOICE_TONE_ID_OFFSET + 2 * VOICE_STRIDE,
            2,
        );

        // full VoiceParams write-back touches only the 13 decoded bytes
        // (`+0x00..=0x0C`); the rest of the 0x34 block is preserved.
        let before = b.to_bytes();
        let mut vp = kit.voices(b.raw())[3];
        vp.tune = 200;
        vp.level = 111;
        vp.category_lock = 1;
        assert!(kit.set_voice_params(b.raw_mut(), 3, &vp));
        assert_eq!(kit.voices(b.raw())[3], vp);
        assert_changed_within(
            &before,
            b.raw(),
            base + VOICE_TONE_ID_OFFSET + 3 * VOICE_STRIDE,
            0x0d,
        );

        // out-of-range voice is refused, no panic.
        assert!(!kit.set_voice_tone(b.raw_mut(), 6, 0));
        assert!(!kit.set_voice_params(b.raw_mut(), 9, &vp));
    }

    #[test]
    fn pattern_setters_write_only_their_fields() {
        // one PTN record.
        let mut v = Vec::new();
        v.extend_from_slice(b"TR6S");
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&5u32.to_le_bytes());
        v.resize(HEADER_LEN, 0);
        v.extend_from_slice(b"PTN ");
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&(PATTERN_RECORD_SIZE as u32).to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&vec![0u8; PATTERN_RECORD_SIZE]);
        let mut b = Backup::parse(v).unwrap();
        let p = b.patterns()[0];
        let base = p.offset;

        let orig = b.to_bytes();
        assert!(p.set_name(b.raw_mut(), "Beat One"));
        assert_eq!(p.name(b.raw()), "Beat One");
        assert_changed_within(&orig, b.raw(), base + PATTERN_NAME_OFFSET, 16);

        let before = b.to_bytes();
        assert!(p.set_tempo_bpm(b.raw_mut(), 128.0));
        assert_eq!(p.tempo_bpm(b.raw()), 128.0);
        assert_changed_within(&before, b.raw(), base + PATTERN_TEMPO_OFFSET, 2);
        // clamps to the schema range (40.0–300.0).
        p.set_tempo_bpm(b.raw_mut(), 9000.0);
        assert_eq!(p.tempo_bpm(b.raw()), 300.0);

        let before = b.to_bytes();
        assert!(p.set_kit_ref(b.raw_mut(), 42));
        assert_eq!(p.kit_ref(b.raw()), 42);
        assert_changed_within(&before, b.raw(), base + PATTERN_KIT_REF_OFFSET, 1);
    }

    /// Generic losslessness proof for any [`RolandBlock`]: decode arbitrary
    /// bytes, encode, decode again — `write_to` must be the exact inverse of
    /// `from_block` over `0..LEN`. One helper covers every block type, which is
    /// the point of the trait.
    fn assert_block_roundtrip<T: RolandBlock + PartialEq + std::fmt::Debug>() {
        // distinct 7-bit values so each decoded field is non-trivial.
        let src: Vec<u8> = (0..T::LEN).map(|i| ((i * 7 + 1) & 0x7f) as u8).collect();
        let decoded = T::from_block(&src);
        let mut buf = vec![0u8; T::LEN];
        decoded.write_to(&mut buf);
        assert_eq!(
            T::from_block(&buf),
            decoded,
            "write_to must be the exact inverse of from_block"
        );
    }

    #[test]
    fn every_roland_block_round_trips_via_the_trait() {
        use crate::fx::{DelayParams, ExtInFx, ReverbParams};
        use crate::sys::{SysGeneral, SysMidi, SysSound};
        assert_block_roundtrip::<VoiceParams>();
        assert_block_roundtrip::<SysGeneral>();
        assert_block_roundtrip::<SysSound>();
        assert_block_roundtrip::<SysMidi>();
        assert_block_roundtrip::<ReverbParams>();
        assert_block_roundtrip::<DelayParams>();
        assert_block_roundtrip::<ExtInFx>();
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_json_round_trips_the_value_types() {
        fn rt<T>()
        where
            T: RolandBlock
                + serde::Serialize
                + serde::de::DeserializeOwned
                + PartialEq
                + std::fmt::Debug,
        {
            let src: Vec<u8> = (0..T::LEN).map(|i| ((i * 7 + 1) & 0x7f) as u8).collect();
            let v = T::from_block(&src);
            let json = serde_json::to_string(&v).unwrap();
            assert_eq!(serde_json::from_str::<T>(&json).unwrap(), v);
        }
        use crate::fx::{
            DelayParams, ExtInFx, InstFxParams, MfxParams, ReverbParams, FX_PRM_COUNT,
        };
        use crate::sys::{SysGeneral, SysMidi, SysSound};
        rt::<VoiceParams>();
        rt::<SysGeneral>();
        rt::<SysSound>();
        rt::<SysMidi>();
        rt::<ReverbParams>();
        rt::<DelayParams>();
        rt::<ExtInFx>();

        // types without RolandBlock: spot-check directly.
        let sw = StepWord {
            raw: [80, 2, 0, 0x80],
        };
        assert_eq!(
            serde_json::from_str::<StepWord>(&serde_json::to_string(&sw).unwrap()).unwrap(),
            sw
        );
        let mw = MotionWord { raw: [1, 2, 3, 4] };
        assert_eq!(
            serde_json::from_str::<MotionWord>(&serde_json::to_string(&mw).unwrap()).unwrap(),
            mw
        );
        assert_eq!(serde_json::to_string(&SubStep::Flam).unwrap(), "\"Flam\"");
        let mfx = MfxParams {
            fx_type: 3,
            switch: true,
            ctrl: 5,
            prm: [7u8; FX_PRM_COUNT],
        };
        assert_eq!(
            serde_json::from_str::<MfxParams>(&serde_json::to_string(&mfx).unwrap()).unwrap(),
            mfx
        );
        let ifx = InstFxParams {
            slot: 2,
            fx_type: 5,
            ctrl: 9,
            prm: [1u8; FX_PRM_COUNT],
            prm_available: FX_PRM_COUNT,
        };
        assert_eq!(
            serde_json::from_str::<InstFxParams>(&serde_json::to_string(&ifx).unwrap()).unwrap(),
            ifx
        );
    }

    #[test]
    fn name_field_pads_and_truncates() {
        let mut buf = vec![0xAAu8; 20];
        assert!(write_name_field(&mut buf, 2, 16, "Hi"));
        assert_eq!(&buf[2..4], b"Hi");
        assert!(buf[4..18].iter().all(|&b| b == b' '), "padded with spaces");
        assert_eq!(&buf[0..2], &[0xAA, 0xAA], "bytes before field untouched");
        assert_eq!(&buf[18..20], &[0xAA, 0xAA], "bytes after field untouched");
        // over-long name truncates to the field width.
        let mut buf = vec![0u8; 16];
        assert!(write_name_field(&mut buf, 0, 16, "0123456789ABCDEFGHIJ"));
        assert_eq!(&buf, b"0123456789ABCDEF");
        // out of range refuses.
        assert!(!write_name_field(&mut buf, 8, 16, "x"));
    }
}
