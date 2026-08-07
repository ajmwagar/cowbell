//! `tr-studio` — the ergonomic layer over [`tr_format`].
//!
//! Where `tr-format` is raw offsets, records, and step words, this is
//! [`StepGrid`]s, named voices, and (in time) pattern builders. It's the
//! friendly half of a FOSS alternative to Roland's TR-EDITOR.
//!
//! Everything here is plaintext user data — no firmware, no decryption.

use tr_format::{Backup, Pattern, PATTERN_STEPS_PER_TRACK, PATTERN_STEP_TRACKS, VOICES};

/// Step-array tracks 0..6 are the six audible voices (BD/SD/LT/HC/CH/OH);
/// verified musically on the reference backup. Tracks 6..11 are the unused
/// INST07–11 slots; 11..25 are engine sub-step / motion planes.
pub const VOICE_TRACKS: usize = 6;

/// A pattern variation's step grid: [`PATTERN_STEP_TRACKS`] tracks ×
/// [`PATTERN_STEPS_PER_TRACK`] steps of velocity (0 = off).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepGrid {
    /// `steps[track][step]` = velocity (0 = off).
    steps: Vec<[u8; PATTERN_STEPS_PER_TRACK]>,
    /// Which variation (0 = A … 7 = H, 8/9 = fills) this grid came from.
    pub variation: usize,
}

impl StepGrid {
    /// Read a pattern variation into a grid.
    pub fn read(raw: &[u8], pattern: &Pattern, variation: usize) -> StepGrid {
        let mut steps = vec![[0u8; PATTERN_STEPS_PER_TRACK]; PATTERN_STEP_TRACKS];
        for (t, row) in steps.iter_mut().enumerate() {
            for (s, cell) in row.iter_mut().enumerate() {
                if let Some(w) = pattern.step_word(raw, variation, t, s) {
                    *cell = w.velocity();
                }
            }
        }
        StepGrid { steps, variation }
    }

    /// Velocity at (track, step); 0 (off) if out of range.
    pub fn velocity(&self, track: usize, step: usize) -> u8 {
        self.steps
            .get(track)
            .and_then(|r| r.get(step))
            .copied()
            .unwrap_or(0)
    }

    /// Whether the step is on.
    pub fn is_on(&self, track: usize, step: usize) -> bool {
        self.velocity(track, step) != 0
    }

    /// Set a step in the grid (in memory). Use [`StepGrid::apply`] to write back.
    pub fn set(&mut self, track: usize, step: usize, velocity: u8) {
        if let Some(row) = self.steps.get_mut(track) {
            if let Some(cell) = row.get_mut(step) {
                *cell = velocity;
            }
        }
    }

    /// Toggle a step on (default velocity 100) / off.
    pub fn toggle(&mut self, track: usize, step: usize) {
        let v = if self.is_on(track, step) { 0 } else { 100 };
        self.set(track, step, v);
    }

    /// The 16-step row for one of the six audible voices (0 = BD … 5 = OH).
    pub fn voice_row(&self, voice: usize) -> Option<&[u8; PATTERN_STEPS_PER_TRACK]> {
        if voice < VOICE_TRACKS {
            self.steps.get(voice)
        } else {
            None
        }
    }

    /// Write this grid back into a backup's raw bytes (via `tr-format`'s
    /// low-level writer). NOTE: does not recompute the per-record checksum, so
    /// the result is for analysis, not yet a device-safe backup.
    pub fn apply(&self, raw: &mut [u8], pattern: &Pattern) {
        for (t, row) in self.steps.iter().enumerate() {
            for (s, &v) in row.iter().enumerate() {
                pattern.set_step_velocity(raw, self.variation, t, s, v);
            }
        }
    }

    /// A labeled ASCII rendering of the six voice rows (beats grouped by 4).
    pub fn render(&self) -> String {
        let mut out = String::new();
        for (i, name) in VOICES.iter().enumerate() {
            let mut cells = String::new();
            for s in 0..PATTERN_STEPS_PER_TRACK {
                if s > 0 && s % 4 == 0 {
                    cells.push(' ');
                }
                cells.push(if self.is_on(i, s) { 'X' } else { '.' });
            }
            out.push_str(&format!("  {name:<3}|{cells}|\n"));
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
        assert_eq!(g.voice_row(0).unwrap()[0], 100);
        assert_eq!(g.render().lines().count(), VOICES.len());
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
