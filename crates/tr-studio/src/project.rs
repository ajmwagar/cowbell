//! [`Project`] — an owning, browse-friendly handle over a whole backup.
//!
//! `tr-format` exposes lightweight offset-handles ([`tr_format::Kit`],
//! [`tr_format::Pattern`]) that thread the raw byte slice through every call.
//! That is precise but clunky for an editor. `Project` owns the [`Backup`] and
//! hands back **owned snapshots** ([`KitInfo`], [`ToneInfo`], [`SampleSlice`],
//! [`PatternInfo`]) you can read and display without borrowing the raw buffer,
//! plus a small set of edits and a [`Project::save`] that keeps the header CRC
//! valid. It is the reusable base the higher-level features (e.g. the breakbeat
//! slicer) build on.
//!
//! All plaintext user data — no firmware, no decryption.

use tr_format::{pcm::PcmTone, Backup, MAGIC_TR6S, MAGIC_TR8S, USER_TONE_ID_MIN, VOICES};

use crate::StepGrid;

/// One voice of a kit, resolved for display: which track, its assigned tone
/// (id + resolved name), and the headline params.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceInfo {
    /// Track label (`BD`/`SD`/`LT`/`HC`/`CH`/`OH`).
    pub track: &'static str,
    /// Assigned tone ID (index into the tone table).
    pub tone_id: u16,
    /// Resolved tone name, if the tone table has it.
    pub tone_name: Option<String>,
    /// Level (0–255).
    pub level: u8,
    /// Pan (0–255, center 128).
    pub pan: u8,
    /// Tune (0–255, center 128).
    pub tune: u8,
}

/// A kit, as an owned snapshot: its name and six resolved voices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KitInfo {
    /// Kit index within the `KIT ` section.
    pub index: usize,
    /// Kit name.
    pub name: String,
    /// The six audible voices (BD…OH).
    pub voices: [VoiceInfo; 6],
}

/// A tone-table entry: id, name, and whether it is a user (imported-sample)
/// tone versus a factory preset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToneInfo {
    /// Tone ID.
    pub id: u16,
    /// Tone name.
    pub name: String,
    /// `true` for user tones (`id >= USER_TONE_ID_MIN`).
    pub is_user: bool,
}

/// A populated `PCMT` record: where a sample's audio lives and the window that
/// plays. Multiple slices sharing an `address` with different `start`/`end` is
/// how the slicer windows one break into many hits (see `tr_format::pcm`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampleSlice {
    /// `PCMT` record index.
    pub index: usize,
    /// PCM byte offset of the sample in the `SMPL` region.
    pub address: u32,
    /// The sample's stored length.
    pub size: u32,
    /// Playback window start (slice in-point).
    pub start: u32,
    /// Playback window end (slice out-point).
    pub end: u32,
    /// Channel mode (mono/stereo).
    pub channel: u8,
    /// The tone slot(s) this record references.
    pub tone_ids: [u32; 4],
}

impl SampleSlice {
    fn from_record(index: usize, t: &PcmTone) -> SampleSlice {
        SampleSlice {
            index,
            address: t.address,
            size: t.size,
            start: t.start,
            end: t.end,
            channel: t.channel,
            tone_ids: t.tone_ids,
        }
    }
}

/// A pattern, as an owned snapshot: its number, name, and tempo.
#[derive(Debug, Clone, PartialEq)]
pub struct PatternInfo {
    /// 1-based pattern number.
    pub number: usize,
    /// Pattern name.
    pub name: String,
    /// Tempo in BPM.
    pub tempo_bpm: f32,
}

/// An owning, editable handle over a backup.
#[derive(Debug, Clone)]
pub struct Project {
    backup: Backup,
}

impl Project {
    /// Open a backup image.
    pub fn open(bytes: impl Into<Vec<u8>>) -> anyhow::Result<Project> {
        Ok(Project {
            backup: Backup::parse(bytes)?,
        })
    }

    /// Wrap an already-parsed [`Backup`].
    pub fn from_backup(backup: Backup) -> Project {
        Project { backup }
    }

    /// The underlying backup (read-only).
    pub fn backup(&self) -> &Backup {
        &self.backup
    }

    /// The underlying backup (mutable) — for edits `Project` does not wrap yet.
    pub fn backup_mut(&mut self) -> &mut Backup {
        &mut self.backup
    }

    /// Consume the `Project`, returning the backup.
    pub fn into_backup(self) -> Backup {
        self.backup
    }

