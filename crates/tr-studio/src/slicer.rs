//! Breakbeat slicer — the pre-sliced path.
//!
//! Turns one imported break into a playable TR kit + pattern by doing the
//! sampler's slicing job *externally* (the box has no native auto-slicer):
//!
//! 1. [`Wav::read`] a break, [`slice_grid`] it into N sample-accurate windows;
//! 2. [`export_slices`] writes one WAV per slice — the user drops these onto the
//!    SD card and imports them to the tone slots named in the spec (the
//!    "pre-sliced" path: each slice is a normal user tone);
//! 3. [`apply_breakbeat`] wires those tone slots across a kit's voices and lays
//!    down a starter "reconstruct" pattern you then flip.
//!
//! The TR-6S plays **six voices**, so at most six distinct slices map to a kit
//! ([`MAX_SLICES`]); a stripped kick/snare/hat/ghost chop fits comfortably.
//! Slices beyond six are still produced as WAVs but are not auto-sequenced.
//!
//! The shared-PCM upgrade (one uploaded break, N windows, no per-slice WAV
//! export or duplication) is [`cowbell-7po.3`], built on `tr_format::pcm`.
//!
//! Pure host-side computation on plaintext audio — no firmware, no device.

use anyhow::{ensure, Result};
use tr_format::PATTERN_STEPS_PER_TRACK;

use crate::{Project, VOICE_TRACKS};

/// The most slices a single kit can auto-sequence — one per audible voice.
pub const MAX_SLICES: usize = VOICE_TRACKS; // 6

// ---------------------------------------------------------------------------
// Minimal canonical-PCM WAV I/O (16-bit; mono/stereo). No external crate.
// ---------------------------------------------------------------------------

/// A decoded PCM WAV: interleaved 16-bit samples plus its rate and channel
/// count. Only canonical 16-bit PCM is handled — enough for drum-break imports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wav {
    /// Sample rate (Hz).
    pub sample_rate: u32,
    /// Channel count (1 = mono, 2 = stereo).
    pub channels: u16,
    /// Interleaved 16-bit samples (`frames * channels` long).
    pub samples: Vec<i16>,
}

fn u32_le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u16_le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(b[o..o + 2].try_into().unwrap())
}

impl Wav {
    /// Number of frames (samples per channel).
    pub fn frames(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.samples.len() / self.channels as usize
        }
    }

    /// Parse a canonical 16-bit PCM WAV. Errors on non-PCM / non-16-bit or a
    /// malformed container.
    pub fn read(bytes: &[u8]) -> Result<Wav> {
        ensure!(
            bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE",
            "not a RIFF/WAVE file"
        );
        // (channels, sample_rate, bits) from `fmt `, and the `data` body.
        let mut fmt: Option<(u16, u32, u16)> = None;
        let mut data: Option<&[u8]> = None;
        let mut o = 12;
        // Walk the chunk list, picking out `fmt ` and `data`; skip the rest.
        while o + 8 <= bytes.len() {
            let id = &bytes[o..o + 4];
            let sz = u32_le(bytes, o + 4) as usize;
            let body_start = o + 8;
            let body_end = body_start.saturating_add(sz).min(bytes.len());
            let body = &bytes[body_start..body_end];
            match id {
                b"fmt " => {
                    ensure!(body.len() >= 16, "fmt chunk too short");
                    let format = u16_le(body, 0);
                    let channels = u16_le(body, 2);
                    let rate = u32_le(body, 4);
                    let bits = u16_le(body, 14);
                    ensure!(
                        format == 1,
                        "only PCM (format 1) is supported, got {format}"
                    );
                    ensure!(bits == 16, "only 16-bit PCM is supported, got {bits}-bit");
                    ensure!(
                        channels == 1 || channels == 2,
                        "unsupported channels {channels}"
                    );
                    fmt = Some((channels, rate, bits));
                }
                b"data" => data = Some(body),
                _ => {}
            }
            // Chunks are word-aligned: pad to even length.
            o = body_start + sz + (sz & 1);
        }
        let (channels, sample_rate, _bits) = fmt.ok_or_else(|| anyhow::anyhow!("no fmt chunk"))?;
        let data = data.ok_or_else(|| anyhow::anyhow!("no data chunk"))?;
        let samples = data
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect();
        Ok(Wav {
            sample_rate,
            channels,
            samples,
        })
    }

    /// Serialize interleaved samples to a canonical 16-bit PCM WAV.
    pub fn write(sample_rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
        let block_align = channels * 2;
        let byte_rate = sample_rate * block_align as u32;
        let data_len = (samples.len() * 2) as u32;
        let mut v = Vec::with_capacity(44 + samples.len() * 2);
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + data_len).to_le_bytes());
        v.extend_from_slice(b"WAVE");
        v.extend_from_slice(b"fmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes()); // PCM
        v.extend_from_slice(&channels.to_le_bytes());
        v.extend_from_slice(&sample_rate.to_le_bytes());
        v.extend_from_slice(&byte_rate.to_le_bytes());
        v.extend_from_slice(&block_align.to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes()); // bits
        v.extend_from_slice(b"data");
        v.extend_from_slice(&data_len.to_le_bytes());
        for s in samples {
            v.extend_from_slice(&s.to_le_bytes());
        }
        v
    }

    /// A frame window `[start, end)` as its own WAV (same rate/channels).
    fn window_wav(&self, start_frame: usize, end_frame: usize) -> Vec<u8> {
        let ch = self.channels as usize;
        let a = start_frame * ch;
        let b = (end_frame * ch).min(self.samples.len());
        Wav::write(self.sample_rate, self.channels, &self.samples[a..b.max(a)])
    }
}

