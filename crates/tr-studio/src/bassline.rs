//! Melodic / bassline patterns via per-step **TUNE motion**.
//!
//! The TR-6S has no piano-roll: pitch-per-step comes from the `TUNE` motion lane
//! (lane 0 of an instrument's motion word). This module is the write side of
//! that — the mirror of [`crate::StepGrid`] for steps — built on `tr-format`'s
//! [`tr_format::Pattern::set_motion_word`] primitive.
//!
//! ## What is solid vs. what needs calibration
//!
//! - **The mechanism is exact.** A motion word is `[TUNE, DECAY, CTRL, flags]`;
//!   flag **bit 7** = "TUNE recorded" (confirmed across every live motion word in
//!   the reference backup). [`Project::apply_tune_lane`] does a read-modify-write
//!   that sets `TUNE` + bit 7 and leaves the other lanes and undecoded flag bits
//!   untouched. This round-trips byte-for-byte.
//! - **The musical mapping is NOT device-verified.** Roland exposes `TUNE` as a
//!   **continuous ±128 detune** (`Script.xml`: `INST TUNE` range `0..255`,
//!   default `128`, knob `offsetValue -128`, `signed`), with **no declared
//!   semitone/cent mapping**. So how many raw units make a semitone depends on
//!   the DSP and must be measured on-device. [`TuneMap`] isolates that one
//!   constant; until it is calibrated, prefer raw values via
//!   [`TuneLane::set_raw`], and treat [`TuneMap::UNCALIBRATED`] as a placeholder.
//!
//! Pure plaintext user data — no firmware, no device.

use anyhow::{ensure, Result};
use tr_format::{PATTERN_STEPS_PER_TRACK, PATTERN_STEP_TRACKS};

use crate::{Project, VOICE_TRACKS};

/// The `TUNE` centre value — raw `128` is "no detune".
pub const TUNE_CENTER: u8 = 128;

/// Bit 7 of a motion word's flags byte: "lane 0 (`TUNE`) is recorded".
const TUNE_RECORDED_FLAG: u8 = 0x80;

/// The motion array slot for voice `v` — `Motion(v)` lives at `12 + v`.
fn tune_slot(voice: usize) -> usize {
    PATTERN_STEP_TRACKS + voice
}

/// Converts musical semitones to raw `TUNE` units around [`TUNE_CENTER`].
///
/// **`units_per_semitone` is not device-verified** — Roland declares no semitone
/// mapping for `TUNE` (it is a continuous ±128 detune). Measure it on hardware
/// (set known pitches, read back the value) and construct with [`TuneMap::new`];
/// [`TuneMap::UNCALIBRATED`] only *assumes* the full ±128 span ≈ ±1 octave so
/// the API is usable end-to-end before calibration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TuneMap {
    /// Raw `TUNE` units per semitone.
    pub units_per_semitone: f32,
}

impl TuneMap {
    /// A placeholder mapping assuming the full ±128 range spans ±1 octave
    /// (12 semitones). **Unverified** — see the type docs.
    pub const UNCALIBRATED: TuneMap = TuneMap {
        units_per_semitone: 128.0 / 12.0,
    };

    /// A calibrated mapping (raw units per semitone), e.g. from a device
    /// measurement.
    pub fn new(units_per_semitone: f32) -> TuneMap {
        TuneMap { units_per_semitone }
    }

    /// Raw `TUNE` for `semitones` above/below centre, clamped to `0..=255`.
    pub fn raw_for_semitone(&self, semitones: i32) -> u8 {
        let v = TUNE_CENTER as f32 + semitones as f32 * self.units_per_semitone;
        v.round().clamp(0.0, 255.0) as u8
    }
}

/// A per-step `TUNE` sequence for one voice — the pitches of a bassline. `None`
/// at a step means "no TUNE motion recorded there".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneLane {
    /// Voice (0 = BD … 5 = OH); its motion lives at slot `12 + voice`.
    pub voice: usize,
    /// Per-step raw `TUNE` value (`0..=255`, centre `128`), or `None`.
    pub values: [Option<u8>; PATTERN_STEPS_PER_TRACK],
}

impl TuneLane {
    /// An empty lane for `voice`.
    pub fn new(voice: usize) -> TuneLane {
        TuneLane {
            voice,
            values: [None; PATTERN_STEPS_PER_TRACK],
        }
    }

    /// Set a step's raw `TUNE` (`0..=255`, centre `128`).
    pub fn set_raw(&mut self, step: usize, tune: u8) {
        if step < PATTERN_STEPS_PER_TRACK {
            self.values[step] = Some(tune);
        }
    }

