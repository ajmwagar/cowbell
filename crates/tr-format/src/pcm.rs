//! User-sample storage: the `PCMT` table + `SMPL` reserved region.
//!
//! A backup that has user samples loaded carries two extra chunks beyond the
//! sequencer data:
//!
//! - **`PCMT`** — the *PCM-tone table*: a flat array of fixed **64-byte**
//!   records, one per tone slot. Each record describes where a sample's audio
//!   lives and, crucially, the **playback window** into it. This is the backup
//!   analogue of the device's SysEx `tone.*` PCM region (`0x40`; see
//!   `docs/device-sysex.md`).
//! - **`SMPL`** — a **zero-length** chunk header whose `extra` field declares
//!   the size of the reserved user-sample region (`0x330_0000`, ≈ 51 MB on the
//!   TR-6S). The raw PCM audio blob follows the header for `extra` bytes.
//!
//!   **The region is always written out, loaded or not.** The reference backup
//!   has *no* user samples and is still 56.9 MB, because the reserved region is
//!   present and filled with `0xFF` — erased flash, not zeros. Two consequences
//!   that are easy to get wrong: file size says nothing about whether samples
//!   are loaded, and a free-space scan looking for `0x00` will report the whole
//!   51 MB as occupied.
//!
//! ## The slice model (why this matters for a breakbeat slicer)
//!
//! Each `PCMT` record separates *where the audio is* from *what part of it
//! plays*:
//!
//! | field | meaning |
//! | ----- | ------- |
//! | `Address` / `AddressRight` | byte offset of the sample's PCM (L / R) in the `SMPL` region |
//! | `Size` | the sample's stored length |
//! | `Start` / `End` | the **playback window** — the slice actually triggered |
//! | `EndMax` | the full playable length (window upper bound) |
//! | `SamplingFrequency`, `Channel`, `Gain` | rate, mono/stereo, level |
//! | `ToneId0..3` | the tone slot(s) that use this PCM record |
//!
//! Because `Start`/`End` are independent of `Address`/`Size`, **several records
//! can point at the same `Address` with different `Start`/`End`** — i.e. one
//! uploaded break can be windowed into N slices with *no audio duplication*.
//! That is exactly what the host-side slicer (cowbell-7po) needs, and it is
//! native to the format.
//!
//! ## Evidence & confidence
//!
//! The record's **field order, offsets, and widths** are transcribed from TR
//! Editor's `Script.xml` `tonePcm` struct and independently cross-validated
//! against the device address map: with the wire form's 8-byte `int8x4` digits,
//! `Channel` lands at `0x38`, exactly where `docs/device-sysex.md` documents
//! `tone.channel`. The 64-byte record size matches the observed `PCMT` stride
//! (`0x10000 / 1024`).
//!
//! What is **not** yet confirmed on real data: the numeric encoding of the
//! 4-byte `int8x4` fields is taken to be little-endian `u32` (the container's
//! convention for every other multi-byte field — chunk `size`/`extra` included),
//! and whether tone-slot index maps 1:1 onto record index or sits behind a small
//! preamble. The only backup on hand has **no user samples**, so these want a
//! sample-loaded backup to nail end-to-end. The record *layout* above does not
//! depend on that; the *values* read back from a populated table do.

use crate::{Backup, Section};

/// Size of one `PCMT` record (`tonePcm`), in bytes. Matches the observed
/// `PCMT` payload stride (`0x10000 / 1024`).
pub const PCM_TONE_ENTRY_SIZE: usize = 0x40;

/// Record index 0 of the `PCMT` table is **not a tone**.
///
/// The payload opens with the same 16-byte array header the `KIT` and `TONE`
/// sections use — `count` (`0x400`), stride (`0x40`), then an 8-byte
/// section-level token — which overlays what would otherwise be record 0's
/// `Address`/`AddressRight`/`Size`/`Start` fields. On the reference backup,
/// reading record 0 as a tone yields `address = 0x400`, i.e. the record count.
///
/// This matters more here than for `KIT`, where the same overlay is harmless
/// because a kit record's first 16 bytes are unused. For `PCMT` those bytes are
/// the four most load-bearing fields in the record.
///
/// **Unresolved:** whether real entries begin at `payload + 0x10` (the `TONE`
/// convention — see `TONE_ENTRY_BASE_IN_PAYLOAD` — giving 1023 usable records)
/// or at `payload + i * 0x40` with index 0 simply sacrificed (the `KIT`
/// convention). The reference backup has an empty table, so it cannot
/// distinguish them; a sample-loaded backup would settle it in one look.
/// Everything below is deliberately written to be correct under **both**
/// readings: index 0 is refused, and no other index moves.
pub const PCM_TONE_RESERVED_INDEX: usize = 0;

