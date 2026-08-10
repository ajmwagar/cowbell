//! `tr-studio` — the ergonomic layer over [`tr_format`].
//!
//! Where `tr-format` is raw offsets, records, and step words, this is
//! [`StepGrid`]s, named voices, and (in time) pattern builders. It's the
//! friendly half of a FOSS alternative to Roland's TR-EDITOR.
//!
//! Everything here is plaintext user data — no firmware, no decryption.

use tr_format::{
    motion_lane_name, Backup, Pattern, StepWord, SubStep, MOTION_LANES, PATTERN_ARRAY_SLOTS,
    PATTERN_STEPS_PER_TRACK, PATTERN_STEP_TRACKS, VOICES,
};

mod project;
pub use project::{KitInfo, PatternInfo, Project, SampleSlice, ToneInfo, VoiceInfo};

/// Step tracks 0..6 are the six audible voices (BD/SD/LT/HC/CH/OH); verified
/// musically on the reference backup. Tracks 6..11 are the unused INST07–11
/// slots and track 11 is TRIG (trigger out).
pub const VOICE_TRACKS: usize = 6;

/// A pattern variation's step grid: [`PATTERN_STEP_TRACKS`] tracks ×
/// [`PATTERN_STEPS_PER_TRACK`] [`StepWord`]s (velocity 0 = off).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepGrid {
    /// `steps[track][step]` — the decoded step word.
    steps: Vec<[StepWord; PATTERN_STEPS_PER_TRACK]>,
    /// Which variation (0 = A … 7 = H, 8/9 = fills) this grid came from.
    pub variation: usize,
}

const OFF: StepWord = StepWord { raw: [0; 4] };

impl StepGrid {
    /// Read a pattern variation into a grid.
    pub fn read(raw: &[u8], pattern: &Pattern, variation: usize) -> StepGrid {
        let mut steps = vec![[OFF; PATTERN_STEPS_PER_TRACK]; PATTERN_STEP_TRACKS];
        for (t, row) in steps.iter_mut().enumerate() {
            for (s, cell) in row.iter_mut().enumerate() {
                if let Some(w) = pattern.step_word(raw, variation, t, s) {
                    *cell = w;
                }
            }
        }
        StepGrid { steps, variation }
    }

    /// The whole step word at (track, step); an off step if out of range.
    pub fn word(&self, track: usize, step: usize) -> StepWord {
        self.steps
            .get(track)
            .and_then(|r| r.get(step))
            .copied()
            .unwrap_or(OFF)
    }

    /// Velocity at (track, step); 0 (off) if out of range.
    pub fn velocity(&self, track: usize, step: usize) -> u8 {
        self.word(track, step).velocity()
    }

    /// Whether the step is on.
    pub fn is_on(&self, track: usize, step: usize) -> bool {
        self.velocity(track, step) != 0
    }

    /// The step's sub-step (retrigger/flam) mode, if any.
    pub fn sub_step(&self, track: usize, step: usize) -> Option<SubStep> {
        self.word(track, step).sub_step()
    }

    /// Whether the step is flagged ALTERNATE.
    pub fn is_alternate(&self, track: usize, step: usize) -> bool {
        self.word(track, step).is_alternate()
    }

    /// Set a step's velocity in the grid (in memory), leaving its sub-step,
    /// alternate flag, and undecoded bits alone. Use [`StepGrid::apply`] to
    /// write back.
    pub fn set(&mut self, track: usize, step: usize, velocity: u8) {
        if let Some(cell) = self.steps.get_mut(track).and_then(|r| r.get_mut(step)) {
            cell.set_velocity(velocity);
        }
    }

    /// Set a step's sub-step mode (`None` clears it).
    pub fn set_sub_step(&mut self, track: usize, step: usize, sub: Option<SubStep>) {
        if let Some(cell) = self.steps.get_mut(track).and_then(|r| r.get_mut(step)) {
            cell.set_sub_step(sub);
        }
    }

    /// Set a step's ALTERNATE flag.
    pub fn set_alternate(&mut self, track: usize, step: usize, on: bool) {
        if let Some(cell) = self.steps.get_mut(track).and_then(|r| r.get_mut(step)) {
            cell.set_alternate(on);
        }
    }

    /// Toggle a step on (default velocity 100) / off.
    pub fn toggle(&mut self, track: usize, step: usize) {
        let v = if self.is_on(track, step) { 0 } else { 100 };
        self.set(track, step, v);
    }

