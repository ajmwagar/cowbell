//! The TR-6S / TR-8S device address map — typed constants and accessors.
//!
//! Every value here is transcribed from the address table in
//! `docs/device-sysex.md` (Roland's parameter-address model for the edit
//! buffer, `temp`). Addresses are the **4-byte, 7-bit-safe** form the protocol
//! puts on the wire.
//!
//! ## The 7-bit address arithmetic (inferred)
//!
//! Roland device addresses are "7-bit-safe": every one of the four bytes is
//! `0x00..=0x7F`. The doc states this and gives array `step`/`block` strides,
//! but does **not** spell out the arithmetic used to walk them. We model an
//! address as a **base-128 big-endian integer** — i.e. each byte is a base-128
//! digit — because that is the only reading under which the documented strides
//! stay 7-bit-safe across their full `count`. Worked example from the doc:
//! `sys.categoryName` has step `0x10`, count 32. Element 8 sits at offset
//! `8 * 0x10 = 0x80`; in plain base-256 that would need a `0x80` byte, which is
//! **not** 7-bit-safe. In base-128 it carries into the next digit
//! (`00 01 00 00`), which is. So [`RolandAddress::offset`] does base-128 add.
//!
//! The base-128 model is now **confirmed by a real RQ1/DT1 capture** (the
//! compuphonic "send pattern/kit" transfer): kit 126 ("kit 127" 1-indexed) is
//! written at `10 7e 00 00` — i.e. the 0-indexed kit number lands directly in
//! address byte 1, and its 11 instrument records step byte 2 by 1 each
//! (`10 7e 10 00 … 10 7e 1a 00`). That capture also **corrected** the per-record
//! strides originally inferred from the doc's byte-pattern literals: the real
//! deltas are the *base-128 value* of one address digit, not the doc's literal
//! (kit block `0x4000` not `0x10000`; kit instrument step `0x80` not `0x100`;
//! pattern block `0x40000` not `0x100000`). Constants confirmed this way are
//! marked "CAPTURE-CONFIRMED" below; those still resting on the doc alone (tone
//! block, tone-PCM offsets) are marked "inferred". Scalar base addresses (e.g.
//! `20 00 00 00`) are transcribed verbatim.

/// A 4-byte Roland device address (each byte 7-bit-safe, `0x00..=0x7F`).
///
/// Interpreted as a base-128 big-endian integer for arithmetic; see the module
/// docs. The raw bytes are exactly what goes on the wire between the command
/// byte and the data/checksum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RolandAddress([u8; 4]);

impl RolandAddress {
    /// Construct from four raw wire bytes (as printed in the doc's address
    /// table). Bytes are stored verbatim; see [`RolandAddress::is_7bit_safe`].
    pub const fn new(bytes: [u8; 4]) -> Self {
        RolandAddress(bytes)
    }

    /// The four raw bytes, big-endian, as they appear on the wire.
    pub const fn bytes(self) -> [u8; 4] {
        self.0
    }

    /// The base-128 integer value of the address (each byte a base-128 digit).
    /// See the module docs on why this is base-128 and not base-256.
    pub const fn to_value(self) -> u32 {
        ((self.0[0] as u32) << 21)
            | ((self.0[1] as u32) << 14)
            | ((self.0[2] as u32) << 7)
            | (self.0[3] as u32)
    }

    /// Build an address from a base-128 integer value. The result is always
    /// 7-bit-safe (each digit is masked to 7 bits).
    pub const fn from_value(v: u32) -> Self {
        RolandAddress([
            ((v >> 21) & 0x7F) as u8,
            ((v >> 14) & 0x7F) as u8,
            ((v >> 7) & 0x7F) as u8,
            (v & 0x7F) as u8,
        ])
    }

    /// Add a base-128 delta (e.g. an array `step` or per-slot `block` stride
    /// from the doc), carrying between 7-bit digits. Wraps on overflow of the
    /// 28-bit space, which no real TR address reaches.
    pub fn offset(self, delta: u32) -> Self {
        RolandAddress::from_value(self.to_value().wrapping_add(delta))
    }

    /// Whether every byte is within 7 bits, as the protocol requires. A
    /// [`RolandAddress`] produced by [`from_value`](Self::from_value) or
    /// [`offset`](Self::offset) is always safe; one built with
    /// [`new`](Self::new) from arbitrary bytes might not be.
    pub const fn is_7bit_safe(self) -> bool {
        self.0[0] < 0x80 && self.0[1] < 0x80 && self.0[2] < 0x80 && self.0[3] < 0x80
    }
}