// ---------------------------------------------------------------------------
// Slicing
// ---------------------------------------------------------------------------

/// One slice: its index and the frame window `[start_frame, end_frame)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slice {
    /// 0-based slice index.
    pub index: usize,
    /// First frame (inclusive).
    pub start_frame: usize,
    /// Last frame (exclusive).
    pub end_frame: usize,
}

impl Slice {
    /// Length of the slice in frames.
    pub fn len_frames(&self) -> usize {
        self.end_frame.saturating_sub(self.start_frame)
    }
}

/// The result of slicing a break: the source's rate/channels and the windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlicePlan {
    /// Source sample rate.
    pub sample_rate: u32,
    /// Source channel count.
    pub channels: u16,
    /// Total source frames.
    pub total_frames: usize,
    /// The slice windows, in order.
    pub slices: Vec<Slice>,
}

/// Divide a break into `n` equal, sample-accurate slices (the remainder frames
/// go to the last slice). This is the deterministic grid mode — a musical bar
/// chopped into `n` even steps. `n` must be `>= 1` and `<= total frames`.
pub fn slice_grid(wav: &Wav, n: usize) -> Result<SlicePlan> {
    ensure!(n >= 1, "need at least one slice");
    let total = wav.frames();
    ensure!(
        total >= n,
        "break has {total} frames, cannot make {n} slices"
    );
    let base = total / n;
    let slices = (0..n)
        .map(|i| {
            let start = i * base;
            // Last slice absorbs the remainder so the whole break is covered.
            let end = if i + 1 == n { total } else { (i + 1) * base };
            Slice {
                index: i,
                start_frame: start,
                end_frame: end,
            }
        })
        .collect();
    Ok(SlicePlan {
        sample_rate: wav.sample_rate,
        channels: wav.channels,
        total_frames: total,
        slices,
    })
}

/// An exported slice: its filename and WAV bytes.
pub type ExportedSlice = (String, Vec<u8>);