    /// Set a step's pitch in semitones relative to centre, via `map`.
    pub fn set_semitone(&mut self, step: usize, semitones: i32, map: &TuneMap) {
        self.set_raw(step, map.raw_for_semitone(semitones));
    }

    /// Clear a step's `TUNE` motion.
    pub fn clear(&mut self, step: usize) {
        if step < PATTERN_STEPS_PER_TRACK {
            self.values[step] = None;
        }
    }
}

/// One note of a bassline: which step, its pitch (semitones from centre), and
/// how hard it hits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Note {
    /// Step index (`0..16`).
    pub step: usize,
    /// Pitch in semitones relative to centre.
    pub semitone: i32,
    /// Velocity (`1..=127`; 0 would be an off step).
    pub velocity: u8,
}

impl Project {
    /// Read a voice's per-step `TUNE` motion for a pattern variation, or `None`
    /// if the pattern/voice is out of range.
    pub fn read_tune_lane(
        &self,
        pattern_number: usize,
        variation: usize,
        voice: usize,
    ) -> Option<TuneLane> {
        if voice >= VOICE_TRACKS {
            return None;
        }
        let p = *self
            .backup()
            .patterns()
            .get(pattern_number.checked_sub(1)?)?;
        let slot = tune_slot(voice);
        let mut lane = TuneLane::new(voice);
        for (step, cell) in lane.values.iter_mut().enumerate() {
            let w = p.motion_word(self.backup().raw(), variation, slot, step)?;
            *cell = w.lane(0);
        }
        Some(lane)
    }

    /// Write a voice's per-step `TUNE` motion for a pattern variation. Each step
    /// is a read-modify-write: it sets `TUNE` + the recorded flag where the lane
    /// has a value, clears both where it does not, and leaves the other lanes
    /// (`DECAY`/`CTRL`) and undecoded flag bits untouched. Length-preserving.
    /// Returns `false` if the pattern/voice is out of range.
    pub fn apply_tune_lane(
        &mut self,
        pattern_number: usize,
        variation: usize,
        lane: &TuneLane,
    ) -> bool {
        if lane.voice >= VOICE_TRACKS {
            return false;
        }
        let Some(p) = pattern_number
            .checked_sub(1)
            .and_then(|i| self.backup().patterns().get(i).copied())
        else {
            return false;
        };
        let slot = tune_slot(lane.voice);
        // Confirm the whole lane is addressable before mutating anything.
        for step in 0..PATTERN_STEPS_PER_TRACK {
            if p.motion_word(self.backup().raw(), variation, slot, step)
                .is_none()
            {
                return false;
            }
        }
        for (step, value) in lane.values.iter().enumerate() {
            let mut w = p
                .motion_word(self.backup().raw(), variation, slot, step)
                .expect("checked addressable above");
            match value {
                Some(v) => {
                    w.raw[0] = *v;
                    w.raw[3] |= TUNE_RECORDED_FLAG;
                }
                None => {
                    w.raw[0] = 0;
                    w.raw[3] &= !TUNE_RECORDED_FLAG;
                }
            }
            p.set_motion_word(self.backup_mut().raw_mut(), variation, slot, step, w);
        }
        true
    }