// Field byte-offsets within a 64-byte record (schema order; see module docs).
const OFF_ADDRESS: usize = 0x00;
const OFF_ADDRESS_RIGHT: usize = 0x04;
const OFF_SIZE: usize = 0x08;
const OFF_START: usize = 0x0C;
const OFF_END: usize = 0x10;
const OFF_END_MAX: usize = 0x14;
const OFF_SAMPLING_FREQUENCY: usize = 0x18;
const OFF_CHANNEL: usize = 0x1C;
const OFF_GAIN: usize = 0x1D;
// 0x1E..0x20 Reserve0 (int4x4, 2 bytes)
const OFF_TONE_ID: [usize; 4] = [0x20, 0x24, 0x28, 0x2C];
// 0x30..0x40 Reserve1_0..3 (4 × int8x4)

/// A decoded `PCMT` record — one sample's storage reference and playback
/// window. See the module docs for the slice model and the confidence notes on
/// the `int8x4` encoding.
///
/// `raw` is the full 64-byte record, retained verbatim so unknown/reserved
/// bytes survive a round-trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PcmTone {
    /// PCM byte offset of the left channel in the `SMPL` region.
    pub address: u32,
    /// PCM byte offset of the right channel (stereo samples).
    pub address_right: u32,
    /// The sample's stored length.
    pub size: u32,
    /// Playback window start (the slice's in-point).
    pub start: u32,
    /// Playback window end (the slice's out-point).
    pub end: u32,
    /// Maximum end — the full playable length.
    pub end_max: u32,
    /// Sampling frequency.
    pub sampling_frequency: u32,
    /// Channel mode (`0..=2`; mono/stereo per `Script.xml`).
    pub channel: u8,
    /// Gain (`0..=36`).
    pub gain: u8,
    /// The tone slot(s) that reference this PCM record.
    pub tone_ids: [u32; 4],
    /// The 64 record bytes exactly as stored.
    pub raw: [u8; PCM_TONE_ENTRY_SIZE],
}

fn le_u32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

impl PcmTone {
    /// Decode a 64-byte record. Returns `None` if `bytes` is too short.
    pub fn from_bytes(bytes: &[u8]) -> Option<PcmTone> {
        if bytes.len() < PCM_TONE_ENTRY_SIZE {
            return None;
        }
        let mut raw = [0u8; PCM_TONE_ENTRY_SIZE];
        raw.copy_from_slice(&bytes[..PCM_TONE_ENTRY_SIZE]);
        Some(PcmTone {
            address: le_u32(&raw, OFF_ADDRESS),
            address_right: le_u32(&raw, OFF_ADDRESS_RIGHT),
            size: le_u32(&raw, OFF_SIZE),
            start: le_u32(&raw, OFF_START),
            end: le_u32(&raw, OFF_END),
            end_max: le_u32(&raw, OFF_END_MAX),
            sampling_frequency: le_u32(&raw, OFF_SAMPLING_FREQUENCY),
            channel: raw[OFF_CHANNEL],
            gain: raw[OFF_GAIN],
            tone_ids: [
                le_u32(&raw, OFF_TONE_ID[0]),
                le_u32(&raw, OFF_TONE_ID[1]),
                le_u32(&raw, OFF_TONE_ID[2]),
                le_u32(&raw, OFF_TONE_ID[3]),
            ],
            raw,
        })
    }

    /// Whether this record carries a sample (non-zero `EndMax` or `Size`). An
    /// all-zero record is an empty slot.
    ///
    /// Note this is a property of the decoded *bytes*: it says nothing about
    /// whether the index those bytes came from is a real tone slot. Record 0 is
    /// the section's array header and always looks "populated" — use
    /// [`Backup::pcm_tone`], which refuses that index, rather than decoding raw
    /// bytes and asking this.
    pub fn is_populated(&self) -> bool {
        self.size != 0 || self.end_max != 0 || self.address != 0
    }
}

