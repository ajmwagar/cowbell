//! `tr-studio` — the ergonomic layer over [`tr_format`].
//!
//! Where `tr-format` is raw offsets, records, and step words, this is
//! [`StepGrid`]s, named voices, and (in time) pattern builders. It's the
//! friendly half of a FOSS alternative to Roland's TR-EDITOR.
//!
//! Everything here is plaintext user data — no firmware, no decryption.

use tr_format::{
    Backup, Pattern, StepWord, SubStep, PATTERN_STEPS_PER_TRACK, PATTERN_STEP_TRACKS, VOICES,
};

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