    /// The 16-step row for one of the six audible voices (0 = BD … 5 = OH).
    pub fn voice_row(&self, voice: usize) -> Option<&[StepWord; PATTERN_STEPS_PER_TRACK]> {
        if voice < VOICE_TRACKS {
            self.steps.get(voice)
        } else {
            None
        }
    }

    /// Write this grid back into a backup's raw bytes (via `tr-format`'s
    /// low-level writer). User slots (≥ 1) carry no checksum, so a
    /// length-preserving edit here leaves the record structurally valid.
    pub fn apply(&self, raw: &mut [u8], pattern: &Pattern) {
        for (t, row) in self.steps.iter().enumerate() {
            for (s, &w) in row.iter().enumerate() {
                pattern.set_step_word(raw, self.variation, t, s, w);
            }
        }
    }

    /// A labeled ASCII rendering of the six voice rows (beats grouped by 4).
    /// A step is `X`, or its sub-step mode: `F` flam, `2`/`3`/`4` retriggers.
    /// Rows with ALTERNATE steps list them after the grid.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for (i, name) in VOICES.iter().enumerate() {
            let mut cells = String::new();
            for s in 0..PATTERN_STEPS_PER_TRACK {
                if s > 0 && s % 4 == 0 {
                    cells.push(' ');
                }
                cells.push(match (self.is_on(i, s), self.sub_step(i, s)) {
                    (false, _) => '.',
                    (true, None) => 'X',
                    (true, Some(SubStep::Flam)) => 'F',
                    (true, Some(SubStep::Half)) => '2',
                    (true, Some(SubStep::Third)) => '3',
                    (true, Some(SubStep::Quarter)) => '4',
                });
            }
            let alt: Vec<String> = (0..PATTERN_STEPS_PER_TRACK)
                .filter(|&s| self.is_alternate(i, s))
                .map(|s| (s + 1).to_string())
                .collect();
            let suffix = if alt.is_empty() {
                String::new()
            } else {
                format!("  alt: {}", alt.join(","))
            };
            out.push_str(&format!("  {name:<3}|{cells}|{suffix}\n"));
        }
        out
    }
}

/// One instrument/plane's recorded motion for a variation: for each of the
/// three lanes, the per-step values (`None` = nothing recorded at that step).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MotionLanes {
    /// The array slot this came from (12–24).
    pub slot: usize,
    /// `lanes[lane][step]`, with the lane's parameter name.
    pub lanes: Vec<(&'static str, [Option<u8>; PATTERN_STEPS_PER_TRACK])>,
}

impl MotionLanes {
    /// Read one motion slot (12–24) of a variation; `None` if not a motion slot.
    pub fn read(
        raw: &[u8],
        pattern: &Pattern,
        variation: usize,
        slot: usize,
    ) -> Option<MotionLanes> {
        let mut lanes = Vec::new();
        for lane in 0..MOTION_LANES {
            let name = motion_lane_name(slot, lane)?;
            let mut vals = [None; PATTERN_STEPS_PER_TRACK];
            for (s, v) in vals.iter_mut().enumerate() {
                *v = pattern
                    .motion_word(raw, variation, slot, s)
                    .and_then(|w| w.lane(lane));
            }
            lanes.push((name, vals));
        }
        Some(MotionLanes { slot, lanes })
    }

    /// Whether any lane recorded anything.
    pub fn is_empty(&self) -> bool {
        self.lanes
            .iter()
            .all(|(_, v)| v.iter().all(Option::is_none))
    }

    /// One line per lane that has data: `TUNE  | 124  --- 128 ...`.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for (name, vals) in &self.lanes {
            if vals.iter().all(Option::is_none) {
                continue;
            }
            let cells: Vec<String> = vals
                .iter()
                .map(|v| v.map_or("  -".to_string(), |x| format!("{x:3}")))
                .collect();
            out.push_str(&format!("  {name:<14}|{}|\n", cells.join(" ")));
        }
        out
    }
}

/// Every motion slot of a variation that has data, in slot order.
pub fn variation_motion(raw: &[u8], pattern: &Pattern, variation: usize) -> Vec<MotionLanes> {
    (PATTERN_STEP_TRACKS..PATTERN_ARRAY_SLOTS)
        .filter_map(|slot| MotionLanes::read(raw, pattern, variation, slot))
        .filter(|m| !m.is_empty())
        .collect()
}