/// Render each slice to its own WAV: `(filename, bytes)`, named
/// `slice_00.wav`, `slice_01.wav`, … These are the files the user imports to
/// tone slots (the pre-sliced path).
pub fn export_slices(wav: &Wav, plan: &SlicePlan) -> Vec<ExportedSlice> {
    plan.slices
        .iter()
        .map(|s| {
            (
                format!("slice_{:02}.wav", s.index),
                wav.window_wav(s.start_frame, s.end_frame),
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Sequencing — wire slices into a kit + pattern on a Project
// ---------------------------------------------------------------------------

/// How to place a sliced break into a backup: which kit + pattern, and the tone
/// slots the slice WAVs will be imported to (one per slice, in order).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreakbeatSpec {
    /// Kit index to build.
    pub kit_index: usize,
    /// 1-based pattern number to write the flip into.
    pub pattern_number: usize,
    /// Pattern variation (0 = A …).
    pub variation: usize,
    /// Tone IDs the slices are assigned to, in slice order. Typically user tones
    /// (`>= tr_format::USER_TONE_ID_MIN`). Only the first [`MAX_SLICES`] are
    /// mapped to voices.
    pub tone_ids: Vec<u16>,
    /// Optional new kit name.
    pub kit_name: Option<String>,
}

/// Build the kit + starter pattern for a sliced break on `project`.
///
/// Assigns each slice's tone to its own voice (up to [`MAX_SLICES`]) and writes
/// a **reconstruct** grid — each slice triggers once, at its natural position in
/// the bar (slice `i` at step `round(i * 16 / n)`), so the pattern plays the
/// break back in order. That is the starting point you then flip (reverse
/// snares, add ratchets/motion). Returns the number of slices mapped to voices.
///
/// Edits are length-preserving; call [`Project::save`] to serialize with a valid
/// CRC. Errors if the kit/pattern is out of range or the spec has no tones.
pub fn apply_breakbeat(
    project: &mut Project,
    plan: &SlicePlan,
    spec: &BreakbeatSpec,
) -> Result<usize> {
    ensure!(!spec.tone_ids.is_empty(), "spec has no tone IDs");
    ensure!(
        spec.tone_ids.len() >= plan.slices.len().min(MAX_SLICES),
        "need a tone ID per mapped slice ({} slices, {} tones)",
        plan.slices.len().min(MAX_SLICES),
        spec.tone_ids.len()
    );

    if let Some(name) = &spec.kit_name {
        ensure!(
            project.set_kit_name(spec.kit_index, name),
            "kit {} out of range",
            spec.kit_index
        );
    }

    let n = plan.slices.len();
    let mapped = n.min(MAX_SLICES);
    let steps = PATTERN_STEPS_PER_TRACK;

    // Assign each mapped slice's tone to its own voice.
    for (voice, &tone) in spec.tone_ids.iter().take(mapped).enumerate() {
        ensure!(
            project.set_kit_voice_tone(spec.kit_index, voice, tone),
            "kit {} voice {voice} out of range",
            spec.kit_index
        );
    }

    // Reconstruct grid: slice i triggers at its natural step.
    let mut grid = project
        .grid(spec.pattern_number, spec.variation)
        .ok_or_else(|| anyhow::anyhow!("pattern {} out of range", spec.pattern_number))?;
    // Clear the six voice rows first so we start from silence.
    for v in 0..VOICE_TRACKS {
        for s in 0..steps {
            grid.set(v, s, 0);
        }
    }
    for i in 0..mapped {
        let step = (i * steps) / n; // natural position in the bar
        grid.set(i, step.min(steps - 1), 100);
    }
    ensure!(
        project.apply_grid(spec.pattern_number, &grid),
        "pattern {} out of range",
        spec.pattern_number
    );

    // Slices beyond MAX_SLICES are not an error — their WAVs still exist for
    // manual use; callers compare `plan.slices.len()` against `mapped`.
    Ok(mapped)
}

// ---------------------------------------------------------------------------
// Shared-PCM windowing (cowbell-7po.3)
// ---------------------------------------------------------------------------
//
// The elegant path: import the break *once* as a single user sample, then make
// each slice a `PCMT` record that points at the SAME PCM `Address` with a
// different `Start`/`End` window. One upload, N slices, zero audio duplication.
//
// The window mapping is **proportional to the source's `EndMax`** (its full
// playable length), so it is unit-agnostic — it does the right thing whether
// the device measures `Start`/`End` in frames or in bytes, sidestepping that
// still-unconfirmed detail.
//
// What this DOES prove (round-trip within our own tooling): N records can be
// written to share one `Address` with distinct windows, losslessly. What it does
// NOT yet prove — and cannot without a sample-loaded reference backup or a
// device — the box *accepts* multiple records/tones sharing an `Address`, the
// `int8x4` numeric encoding (taken as LE `u32`), and how a fresh `TONE`-table
// entry is created for each slice tone. Those are the residual gates; this is
// the computation underneath them.

/// One planned shared-PCM slice: the `PCMT` record to write and its window
/// (`start`/`end`) over the source sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedWindow {
    /// `PCMT` record index to write.
    pub record_index: usize,
    /// Playback window start, in the source sample's native units.
    pub start: u32,
    /// Playback window end.
    pub end: u32,
}

/// Map a [`SlicePlan`]'s frame windows onto a source sample of length `extent`
/// (its `EndMax`), writing into `slice_records`. Proportional, so unit-agnostic.
/// Errors if `slice_records` is shorter than the plan or `extent`/frames is 0.
pub fn plan_shared_pcm(
    extent: u32,
    plan: &SlicePlan,
    slice_records: &[usize],
) -> Result<Vec<SharedWindow>> {
    let n = plan.slices.len();
    ensure!(
        slice_records.len() >= n,
        "need a PCMT record per slice ({n} slices, {} records)",
        slice_records.len()
    );
    ensure!(extent > 0 && plan.total_frames > 0, "empty source or plan");
    let total = plan.total_frames as u64;
    let ext = extent as u64;
    let map = |frame: usize| -> u32 { ((frame as u64 * ext) / total) as u32 };
    Ok(plan
        .slices
        .iter()
        .zip(slice_records)
        .map(|(s, &rec)| SharedWindow {
            record_index: rec,
            start: map(s.start_frame),
            end: map(s.end_frame),
        })
        .collect())
}

/// Window one already-imported break into N shared-PCM slice records on
/// `project`: every `slice_records[i]` is pointed at the source sample's PCM
/// `Address` with slice `i`'s window. The break's audio is stored once (in the
/// `source_record`); nothing is duplicated.
///
/// Length-preserving. Returns the windows written. Pair with [`apply_breakbeat`]
/// (kit assignment + flip) using the same tone IDs. See the residual gates in
/// this module's shared-PCM section — this is unverified on a real device.
pub fn apply_shared_pcm(
    project: &mut Project,
    source_record: usize,
    plan: &SlicePlan,
    slice_records: &[usize],
) -> Result<Vec<SharedWindow>> {
    let src = project
        .backup()
        .pcm_tone(source_record)
        .ok_or_else(|| anyhow::anyhow!("source PCMT record {source_record} not found"))?;
    let extent = if src.end_max > 0 {
        src.end_max
    } else {
        src.size
    };
    let (addr, addr_r) = (src.address, src.address_right);
    let windows = plan_shared_pcm(extent, plan, slice_records)?;
    let backup = project.backup_mut();
    for w in &windows {
        ensure!(
            backup.set_pcm_tone_address(w.record_index, addr, addr_r),
            "PCMT record {} out of range",
            w.record_index
        );
        ensure!(
            backup.set_pcm_tone_window(w.record_index, w.start, w.end),
            "PCMT record {} out of range",
            w.record_index
        );
    }
    Ok(windows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tr_format::{HEADER_LEN, PATTERN_RECORD_SIZE};

    /// A rising-ramp mono break of `frames` frames at 44.1k.
    fn ramp_wav(frames: usize) -> Wav {
        Wav {
            sample_rate: 44_100,
            channels: 1,
            samples: (0..frames).map(|i| (i as i16).wrapping_mul(7)).collect(),
        }
    }

    fn synthetic_backup() -> Vec<u8> {
        use tr_format::KIT_RECORD_SIZE;
        let mut v = Vec::new();
        v.extend_from_slice(b"TR6S");
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&5u32.to_le_bytes());
        v.resize(HEADER_LEN, 0);
        // One KIT record (zeroed) so voice assignment has somewhere to write.
        v.extend_from_slice(b"KIT ");
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&(KIT_RECORD_SIZE as u32).to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&vec![0u8; KIT_RECORD_SIZE]);
        // One pattern.
        v.extend_from_slice(b"PTN ");
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&(PATTERN_RECORD_SIZE as u32).to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&vec![0u8; PATTERN_RECORD_SIZE]);
        v
    }

    /// A backup with a `PCMT` chunk of `records` 64-byte entries.
    ///
    /// Record 0 is left empty: on a real backup that slot is the section's
    /// 16-byte array header, not a tone, and `Backup::pcm_tone` refuses it.
    /// Record 1 is the "source" sample (address `addr`, EndMax `extent`); the
    /// rest are spare slots for slices.
    fn backup_with_pcmt(records: usize, addr: u32, extent: u32) -> Vec<u8> {
        const ENTRY: usize = 0x40;
        let mut v = synthetic_backup();
        let payload = (records * ENTRY) as u32;
        v.extend_from_slice(b"PCMT");
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&payload.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        let mut recs = vec![0u8; records * ENTRY];
        let src = ENTRY; // record 1 — record 0 is the array header
        recs[src..src + 4].copy_from_slice(&addr.to_le_bytes()); // Address
        recs[src + 0x08..src + 0x0C].copy_from_slice(&extent.to_le_bytes()); // Size
        recs[src + 0x14..src + 0x18].copy_from_slice(&extent.to_le_bytes()); // EndMax
        v.extend_from_slice(&recs);
        v
    }

    #[test]
    fn shared_pcm_windows_share_one_address() {
        // Source record 1 holds the break (addr 0x1000, full length 0x8000).
        // Records 2..=5 become the four slices. Record 0 is the array header.
        let raw = backup_with_pcmt(6, 0x1000, 0x8000);
        let mut project = Project::open(raw).unwrap();
        let plan = slice_grid(&ramp_wav(64), 4).unwrap(); // 4 windows: 0,16,32,48,64
        let slice_records = [2usize, 3, 4, 5];

        let windows = apply_shared_pcm(&mut project, 1, &plan, &slice_records).unwrap();
        assert_eq!(windows.len(), 4);

        // Proportional to EndMax 0x8000 over 64 frames -> 0x2000 per quarter.
        let src_addr = project.backup().pcm_tone(1).unwrap().address;
        for (i, w) in windows.iter().enumerate() {
            let rec = project.backup().pcm_tone(w.record_index).unwrap();
            assert_eq!(rec.address, src_addr, "slice {i} shares the source address");
            assert_eq!(
                (rec.start, rec.end),
                (i as u32 * 0x2000, (i as u32 + 1) * 0x2000)
            );
        }
        // The source itself is untouched.
        assert_eq!(project.backup().pcm_tone(1).unwrap().start, 0);
        // ...and the array-header slot is not addressable as a tone at all.
        assert!(project.backup().pcm_tone(0).is_none());

        // Length-preserving + CRC valid after save.
        let bytes = project.save();
        assert!(Project::open(bytes).unwrap().backup().header_crc_valid());
    }

    #[test]
    fn shared_pcm_plan_is_unit_agnostic_and_validated() {
        let plan = slice_grid(&ramp_wav(100), 4).unwrap();
        // extent in "bytes" vs "frames" both map proportionally.
        let a = plan_shared_pcm(1000, &plan, &[0, 1, 2, 3]).unwrap();
        assert_eq!(a[0].start, 0);
        assert_eq!(a[3].end, 1000); // last window reaches the full extent
                                    // too few records / empty extent are rejected.
        assert!(plan_shared_pcm(1000, &plan, &[0, 1]).is_err());
        assert!(plan_shared_pcm(0, &plan, &[0, 1, 2, 3]).is_err());
        // missing source record is an error.
        let mut p = Project::open(synthetic_backup()).unwrap();
        assert!(apply_shared_pcm(&mut p, 0, &plan, &[1, 2, 3, 4]).is_err());
    }

    #[test]
    fn wav_round_trips() {
        let w = ramp_wav(100);
        let bytes = Wav::write(w.sample_rate, w.channels, &w.samples);
        let back = Wav::read(&bytes).unwrap();
        assert_eq!(w, back);
        assert_eq!(back.frames(), 100);
    }

    #[test]
    fn wav_rejects_non_pcm_and_missing_chunks() {
        assert!(Wav::read(b"nope").is_err());
        // 8-bit fmt should be rejected.
        let mut bytes = Wav::write(44_100, 1, &[1, 2, 3]);
        bytes[34] = 8; // bits-per-sample field
        assert!(Wav::read(&bytes).is_err());
    }

    #[test]
    fn grid_slices_cover_the_whole_break() {
        let w = ramp_wav(64);
        let plan = slice_grid(&w, 4).unwrap();
        assert_eq!(plan.slices.len(), 4);
        assert_eq!(
            plan.slices[0],
            Slice {
                index: 0,
                start_frame: 0,
                end_frame: 16
            }
        );
        assert_eq!(plan.slices[3].end_frame, 64); // last absorbs remainder
                                                  // Contiguous, gapless coverage.
        for w2 in plan.slices.windows(2) {
            assert_eq!(w2[0].end_frame, w2[1].start_frame);
        }
        // Remainder handling: 10 frames / 4 -> last slice is longer.
        let plan2 = slice_grid(&ramp_wav(10), 4).unwrap();
        assert_eq!(plan2.slices[3].end_frame, 10);
        assert_eq!(
            plan2.slices.iter().map(|s| s.len_frames()).sum::<usize>(),
            10
        );

        assert!(slice_grid(&w, 0).is_err());
        assert!(slice_grid(&ramp_wav(3), 4).is_err());
    }

    #[test]
    fn export_slices_are_valid_wavs_of_the_right_length() {
        let w = ramp_wav(64);
        let plan = slice_grid(&w, 4).unwrap();
        let files = export_slices(&w, &plan);
        assert_eq!(files.len(), 4);
        assert_eq!(files[0].0, "slice_00.wav");
        let s0 = Wav::read(&files[0].1).unwrap();
        assert_eq!(s0.frames(), 16);
        assert_eq!(s0.samples, w.samples[0..16]);
    }

    #[test]
    fn apply_breakbeat_wires_voices_and_reconstruct_grid() {
        let w = ramp_wav(64);
        let plan = slice_grid(&w, 4).unwrap();
        let mut project = Project::open(synthetic_backup()).unwrap();
        let spec = BreakbeatSpec {
            kit_index: 0,
            pattern_number: 1,
            variation: 0,
            tone_ids: vec![624, 625, 626, 627],
            kit_name: None, // no KIT section in this synthetic backup
        };
        let mapped = apply_breakbeat(&mut project, &plan, &spec).unwrap();
        assert_eq!(mapped, 4);

        // The reconstruct grid: slice i at step i*16/4 = 0,4,8,12 on voices 0..3.
        let g = project.grid(1, 0).unwrap();
        for (i, step) in [0usize, 4, 8, 12].into_iter().enumerate() {
            assert!(g.is_on(i, step), "slice {i} at step {step}");
        }
        assert!(!g.is_on(0, 1));

        // Saving keeps the header CRC valid.
        let bytes = project.save();
        assert!(Project::open(bytes).unwrap().backup().header_crc_valid());
    }

    #[test]
    fn more_than_six_slices_maps_only_six() {
        let w = ramp_wav(160);
        let plan = slice_grid(&w, 8).unwrap();
        let mut project = Project::open(synthetic_backup()).unwrap();
        let spec = BreakbeatSpec {
            kit_index: 0,
            pattern_number: 1,
            variation: 0,
            tone_ids: (624..624 + 8).collect(),
            kit_name: None,
        };
        let mapped = apply_breakbeat(&mut project, &plan, &spec).unwrap();
        assert_eq!(mapped, MAX_SLICES); // 6, not 8
    }

    #[test]
    fn apply_breakbeat_validates_inputs() {
        let plan = slice_grid(&ramp_wav(64), 4).unwrap();
        let mut project = Project::open(synthetic_backup()).unwrap();
        // No tones.
        let empty = BreakbeatSpec {
            kit_index: 0,
            pattern_number: 1,
            variation: 0,
            tone_ids: vec![],
            kit_name: None,
        };
        assert!(apply_breakbeat(&mut project, &plan, &empty).is_err());
        // Pattern out of range.
        let bad = BreakbeatSpec {
            kit_index: 0,
            pattern_number: 99,
            variation: 0,
            tone_ids: vec![624, 625, 626, 627],
            kit_name: None,
        };
        assert!(apply_breakbeat(&mut project, &plan, &bad).is_err());
    }
}