impl Section {
    /// Number of `PCMT` records this section holds (`payload_len / 64`). Zero
    /// for a non-`PCMT` section.
    pub fn pcm_tone_count(&self) -> usize {
        if self.tag != *b"PCMT" {
            return 0;
        }
        self.payload_len / PCM_TONE_ENTRY_SIZE
    }

    /// Indices that address a real tone record — `1..pcm_tone_count()`.
    ///
    /// Index 0 is the section's array header, not a tone; see
    /// [`PCM_TONE_RESERVED_INDEX`].
    pub fn pcm_tone_indices(&self) -> std::ops::Range<usize> {
        let n = self.pcm_tone_count();
        if n == 0 {
            0..0
        } else {
            (PCM_TONE_RESERVED_INDEX + 1)..n
        }
    }
}

impl Backup {
    /// Decode `PCMT` record `i`, or `None` if there is no `PCMT` section or `i`
    /// is out of range.
    /// Index 0 is the section's array header rather than a tone, so it is
    /// refused; see [`PCM_TONE_RESERVED_INDEX`].
    pub fn pcm_tone(&self, i: usize) -> Option<PcmTone> {
        let sec = self.find("PCMT")?;
        if i <= PCM_TONE_RESERVED_INDEX || i >= sec.pcm_tone_count() {
            return None;
        }
        let o = sec.payload_offset + i * PCM_TONE_ENTRY_SIZE;
        PcmTone::from_bytes(&self.raw()[o..o + PCM_TONE_ENTRY_SIZE])
    }

    /// All `PCMT` records (empty if there is no `PCMT` section).
    pub fn pcm_tones(&self) -> Vec<PcmTone> {
        match self.find("PCMT") {
            Some(sec) => sec
                .pcm_tone_indices()
                .filter_map(|i| self.pcm_tone(i))
                .collect(),
            None => Vec::new(),
        }
    }

    /// Byte offset of `PCMT` record `i` within the file, if it exists.
    fn pcm_tone_offset(&self, i: usize) -> Option<usize> {
        let sec = self.find("PCMT")?;
        (i > PCM_TONE_RESERVED_INDEX && i < sec.pcm_tone_count())
            .then(|| sec.payload_offset + i * PCM_TONE_ENTRY_SIZE)
    }

    /// Rewrite record `i`'s playback window (`Start`/`End`) in place — the core
    /// slice edit. Length-preserving: only the two 4-byte fields change, every
    /// other byte of the record (and file) is untouched. Returns `false` if the
    /// record is out of range.
    pub fn set_pcm_tone_window(&mut self, i: usize, start: u32, end: u32) -> bool {
        let Some(o) = self.pcm_tone_offset(i) else {
            return false;
        };
        let raw = self.raw_mut();
        raw[o + OFF_START..o + OFF_START + 4].copy_from_slice(&start.to_le_bytes());
        raw[o + OFF_END..o + OFF_END + 4].copy_from_slice(&end.to_le_bytes());
        true
    }

    /// Point record `i` at a sample's PCM (`Address`/`AddressRight`) — used when
    /// several slice records share one uploaded sample. Length-preserving.
    /// Returns `false` if the record is out of range.
    pub fn set_pcm_tone_address(&mut self, i: usize, address: u32, address_right: u32) -> bool {
        let Some(o) = self.pcm_tone_offset(i) else {
            return false;
        };
        let raw = self.raw_mut();
        raw[o + OFF_ADDRESS..o + OFF_ADDRESS + 4].copy_from_slice(&address.to_le_bytes());
        raw[o + OFF_ADDRESS_RIGHT..o + OFF_ADDRESS_RIGHT + 4]
            .copy_from_slice(&address_right.to_le_bytes());
        true
    }
}

/// The `SMPL` chunk's declared reserved-region size, in bytes — the amount of
/// PCM storage that follows the `SMPL` header. `None` if there is no `SMPL`
/// section.
impl Backup {
    pub fn sample_region_size(&self) -> Option<u32> {
        self.find("SMPL").map(|s| s.extra)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CHUNK_HEADER_LEN, HEADER_LEN, MAGIC_TR6S};