/// A field that repeats at a fixed stride across kits / patterns / tones /
/// slots. Mirrors the doc's `step × count` / `block` columns.
///
/// `step` is the base-128 delta between consecutive elements (see module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArrayField {
    /// Address of element 0.
    pub base: RolandAddress,
    /// Bytes per element, as documented (0 where the doc does not state a size).
    pub size: u32,
    /// Base-128 stride between consecutive elements.
    pub step: u32,
    /// Number of elements.
    pub count: u32,
}

impl ArrayField {
    /// Address of element `i`. Returns `None` if `i >= count`.
    pub fn nth(&self, i: u32) -> Option<RolandAddress> {
        if i >= self.count {
            return None;
        }
        Some(self.base.offset(self.step.wrapping_mul(i)))
    }
}

// ---------------------------------------------------------------------------
// Address-region prefixes (the high byte), from the doc.
// ---------------------------------------------------------------------------

/// `0x01` — step/sequencer + system region.
pub const REGION_STEP_SYSTEM: u8 = 0x01;
/// `0x10` — kit region.
pub const REGION_KIT: u8 = 0x10;
/// `0x20` — pattern region.
pub const REGION_PATTERN: u8 = 0x20;
/// `0x30` — tone metadata region.
pub const REGION_TONE_META: u8 = 0x30;
/// `0x40` — tone PCM region (sample references).
pub const REGION_TONE_PCM: u8 = 0x40;
/// `0x50` — utility region.
pub const REGION_UTILITY: u8 = 0x50;

// ---------------------------------------------------------------------------
// System (`sys.*`) — region 0x00/0x01
// ---------------------------------------------------------------------------

/// `sys.categoryName`: 32 user-category names, 16 bytes each, step `0x10`.
/// Cross-validates with the backup's `SYS ` `sysCategory` block (32×16).
pub const SYS_CATEGORY_NAME: ArrayField = ArrayField {
    base: RolandAddress::new([0x00, 0x01, 0x00, 0x00]),
    size: 16,
    step: 0x10,
    count: 32,
};

// ---------------------------------------------------------------------------
// Step / sequencer (`stp.*`) — region 0x01
// ---------------------------------------------------------------------------

/// `stp.currentKit` (1 byte).
pub const STP_CURRENT_KIT: RolandAddress = RolandAddress::new([0x01, 0x00, 0x00, 0x00]);
/// `stp.currentPattern` (1 byte).
pub const STP_CURRENT_PATTERN: RolandAddress = RolandAddress::new([0x01, 0x00, 0x00, 0x01]);
/// `stp.nextPattern` (1 byte).
pub const STP_NEXT_PATTERN: RolandAddress = RolandAddress::new([0x01, 0x00, 0x00, 0x02]);
/// `stp.patternSelect` (4 bytes).
pub const STP_PATTERN_SELECT: RolandAddress = RolandAddress::new([0x01, 0x00, 0x00, 0x1B]);

// ---------------------------------------------------------------------------
// Kit (`kit.*`) — region 0x10, per-kit block stride 0x4000 (CAPTURE-CONFIRMED)
// ---------------------------------------------------------------------------

/// Per-kit block stride (base-128 value delta). **CAPTURE-CONFIRMED**: kit 126
/// is addressed at `10 7e 00 00`, so the 0-indexed kit number lands directly in
/// address byte 1 (`0x4000` = one unit of that digit). The doc's `0x10000`
/// literal was a byte-pattern, not the value delta, and is 4× too large.
pub const KIT_BLOCK: u32 = 0x4000;
/// Number of kits (IDs 0–127).
pub const KIT_COUNT: u32 = 128;

/// `kit.name` of kit 0 (16 bytes). Use [`kit_name`] for an arbitrary kit.
pub const KIT_NAME: RolandAddress = RolandAddress::new([0x10, 0x00, 0x00, 0x00]);

/// A kit's per-instrument record: **16 bytes each, 11 instruments**, step `0x80`
/// (address byte 2 += 1 per instrument). **CAPTURE-CONFIRMED** — kit 126's
/// instrument blocks land at `10 7e 10 00 … 10 7e 1a 00`, each a 16-byte DT1.
/// The first two bytes are the [`KIT_TONE_ID`]; the remaining 14 are the device
/// encoding of the per-voice parameters (field layout not yet decoded — the
/// backup's `VoiceParams` is the decoded reference for the same knobs).
/// Count 11 = the TR-8S instrument count; a TR-6S populates 6.
pub const KIT_INSTRUMENT: ArrayField = ArrayField {
    base: RolandAddress::new([0x10, 0x00, 0x10, 0x00]),
    size: 16,
    step: 0x80,
    count: 11,
};