    /// Build a bassline on `voice`: clear its step row, then for each [`Note`]
    /// place a hit (velocity) and its `TUNE` pitch (via `map`). Writes both the
    /// step grid and the tune lane. Errors if the pattern/voice is out of range
    /// or a note step is `>= 16`.
    pub fn write_bassline(
        &mut self,
        pattern_number: usize,
        variation: usize,
        voice: usize,
        notes: &[Note],
        map: &TuneMap,
    ) -> Result<()> {
        ensure!(voice < VOICE_TRACKS, "voice {voice} out of range");
        let mut grid = self
            .grid(pattern_number, variation)
            .ok_or_else(|| anyhow::anyhow!("pattern {pattern_number} out of range"))?;
        let mut lane = TuneLane::new(voice);
        for s in 0..PATTERN_STEPS_PER_TRACK {
            grid.set(voice, s, 0);
        }
        for n in notes {
            ensure!(
                n.step < PATTERN_STEPS_PER_TRACK,
                "note step {} out of range",
                n.step
            );
            grid.set(voice, n.step, n.velocity);
            lane.set_semitone(n.step, n.semitone, map);
        }
        ensure!(
            self.apply_grid(pattern_number, &grid),
            "pattern {pattern_number} out of range"
        );
        ensure!(
            self.apply_tune_lane(pattern_number, variation, &lane),
            "pattern {pattern_number} out of range"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tr_format::{MotionWord, HEADER_LEN, PATTERN_RECORD_SIZE};

    fn synthetic_backup() -> Vec<u8> {
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
        v
    }

    #[test]
    fn tune_map_is_signed_around_centre() {
        let m = TuneMap::UNCALIBRATED;
        assert_eq!(m.raw_for_semitone(0), TUNE_CENTER);
        assert!(m.raw_for_semitone(3) > TUNE_CENTER);
        assert!(m.raw_for_semitone(-3) < TUNE_CENTER);
        // clamps, never wraps.
        assert_eq!(m.raw_for_semitone(1000), 255);
        assert_eq!(m.raw_for_semitone(-1000), 0);
    }

    #[test]
    fn tune_lane_round_trips_and_sets_the_recorded_flag() {
        let mut p = Project::open(synthetic_backup()).unwrap();
        let mut lane = TuneLane::new(0);
        lane.set_raw(0, 140);
        lane.set_raw(4, 100);
        assert!(p.apply_tune_lane(1, 0, &lane));

        let back = p.read_tune_lane(1, 0, 0).unwrap();
        assert_eq!(back.values[0], Some(140));
        assert_eq!(back.values[4], Some(100));
        assert_eq!(back.values[1], None);

        // The recorded flag (bit 7) is set on written steps.
        let pat = p.backup().patterns()[0];
        let w = pat
            .motion_word(p.backup().raw(), 0, tune_slot(0), 0)
            .unwrap();
        assert_eq!(w.raw[0], 140);
        assert!(w.raw[3] & TUNE_RECORDED_FLAG != 0);

        // save() keeps the header CRC valid.
        let bytes = p.save();
        assert!(Project::open(bytes).unwrap().backup().header_crc_valid());
    }

    #[test]
    fn apply_preserves_other_lanes_and_flag_bits() {
        let mut p = Project::open(synthetic_backup()).unwrap();
        let pat = p.backup().patterns()[0];
        // Seed DECAY (lane 1) + its flag bit 6 + an unknown flag bit at step 2.
        let seeded = MotionWord {
            raw: [0, 77, 5, 0x40 | 0x01],
        };
        pat.set_motion_word(p.backup_mut().raw_mut(), 0, tune_slot(0), 2, seeded);

        let mut lane = TuneLane::new(0);
        lane.set_raw(2, 200);
        assert!(p.apply_tune_lane(1, 0, &lane));

        let w = pat
            .motion_word(p.backup().raw(), 0, tune_slot(0), 2)
            .unwrap();
        assert_eq!(w.raw[0], 200); // TUNE written
        assert_eq!(w.raw[1], 77); // DECAY preserved
        assert_eq!(w.raw[2], 5); // CTRL preserved
        assert_eq!(w.raw[3], 0x80 | 0x40 | 0x01); // TUNE flag added, others kept
    }

    #[test]
    fn write_bassline_places_hits_and_pitches() {
        let mut p = Project::open(synthetic_backup()).unwrap();
        let notes = [
            Note {
                step: 0,
                semitone: 0,
                velocity: 110,
            },
            Note {
                step: 3,
                semitone: 7,
                velocity: 90,
            },
            Note {
                step: 8,
                semitone: -5,
                velocity: 100,
            },
        ];
        p.write_bassline(1, 0, 0, &notes, &TuneMap::UNCALIBRATED)
            .unwrap();

        let grid = p.grid(1, 0).unwrap();
        assert!(grid.is_on(0, 0) && grid.is_on(0, 3) && grid.is_on(0, 8));
        assert!(!grid.is_on(0, 1));
        assert_eq!(grid.velocity(0, 3), 90);

        let lane = p.read_tune_lane(1, 0, 0).unwrap();
        assert_eq!(lane.values[0], Some(TUNE_CENTER));
        assert!(lane.values[3].unwrap() > TUNE_CENTER); // +7 semis
        assert!(lane.values[8].unwrap() < TUNE_CENTER); // -5 semis
        assert_eq!(lane.values[1], None);

        // out-of-range guards
        assert!(p
            .write_bassline(1, 0, 9, &notes, &TuneMap::UNCALIBRATED)
            .is_err());
        assert!(p
            .write_bassline(99, 0, 0, &notes, &TuneMap::UNCALIBRATED)
            .is_err());
        let bad = [Note {
            step: 99,
            semitone: 0,
            velocity: 100,
        }];
        assert!(p
            .write_bassline(1, 0, 0, &bad, &TuneMap::UNCALIBRATED)
            .is_err());
    }
}