    /// The device this backup is for (`"TR-6S"` / `"TR-8S"`), or `"unknown"`.
    pub fn device(&self) -> &'static str {
        match &self.backup.magic() {
            m if m == MAGIC_TR6S => "TR-6S",
            m if m == MAGIC_TR8S => "TR-8S",
            _ => "unknown",
        }
    }

    // ---- browse ----------------------------------------------------------

    /// All kits, as owned snapshots.
    pub fn kits(&self) -> Vec<KitInfo> {
        let raw = self.backup.raw();
        self.backup
            .kits()
            .iter()
            .enumerate()
            .map(|(index, kit)| {
                let name = kit.name(raw);
                let params = kit.voices(raw);
                let voices = std::array::from_fn(|v| {
                    let p = &params[v];
                    VoiceInfo {
                        track: VOICES[v],
                        tone_id: p.tone,
                        tone_name: self.backup.tone_name(p.tone),
                        level: p.level,
                        pan: p.pan,
                        tune: p.tune,
                    }
                });
                KitInfo {
                    index,
                    name,
                    voices,
                }
            })
            .collect()
    }

    /// One kit by index.
    pub fn kit(&self, index: usize) -> Option<KitInfo> {
        self.kits().into_iter().nth(index)
    }

    /// Every tone-table entry that has a name, as owned snapshots.
    pub fn tones(&self) -> Vec<ToneInfo> {
        (0u16..=1023)
            .filter_map(|id| {
                let name = self.backup.tone_name(id)?;
                if name.is_empty() {
                    return None;
                }
                Some(ToneInfo {
                    id,
                    name,
                    is_user: id >= USER_TONE_ID_MIN,
                })
            })
            .collect()
    }

    /// Every populated user-sample slice (`PCMT` record), in table order.
    ///
    /// Surfaces the sample table directly; linking a slice back to the tone
    /// slot that plays it is tracked separately (needs a sample-loaded backup —
    /// see `tr_format::pcm`).
    pub fn samples(&self) -> Vec<SampleSlice> {
        self.backup
            .pcm_tones()
            .iter()
            .enumerate()
            .filter(|(_, t)| t.is_populated())
            .map(|(i, t)| SampleSlice::from_record(i, t))
            .collect()
    }

    /// The declared size of the reserved user-sample region, if any.
    pub fn sample_region_size(&self) -> Option<u32> {
        self.backup.sample_region_size()
    }

    /// All patterns, as owned snapshots.
    pub fn patterns(&self) -> Vec<PatternInfo> {
        let raw = self.backup.raw();
        self.backup
            .patterns()
            .iter()
            .enumerate()
            .map(|(i, p)| PatternInfo {
                number: i + 1,
                name: p.name(raw),
                tempo_bpm: p.tempo_bpm(raw),
            })
            .collect()
    }

    /// Read a pattern variation's step grid (1-based pattern number).
    pub fn grid(&self, pattern_number: usize, variation: usize) -> Option<StepGrid> {
        crate::pattern_grid(&self.backup, pattern_number, variation)
    }

    // ---- edit ------------------------------------------------------------

    /// Rename a kit. Returns `false` if the index is out of range.
    pub fn set_kit_name(&mut self, index: usize, name: &str) -> bool {
        let Some(kit) = self.backup.kits().get(index).copied() else {
            return false;
        };
        kit.set_name(self.backup.raw_mut(), name)
    }

    /// Assign a tone to one of a kit's six voices. Returns `false` if out of
    /// range.
    pub fn set_kit_voice_tone(&mut self, index: usize, voice: usize, tone_id: u16) -> bool {
        let Some(kit) = self.backup.kits().get(index).copied() else {
            return false;
        };
        kit.set_voice_tone(self.backup.raw_mut(), voice, tone_id)
    }

    /// Write a step grid back into its pattern (1-based number). Returns `false`
    /// if the pattern does not exist.
    pub fn apply_grid(&mut self, pattern_number: usize, grid: &StepGrid) -> bool {
        let Some(p) = pattern_number
            .checked_sub(1)
            .and_then(|i| self.backup.patterns().get(i).copied())
        else {
            return false;
        };
        grid.apply(self.backup.raw_mut(), &p);
        true
    }

    /// Recompute the header CRC and return the current bytes — the safe way to
    /// serialize after edits.
    pub fn save(&mut self) -> Vec<u8> {
        self.backup.recompute_header_crc();
        self.backup.to_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tr_format::{HEADER_LEN, PATTERN_RECORD_SIZE};

    /// A synthetic backup with one pattern (BD steps 0 & 4 on). No Roland bytes.
    fn synthetic() -> Vec<u8> {
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
        for step in [0usize, 4] {
            rec[0xA0 + 4 + step * 4] = 100;
        }
        v.extend_from_slice(&rec);
        v
    }

    #[test]
    fn opens_and_browses() {
        let p = Project::open(synthetic()).unwrap();
        assert_eq!(p.device(), "TR-6S");
        let pats = p.patterns();
        assert_eq!(pats.len(), 1);
        assert_eq!(pats[0].number, 1);
        // no KIT/TONE/PCMT sections here -> empty, gracefully.
        assert!(p.kits().is_empty());
        assert!(p.tones().is_empty());
        assert!(p.samples().is_empty());
    }

    #[test]
    fn edits_pattern_and_saves_with_valid_crc() {
        let mut p = Project::open(synthetic()).unwrap();
        let mut g = p.grid(1, 0).unwrap();
        assert!(g.is_on(0, 0));
        g.toggle(1, 2); // SD step 2 on
        assert!(p.apply_grid(1, &g));
        assert!(!p.apply_grid(2, &g)); // no pattern 2

        let bytes = p.save();
        // The saved header CRC verifies against the round-tripped image.
        let reopened = Project::open(bytes).unwrap();
        assert!(reopened.backup().header_crc_valid());
        assert!(reopened.grid(1, 0).unwrap().is_on(1, 2));
    }

    #[test]
    fn out_of_range_edits_are_false() {
        let mut p = Project::open(synthetic()).unwrap();
        assert!(!p.set_kit_name(0, "x")); // no KIT section
        assert!(!p.set_kit_voice_tone(0, 0, 5));
        assert!(p.kit(0).is_none());
    }
}