/// `kit.toneId` of kit 0: the first 2 bytes (base-128 `u16`) of each 16-byte
/// [`KIT_INSTRUMENT`] record, step `0x80`. **CAPTURE-CONFIRMED.**
/// Count 11 = the TR-8S instrument count; a TR-6S populates 6.
pub const KIT_TONE_ID: ArrayField = ArrayField {
    base: RolandAddress::new([0x10, 0x00, 0x10, 0x00]),
    size: 2,
    step: 0x80,
    count: 11,
};

/// `kit.name` address of kit `i` (`0..KIT_COUNT`).
pub fn kit_name(i: u32) -> Option<RolandAddress> {
    (i < KIT_COUNT).then(|| KIT_NAME.offset(KIT_BLOCK.wrapping_mul(i)))
}

/// `kit.toneId` field for kit `i` (`0..KIT_COUNT`), already shifted by the
/// per-kit block. Index its elements with [`ArrayField::nth`].
pub fn kit_tone_ids(i: u32) -> Option<ArrayField> {
    (i < KIT_COUNT).then(|| ArrayField {
        base: KIT_TONE_ID.base.offset(KIT_BLOCK.wrapping_mul(i)),
        ..KIT_TONE_ID
    })
}

/// The 11 × 16-byte [`KIT_INSTRUMENT`] records for kit `i` (`0..KIT_COUNT`),
/// already shifted by the per-kit block. Index them with [`ArrayField::nth`].
pub fn kit_instruments(i: u32) -> Option<ArrayField> {
    (i < KIT_COUNT).then(|| ArrayField {
        base: KIT_INSTRUMENT.base.offset(KIT_BLOCK.wrapping_mul(i)),
        ..KIT_INSTRUMENT
    })
}

// ---------------------------------------------------------------------------
// Pattern (`ptn.*`) — region 0x20, per-pattern block stride 0x40000 (CAPTURE-CONFIRMED)
// ---------------------------------------------------------------------------

/// Per-pattern block stride (base-128 value delta). **CAPTURE-CONFIRMED**:
/// consecutive patterns step address byte 1 by `0x10` (`20 00 00 00` →
/// `20 10 00 00`), i.e. 8 patterns per region byte, rolling `0x20 → 0x21 → …`.
/// `0x40000` is that `0x10`-in-byte-1 delta; the doc's `0x100000` literal was 4×
/// too large.
pub const PATTERN_BLOCK: u32 = 0x40000;
/// Number of patterns (IDs 0–127).
pub const PATTERN_COUNT: u32 = 128;

/// `ptn.name` of pattern 0 (16 bytes). Use [`pattern_name`] for an arbitrary
/// pattern.
pub const PATTERN_NAME: RolandAddress = RolandAddress::new([0x20, 0x00, 0x00, 0x00]);
/// `ptn.kitReference` of pattern 0 (u16). Our backup stores the referenced kit
/// in one byte; the wider device field is consistent.
pub const PATTERN_KIT_REFERENCE: RolandAddress = RolandAddress::new([0x20, 0x00, 0x00, 0x14]);
/// `ptn.kitReferenceSw` of pattern 0 (1 byte).
pub const PATTERN_KIT_REFERENCE_SW: RolandAddress = RolandAddress::new([0x20, 0x00, 0x01, 0x06]);

/// `ptn.name` address of pattern `i` (`0..PATTERN_COUNT`).
pub fn pattern_name(i: u32) -> Option<RolandAddress> {
    (i < PATTERN_COUNT).then(|| PATTERN_NAME.offset(PATTERN_BLOCK.wrapping_mul(i)))
}

/// `ptn.kitReference` address of pattern `i` (`0..PATTERN_COUNT`).
pub fn pattern_kit_reference(i: u32) -> Option<RolandAddress> {
    (i < PATTERN_COUNT).then(|| PATTERN_KIT_REFERENCE.offset(PATTERN_BLOCK.wrapping_mul(i)))
}

// ---------------------------------------------------------------------------
// Tone metadata (`tone.*`) — region 0x30, per-tone block stride 0x10000
// ---------------------------------------------------------------------------

/// Per-tone block stride (base-128 value delta) — the doc's `block 0x10000/tone`.
pub const TONE_BLOCK: u32 = 0x10000;

/// `tone.name` of tone 0 (16 bytes).
pub const TONE_NAME: RolandAddress = RolandAddress::new([0x30, 0x00, 0x00, 0x00]);
/// `tone.category` of tone 0 (1 byte).
pub const TONE_CATEGORY: RolandAddress = RolandAddress::new([0x30, 0x00, 0x00, 0x10]);
/// `tone.type` of tone 0 (1 byte).
pub const TONE_TYPE: RolandAddress = RolandAddress::new([0x30, 0x00, 0x00, 0x11]);