    /// Build a synthetic backup: header + a `PCMT` chunk with `n` records, plus
    /// a zero-length `SMPL` header declaring a reserved region. No Roland bytes.
    fn synthetic_with_pcmt(records: &[[u8; PCM_TONE_ENTRY_SIZE]], sample_region: u32) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(MAGIC_TR6S);
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&5u32.to_le_bytes()); // version
        v.resize(HEADER_LEN, 0);
        // PCMT chunk
        let payload_len = (records.len() * PCM_TONE_ENTRY_SIZE) as u32;
        v.extend_from_slice(b"PCMT");
        v.extend_from_slice(&0u32.to_le_bytes()); // reserved == 0
        v.extend_from_slice(&payload_len.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes()); // extra
        for r in records {
            v.extend_from_slice(r);
        }
        // SMPL chunk: zero-length header, extra = reserved region size
        v.extend_from_slice(b"SMPL");
        v.extend_from_slice(&0u32.to_le_bytes()); // reserved == 0
        v.extend_from_slice(&0u32.to_le_bytes()); // payload_len == 0
        v.extend_from_slice(&sample_region.to_le_bytes()); // extra
        let _ = CHUNK_HEADER_LEN; // (documents the 16-byte header we just wrote)
        v
    }

    fn record(address: u32, size: u32, start: u32, end: u32) -> [u8; PCM_TONE_ENTRY_SIZE] {
        let mut r = [0u8; PCM_TONE_ENTRY_SIZE];
        r[OFF_ADDRESS..OFF_ADDRESS + 4].copy_from_slice(&address.to_le_bytes());
        r[OFF_SIZE..OFF_SIZE + 4].copy_from_slice(&size.to_le_bytes());
        r[OFF_START..OFF_START + 4].copy_from_slice(&start.to_le_bytes());
        r[OFF_END..OFF_END + 4].copy_from_slice(&end.to_le_bytes());
        r[OFF_END_MAX..OFF_END_MAX + 4].copy_from_slice(&size.to_le_bytes());
        r[OFF_CHANNEL] = 1;
        r[OFF_TONE_ID[0]..OFF_TONE_ID[0] + 4].copy_from_slice(&624u32.to_le_bytes());
        r
    }

    /// The reference backup, if the working copy has one. Not committed (no
    /// Roland data in the repo), so this skips when absent.
    fn reference_backup() -> Option<Backup> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../roland_backup_dump/TR-6S/BACKUP/tr6s_bak.bin"
        );
        std::path::Path::new(path)
            .exists()
            .then(|| Backup::parse(std::fs::read(path).unwrap()).unwrap())
    }

    /// The bug this guard exists for, on real data.
    ///
    /// The `PCMT` payload opens with the section's array header — count `0x400`,
    /// stride `0x40`, then a token — which decodes as a tone with
    /// `address = 0x400`. Before the reserved-index guard, `is_populated()`
    /// returned true for it on every real backup, so "how many samples are
    /// loaded" answered 1 on a device with none.
    #[test]
    fn the_array_header_is_not_reported_as_a_loaded_sample() {
        let Some(b) = reference_backup() else { return };
        let sec = b.find("PCMT").expect("reference backup has a PCMT chunk");

        // The header bytes really do look like a populated tone...
        let header = PcmTone::from_bytes(
            &b.raw()[sec.payload_offset..sec.payload_offset + PCM_TONE_ENTRY_SIZE],
        )
        .unwrap();
        assert!(
            header.is_populated(),
            "precondition: the raw header bytes look populated"
        );
        assert_eq!(header.address, 0x400, "that 'address' is the record count");

        // ...but it is not reachable as a tone, and does not inflate the count.
        assert!(b.pcm_tone(0).is_none());
        assert_eq!(
            b.pcm_tones().iter().filter(|t| t.is_populated()).count(),
            0,
            "this backup has no user samples loaded"
        );
    }

    /// Neither setter may touch record 0: its `+0x08` field carries a
    /// section-level token whose algorithm is unresolved, and writing there
    /// would corrupt it.
    #[test]
    fn setters_refuse_the_reserved_index_on_the_real_backup() {
        let Some(b) = reference_backup() else { return };
        let before = b.to_bytes();
        let mut b = b;

        assert!(!b.set_pcm_tone_window(0, 1, 2));
        assert!(!b.set_pcm_tone_address(0, 1, 2));
        assert_eq!(b.to_bytes(), before, "a refused write must change nothing");
    }

    #[test]
    fn decodes_records_and_slice_window() {
        // Record 0 stands in for the section's array header — it is never a
        // tone on a real backup, so the fixture must not put one there either.
        let recs = [
            record(0, 0, 0, 0),
            record(0x1000, 0x8000, 0, 0x8000),
            record(0, 0, 0, 0),
        ];
        let raw = synthetic_with_pcmt(&recs, 0x330_0000);
        let b = Backup::parse(raw).unwrap();

        assert!(b.find("PCMT").is_some());
        assert_eq!(b.find("PCMT").unwrap().pcm_tone_count(), 3);
        assert_eq!(b.sample_region_size(), Some(0x330_0000));

        // The reserved index is refused rather than decoded as a tone.
        assert!(b.pcm_tone(0).is_none(), "record 0 is the array header");

        let t = b.pcm_tone(1).unwrap();
        assert_eq!(t.address, 0x1000);
        assert_eq!(t.size, 0x8000);
        assert_eq!(t.start, 0);
        assert_eq!(t.end, 0x8000);
        assert_eq!(t.end_max, 0x8000);
        assert_eq!(t.channel, 1);
        assert_eq!(t.tone_ids[0], 624);
        assert!(t.is_populated());
        assert!(!b.pcm_tone(2).unwrap().is_populated());
        assert!(b.pcm_tone(3).is_none());

        // Enumeration skips the header, so it never reports a phantom sample.
        assert_eq!(b.pcm_tones().len(), 2);
        assert_eq!(b.find("PCMT").unwrap().pcm_tone_indices(), 1..3);
    }

    #[test]
    fn window_edit_is_length_preserving_and_local() {
        let recs = [
            record(0, 0, 0, 0), // array header slot
            record(0x1000, 0x8000, 0, 0x8000),
            record(0x1000, 0x8000, 0, 0x8000),
        ];
        let raw = synthetic_with_pcmt(&recs, 0x330_0000);
        let before = raw.clone();
        let mut b = Backup::parse(raw).unwrap();

        // Two slices of ONE sample: same Address, different windows.
        assert!(b.set_pcm_tone_window(1, 0x100, 0x2000));
        assert!(b.set_pcm_tone_window(2, 0x2000, 0x4000));
        assert!(!b.set_pcm_tone_window(3, 0, 1)); // out of range
        // Writing the reserved index would clobber the section token, whose
        // algorithm is unresolved — refuse rather than corrupt it.
        assert!(!b.set_pcm_tone_window(0, 0, 1));
        assert!(!b.set_pcm_tone_address(0, 1, 1));

        let after = b.to_bytes();
        assert_eq!(after.len(), before.len(), "edit must be length-preserving");

        let (s0, s1) = (b.pcm_tone(1).unwrap(), b.pcm_tone(2).unwrap());
        assert_eq!((s0.start, s0.end), (0x100, 0x2000));
        assert_eq!((s1.start, s1.end), (0x2000, 0x4000));
        // Shared sample: both windows into the same PCM address.
        assert_eq!(s0.address, s1.address);

        // Every byte that changed must lie inside a Start/End field of the two
        // records we edited — nothing else in the file moved.
        let pcmt = b.find("PCMT").unwrap().payload_offset;
        let allowed: Vec<std::ops::Range<usize>> = (1..3)
            .flat_map(|i| {
                let base = pcmt + i * PCM_TONE_ENTRY_SIZE;
                [
                    base + OFF_START..base + OFF_START + 4,
                    base + OFF_END..base + OFF_END + 4,
                ]
            })
            .collect();
        for (idx, (x, y)) in before.iter().zip(after.iter()).enumerate() {
            if x != y {
                assert!(
                    allowed.iter().any(|r| r.contains(&idx)),
                    "byte {idx:#x} changed outside a Start/End field"
                );
            }
        }
    }

    #[test]
    fn no_pcmt_section_is_graceful() {
        let recs: [[u8; PCM_TONE_ENTRY_SIZE]; 0] = [];
        let raw = synthetic_with_pcmt(&recs, 0);
        // PCMT with zero records still parses; pcm_tones is empty.
        let b = Backup::parse(raw).unwrap();
        assert!(b.pcm_tones().is_empty());
        assert_eq!(b.pcm_tone(0), None);
    }
}