/// Convenience: read a pattern's variation grid straight from a backup.
pub fn pattern_grid(backup: &Backup, pattern_number: usize, variation: usize) -> Option<StepGrid> {
    let p = *backup.patterns().get(pattern_number.checked_sub(1)?)?;
    Some(StepGrid::read(backup.raw(), &p, variation))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_backup_with_pattern() -> Vec<u8> {
        use tr_format::{HEADER_LEN, PATTERN_RECORD_SIZE};
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
        // var0 track0 (BD) steps 0 and 4 on
        for step in [0usize, 4] {
            rec[0xA0 + 4 + step * 4] = 100;
        }
        v.extend_from_slice(&rec);
        v
    }

    #[test]
    fn reads_grid_and_voice_row() {
        let b = Backup::parse(synthetic_backup_with_pattern()).unwrap();
        let g = pattern_grid(&b, 1, 0).unwrap();
        assert!(g.is_on(0, 0));
        assert!(g.is_on(0, 4));
        assert!(!g.is_on(0, 1));
        assert_eq!(g.voice_row(0).unwrap()[0].velocity(), 100);
        assert_eq!(g.render().lines().count(), VOICES.len());
    }

    #[test]
    fn reads_motion_lanes() {
        use tr_format::PATTERN_RECORD_SIZE;
        let mut v = synthetic_backup_with_pattern();
        // var0, slot 12 (BD motion), step 2: TUNE=124, DECAY=116, CTRL=88,
        // flags = lane0 + lane1 recorded.
        let rec_start = v.len() - PATTERN_RECORD_SIZE;
        let o = rec_start + 0xA0 + 4 + 12 * 64 + 2 * 4;
        v[o..o + 4].copy_from_slice(&[124, 116, 88, 0b1100_0010]);
        let b = Backup::parse(v).unwrap();
        let p = b.patterns()[0];

        let m = MotionLanes::read(b.raw(), &p, 0, 12).unwrap();
        assert!(!m.is_empty());
        assert_eq!(m.lanes[0].0, "TUNE");
        assert_eq!(m.lanes[0].1[2], Some(124));
        assert_eq!(m.lanes[1].1[2], Some(116));
        assert_eq!(m.lanes[2].1[2], Some(88));
        assert_eq!(m.lanes[0].1[0], None);
        assert!(m.render().contains("TUNE"));

        // An untouched motion slot is empty, and only slot 12 shows up.
        assert!(MotionLanes::read(b.raw(), &p, 0, 13).unwrap().is_empty());
        assert_eq!(MotionLanes::read(b.raw(), &p, 0, 0), None);
        let planes = variation_motion(b.raw(), &p, 0);
        assert_eq!(planes.len(), 1);
        assert_eq!(planes[0].slot, 12);
        assert!(variation_motion(b.raw(), &p, 1).is_empty());
    }

    #[test]
    fn sub_step_and_alternate_survive_apply() {
        let mut b = Backup::parse(synthetic_backup_with_pattern()).unwrap();
        let p = b.patterns()[0];
        let mut g = StepGrid::read(b.raw(), &p, 0);
        g.set_sub_step(0, 0, Some(SubStep::Quarter));
        g.set_alternate(0, 4, true);
        g.apply(b.raw_mut(), &p);

        let g2 = StepGrid::read(b.raw(), &p, 0);
        assert_eq!(g2.sub_step(0, 0), Some(SubStep::Quarter));
        assert!(!g2.is_alternate(0, 0));
        assert!(g2.is_alternate(0, 4));
        assert_eq!(g2.sub_step(0, 4), None);
        assert_eq!(g2.velocity(0, 0), 100);
        let bd = g2.render().lines().next().unwrap().to_string();
        assert!(bd.contains("4..."), "{bd}");
        assert!(bd.ends_with("alt: 5"), "{bd}");
    }

    #[test]
    fn edit_and_apply_round_trips_through_raw() {
        let mut b = Backup::parse(synthetic_backup_with_pattern()).unwrap();
        let p = b.patterns()[0];
        let mut g = StepGrid::read(b.raw(), &p, 0);
        g.toggle(1, 2); // turn on SD step 2
        g.set(0, 0, 0); // turn off BD step 0
        g.apply(b.raw_mut(), &p);
        let g2 = StepGrid::read(b.raw(), &p, 0);
        assert!(g2.is_on(1, 2));
        assert!(!g2.is_on(0, 0));
        assert!(g2.is_on(0, 4));
    }
}