/// `tone.name` address of tone `i`. There is no documented tone count here;
/// see [`USER_TONE_ID_MIN`] for the preset/user boundary.
pub fn tone_name(i: u32) -> RolandAddress {
    TONE_NAME.offset(TONE_BLOCK.wrapping_mul(i))
}

// ---------------------------------------------------------------------------
// Tone PCM / sample references (`tone.*`) — region 0x40
// ---------------------------------------------------------------------------

/// `tone.address`: sample PCM start, left (8 bytes).
pub const TONE_PCM_ADDRESS_L: RolandAddress = RolandAddress::new([0x40, 0x00, 0x00, 0x00]);
/// `tone.addressRight`: sample PCM start, right (8 bytes).
pub const TONE_PCM_ADDRESS_R: RolandAddress = RolandAddress::new([0x40, 0x00, 0x00, 0x08]);
/// `tone.size`: sample length (8 bytes).
pub const TONE_PCM_SIZE: RolandAddress = RolandAddress::new([0x40, 0x00, 0x00, 0x10]);
/// `tone.channel`: mono/stereo (1 byte).
pub const TONE_PCM_CHANNEL: RolandAddress = RolandAddress::new([0x40, 0x00, 0x00, 0x38]);

// ---------------------------------------------------------------------------
// Utility — region 0x50
// ---------------------------------------------------------------------------

/// `utility` base. The doc gives no field breakdown yet.
pub const UTILITY: RolandAddress = RolandAddress::new([0x50, 0x00, 0x00, 0x00]);

// ---------------------------------------------------------------------------
// Device constants (from the same ARIA config; see the doc's constants table).
// ---------------------------------------------------------------------------

/// Preset tones occupy IDs `0..=623`; **user** tones start at 624.
pub const USER_TONE_ID_MIN: u16 = 624;
/// Highest user tone ID (`624..=1023` user).
pub const USER_TONE_ID_MAX: u16 = 1023;
/// Sample sector stride: `0x20000` (128 KB) each.
pub const SAMPLE_SECTOR_SIZE: u32 = 0x20000;
/// First / last sample sector index (104–511).
pub const SAMPLE_SECTOR_MIN: u32 = 104;
/// Last sample sector index (inclusive).
pub const SAMPLE_SECTOR_MAX: u32 = 511;
/// Low bound of the mapped sample-RAM window.
pub const SAMPLE_ADDRESS_MIN: u32 = 0xD0_0000;
/// High bound of the mapped sample-RAM window.
pub const SAMPLE_ADDRESS_MAX: u32 = 0x3FF_FFFF;
/// User-sample storage size, `0x3300000` (≈51 MB). Cross-validates with the
/// backup `SMPL` chunk's reserved region.
pub const STORAGE_SIZE: u32 = 0x330_0000;

// ---------------------------------------------------------------------------
// Editor-model (Script.xml / backup-file) section bases — a DIFFERENT address
// space from the device-SysEx addresses above. Provided for cross-reference.
// ---------------------------------------------------------------------------

/// **Editor-model** section base addresses, as TR Editor's `Script.xml` and the
/// backup-file format use them (`tr-format`'s `Script.xml` oracle;
/// `docs/tr-format.md`): `kit = 03 00 00 00`, `ptn = 04 00 00 00`.
///
/// **These are NOT the device's RQ1/DT1 addresses.** They are the editor's
/// internal / backup-file coordinate system. The actual device SysEx capture
/// (`docs/device-sysex.md`) puts the same sections at *different* region bytes —
/// kit at `0x10`, pattern at `0x20`, tone at `0x30` (see [`KIT_NAME`],
/// [`PATTERN_NAME`], [`TONE_NAME`]). Because only the capture is attested wire
/// traffic, the device-SysEx constants above are what you send; these
/// editor-model bases are exposed **only** to make that discrepancy explicit and
/// to cross-reference the backup format. Do not send them in an RQ1 expecting a
/// device reply until a capture confirms the device honours this coordinate
/// system (it appears not to).
pub mod editor_model {
    use super::RolandAddress;

    /// `Script.xml` kit section base (`03 00 00 00`) — editor/backup model.
    pub const KIT: RolandAddress = RolandAddress::new([0x03, 0x00, 0x00, 0x00]);
    /// `Script.xml` pattern section base (`04 00 00 00`) — editor/backup model.
    pub const PATTERN: RolandAddress = RolandAddress::new([0x04, 0x00, 0x00, 0x00]);
}
