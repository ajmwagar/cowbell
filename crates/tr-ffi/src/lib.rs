//! `tr-ffi` — the UniFFI surface over [`tr_format`] and [`tr_studio`].
//!
//! This crate is the boundary between the Rust core and the Swift app
//! (Groovebank, macOS + iPadOS). It holds **no format knowledge of its own**:
//! every offset, mask and decode lives one layer down. What it adds is the
//! shape Swift wants — owned value types instead of borrowed views into a byte
//! buffer, `Result` instead of `Option`/`bool`-returning setters, and one
//! `Arc`-shared handle that owns the loaded file.
//!
//! ## Why the DTOs look the way they do
//!
//! `tr-format`'s [`Kit`]/[`Pattern`] are *cursors*: an index plus an offset,
//! with every accessor taking `&[u8]`. That is the right design in Rust and the
//! wrong one across an FFI — Swift would have to hold the buffer and pass it
//! back on every call. So each record here is fully materialised
//! ([`KitInfo`], [`PatternInfo`], [`GridInfo`]) at the moment it is asked for.
//! Reads are cheap and the app is not in an audio thread; correctness across
//! the boundary is worth more than avoiding a copy.
//!
//! ## Indexing
//!
//! **Everything crossing this boundary is 0-based**, including `pattern_index`
//! and `kit_index`, even though the hardware and `tr-format`'s CLI display
//! slots 1-based. The `index` field carried on each record is the same 0-based
//! value; presentation is the app's job. (`tr_studio::pattern_grid` takes a
//! 1-based number — the conversion happens here, once, in [`BackupFile::grid`].)
//!
//! Everything here is plaintext user data — no firmware, no decryption.

use std::ops::Range;
use std::sync::{Arc, RwLock};

use tr_format::{
    Backup, Kit, Pattern, TrackRole, HEADER_LEN, KIT_RECORD_SIZE, PATTERN_RECORD_SIZE,
    PATTERN_STEPS_PER_TRACK, PATTERN_STEP_TRACKS, PATTERN_VARIATIONS, SLIDER_COLORS,
};
use tr_studio::StepGrid;

pub mod device;
pub mod journal;

pub use journal::{EditKind, EditRecord, EditSpan};
use journal::Journal;

uniffi::setup_scaffolding!();

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Everything that can go wrong across the boundary.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum TrError {
    /// The bytes are not a TR backup container (bad magic, truncated, …).
    #[error("could not parse backup: {message}")]
    Parse { message: String },
    /// The file could not be read from / written to disk.
    #[error("i/o error on {path}: {message}")]
    Io { path: String, message: String },
    /// A slot index was past the end of its section.
    #[error("{what} index {index} out of range (have {count})")]
    OutOfRange { what: String, index: u32, count: u32 },
    /// A write was rejected by the underlying format layer.
    #[error("write rejected: {message}")]
    Write { message: String },
    /// Something went wrong talking to a connected device: the transport
    /// failed, a reply timed out, or the device answered with something the
    /// protocol layer rejected (bad checksum, wrong address, not a DT1).
    #[error("device error: {message}")]
    Device { message: String },
}

// ---------------------------------------------------------------------------
// Value types
// ---------------------------------------------------------------------------

/// One chunk in the container's section directory.
#[derive(Debug, Clone, uniffi::Record)]
pub struct SectionInfo {
    /// Four-character chunk tag, e.g. `"KIT "` or `"PTN "`.
    pub tag: String,
    pub payload_offset: u64,
    pub payload_len: u64,
}

/// Header-level facts about a loaded backup.
#[derive(Debug, Clone, uniffi::Record)]
pub struct BackupInfo {
    /// `"TR6S"` or `"TR8S"`.
    pub magic: String,
    pub version: u32,
    /// Whether the stored header CRC matches the recomputed one. A `false` here
    /// is worth surfacing in the UI — it means the file was edited by something
    /// that did not fix up the checksum.
    pub header_crc_valid: bool,
    pub kit_count: u32,
    pub pattern_count: u32,
    /// Total file size in bytes.
    pub byte_len: u64,
    pub sections: Vec<SectionInfo>,
}

/// One of a kit's six voices, with its tone name already resolved.
#[derive(Debug, Clone, uniffi::Record)]
pub struct VoiceInfo {
    /// Panel name — `"BD"`, `"SD"`, `"LT"`, `"HC"`, `"CH"`, `"OH"`.
    pub name: String,
    pub tone_id: u16,
    /// Resolved from the backup's tone table; empty if the id is not present.
    pub tone_name: String,
    /// Index into the twelve panel LED colours — what this voice's fader and pad
    /// actually light up as on the hardware.
    pub color: u8,
    /// The colour's panel name, e.g. `"SkyBlue"`.
    pub color_name: String,
    pub tune: u8,
    pub decay: u8,
    pub level: u8,
    pub gain: u8,
    pub pan: u8,
    pub reverb_send: u8,
    pub delay_send: u8,
}

/// A kit slot: name plus its six voices.
#[derive(Debug, Clone, uniffi::Record)]
pub struct KitInfo {
    /// 0-based slot index.
    pub index: u32,
    pub name: String,
    pub voices: Vec<VoiceInfo>,
}

/// A pattern slot's header fields. The step data is fetched separately via
/// [`BackupFile::grid`] — a pattern record is ~24 KiB and the browser list does
/// not need it.
#[derive(Debug, Clone, uniffi::Record)]
pub struct PatternInfo {
    /// 0-based slot index.
    pub index: u32,
    pub name: String,
    pub tempo_bpm: f32,
    /// The kit slot this pattern plays, as stored: **1-based**, 1–128. This one
    /// field keeps the hardware's numbering because it is a stored value, not
    /// an index into anything we hand back.
    pub kit_ref: u8,
}

/// A single step in a track row.
#[derive(Debug, Clone, uniffi::Record)]
pub struct StepInfo {
    pub on: bool,
    /// 0 when off; otherwise the recorded velocity.
    pub velocity: u8,
    /// Sub-step label (flam/roll subdivision), or `None` for a plain hit.
    pub sub_step: Option<String>,
    /// Whether the step fires on the alternate pass.
    pub alternate: bool,
}

/// One track row of a variation grid.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TrackInfo {
    /// Panel name for the track on this device.
    pub name: String,
    /// True for the tracks that make sound on this device — six on a TR-6S,
    /// eleven on a TR-8S. The trigger-out row is not a voice. The app uses this
    /// to decide what to show by default.
    pub is_voice: bool,
    /// LED colour index of this track, taken from the kit the pattern
    /// references — so the grid lights up the way the box will when this
    /// pattern plays. `None` for the trigger row and unresolvable kit refs.
    pub color: Option<u8>,
    /// Always [`PATTERN_STEPS_PER_TRACK`] entries.
    pub steps: Vec<StepInfo>,
}

/// A pattern variation's full step grid.
#[derive(Debug, Clone, uniffi::Record)]
pub struct GridInfo {
    pub pattern_index: u32,
    /// 0 = A … 7 = H, 8/9 = the two fills.
    pub variation: u32,
    /// Display label for the variation — `"A"`…`"H"`, `"Fill 1"`, `"Fill 2"`.
    pub variation_label: String,
    pub tracks: Vec<TrackInfo>,
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// The core's version string, for the app's about box and for proving at
/// runtime that Swift is talking to the Rust it thinks it is.
#[uniffi::export]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Number of variations a pattern has (A–H plus two fills).
#[uniffi::export]
pub fn variation_count() -> u32 {
    PATTERN_VARIATIONS as u32
}

/// Steps per track in a variation.
#[uniffi::export]
pub fn steps_per_track() -> u32 {
    PATTERN_STEPS_PER_TRACK as u32
}

/// The twelve panel LED colour names, in stored-index order.
///
/// The app renders its own swatches, but the names have to come from here so a
/// picker cannot offer a thirteenth colour the hardware has no value for.
#[uniffi::export]
pub fn slider_color_names() -> Vec<String> {
    SLIDER_COLORS.iter().map(|s| s.to_string()).collect()
}

/// Display label for a variation index: `"A"`…`"H"`, then `"Fill 1"`/`"Fill 2"`.
#[uniffi::export]
pub fn variation_label(variation: u32) -> String {
    match variation {
        0..=7 => ((b'A' + variation as u8) as char).to_string(),
        8 | 9 => format!("Fill {}", variation - 7),
        _ => format!("?{variation}"),
    }
}

// ---------------------------------------------------------------------------
// The handle
// ---------------------------------------------------------------------------

/// A loaded backup file.
///
/// Swift holds this as a reference type and calls methods on it; the byte
/// buffer never crosses the boundary except on an explicit
/// [`BackupFile::to_bytes`]. The `RwLock` is what makes the mutating setters
/// safe to expose behind UniFFI's `&self`-only methods — reads take a shared
/// guard, edits take an exclusive one. UniFFI objects are `Send + Sync`, so
/// this must genuinely be safe under concurrent access, not just conventionally
/// single-threaded.
#[derive(Debug, uniffi::Object)]
pub struct BackupFile {
    state: RwLock<State>,
    /// Where it came from, if it was opened from disk — used for `save()` with
    /// no argument and for error messages.
    path: RwLock<Option<String>>,
}

/// The buffer and its edit history, under one lock.
///
/// They share a lock rather than having one each because applying an edit and
/// recording it have to be atomic: a reader that caught the buffer changed but
/// the journal not yet updated would see a state that can never be undone.
#[derive(Debug)]
struct State {
    backup: Backup,
    journal: Journal,
}

impl BackupFile {
    fn new(backup: Backup, path: Option<String>) -> Arc<Self> {
        Arc::new(BackupFile {
            state: RwLock::new(State {
                backup,
                journal: Journal::default(),
            }),
            path: RwLock::new(path),
        })
    }

    /// Run `f` against the parsed backup under a read guard.
    fn read<T>(&self, f: impl FnOnce(&Backup) -> T) -> T {
        f(&self.state.read().expect("backup lock poisoned").backup)
    }

    fn with_journal<T>(&self, f: impl FnOnce(&mut Journal) -> T) -> T {
        f(&mut self.state.write().expect("backup lock poisoned").journal)
    }

    /// Apply a user edit and journal the bytes it moved.
    ///
    /// `window` is computed from the backup before the edit runs, because it is
    /// derived from record offsets that the edit itself may overwrite.
    fn journal_edit(
        &self,
        kind: EditKind,
        window: impl FnOnce(&Backup) -> Range<usize>,
        f: impl FnOnce(&mut Backup) -> Result<(), TrError>,
    ) -> Result<(), TrError> {
        let mut state = self.state.write().expect("backup lock poisoned");
        let State { backup, journal } = &mut *state;

        // The header travels with every edit. Today the header CRC covers only
        // the header, so a record edit leaves it untouched — but that is a
        // property of the CRC's current scope, not a guarantee, and 64 bytes is
        // not worth being clever about.
        let windows = [0..HEADER_LEN, window(backup)];

        let snapshots = journal::snapshot(backup.raw(), &windows);
        let result = f(backup);
        // Diff even on failure: a setter that refused should have changed
        // nothing, and an empty diff records nothing — so this both stays
        // correct and quietly catches a setter that half-applied before
        // erroring, rather than leaving those bytes un-undoable.
        journal.record_diff(backup.raw(), kind, &snapshots);
        result
    }
}

#[uniffi::export]
impl BackupFile {
    /// Parse a backup from bytes already in memory (a document handed over by
    /// the file picker, a SysEx dump, an iCloud read).
    #[uniffi::constructor]
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Arc<Self>, TrError> {
        let backup = Backup::parse(bytes).map_err(|e| TrError::Parse {
            message: e.to_string(),
        })?;
        Ok(Self::new(backup, None))
    }

    /// Read and parse a backup from a filesystem path.
    #[uniffi::constructor]
    pub fn open(path: String) -> Result<Arc<Self>, TrError> {
        let bytes = std::fs::read(&path).map_err(|e| TrError::Io {
            path: path.clone(),
            message: e.to_string(),
        })?;
        let this = Self::from_bytes(bytes)?;
        *this.path.write().expect("path lock poisoned") = Some(path);
        Ok(this)
    }

    /// The path this was opened from, if any.
    pub fn source_path(&self) -> Option<String> {
        self.path.read().expect("path lock poisoned").clone()
    }

    /// Header, section directory, and slot counts.
    pub fn info(&self) -> BackupInfo {
        self.read(|b| BackupInfo {
            magic: String::from_utf8_lossy(&b.magic()).to_string(),
            version: b.version(),
            header_crc_valid: b.header_crc_valid(),
            kit_count: b.kits().len() as u32,
            pattern_count: b.patterns().len() as u32,
            byte_len: b.raw().len() as u64,
            sections: b
                .sections()
                .iter()
                .map(|s| SectionInfo {
                    tag: s.tag_str(),
                    payload_offset: s.payload_offset as u64,
                    payload_len: s.payload_len as u64,
                })
                .collect(),
        })
    }

    /// Every kit slot, with voices and resolved tone names.
    pub fn kits(&self) -> Vec<KitInfo> {
        self.read(|b| b.kits().iter().map(|k| kit_info(b, k)).collect())
    }

    /// One kit slot.
    pub fn kit(&self, index: u32) -> Result<KitInfo, TrError> {
        self.read(|b| {
            let kits = b.kits();
            let k = kits.get(index as usize).ok_or_else(|| TrError::OutOfRange {
                what: "kit".into(),
                index,
                count: kits.len() as u32,
            })?;
            Ok(kit_info(b, k))
        })
    }

    /// Every pattern slot's header fields (no step data — see [`Self::grid`]).
    pub fn patterns(&self) -> Vec<PatternInfo> {
        self.read(|b| b.patterns().iter().map(|p| pattern_info(b, p)).collect())
    }

    /// One pattern's header fields.
    pub fn pattern(&self, index: u32) -> Result<PatternInfo, TrError> {
        self.read(|b| {
            let pats = b.patterns();
            let p = pats.get(index as usize).ok_or_else(|| TrError::OutOfRange {
                what: "pattern".into(),
                index,
                count: pats.len() as u32,
            })?;
            Ok(pattern_info(b, p))
        })
    }

    /// The step grid for one variation of one pattern.
    pub fn grid(&self, pattern_index: u32, variation: u32) -> Result<GridInfo, TrError> {
        if variation as usize >= PATTERN_VARIATIONS {
            return Err(TrError::OutOfRange {
                what: "variation".into(),
                index: variation,
                count: PATTERN_VARIATIONS as u32,
            });
        }
        self.read(|b| {
            let pats = b.patterns();
            let count = pats.len() as u32;
            let p = pats
                .get(pattern_index as usize)
                .ok_or_else(|| TrError::OutOfRange {
                    what: "pattern".into(),
                    index: pattern_index,
                    count,
                })?;
            let grid = StepGrid::read(b.raw(), p, variation as usize);
            let voice_tracks = voice_track_count(b);
            // Row colours come from the kit this pattern plays, so the grid
            // lights up the way the box will. `kit_ref` is stored 1-based.
            let kit_ref = p.kit_ref(b.raw());
            let colors = (kit_ref > 0)
                .then(|| b.kits().get(kit_ref as usize - 1).map(|k| k.slider_colors(b.raw(), voice_tracks)))
                .flatten()
                .unwrap_or_default();
            Ok(GridInfo {
                pattern_index,
                variation,
                variation_label: variation_label(variation),
                tracks: (0..PATTERN_STEP_TRACKS)
                    .map(|t| TrackInfo {
                        name: track_name(b, t),
                        is_voice: t < voice_tracks,
                        color: colors.get(t).copied(),
                        steps: (0..PATTERN_STEPS_PER_TRACK)
                            .map(|s| {
                                let w = grid.word(t, s);
                                StepInfo {
                                    on: w.is_on(),
                                    velocity: w.velocity(),
                                    sub_step: w.sub_step().map(|x| x.label().to_string()),
                                    alternate: w.is_alternate(),
                                }
                            })
                            .collect(),
                    })
                    .collect(),
            })
        })
    }

    // -- edits --------------------------------------------------------------
    //
    // Each of these mutates the byte buffer in place and then fixes up the
    // header CRC, so the handle is always in a state that could be written to
    // disk. Doing it per-edit rather than at save time means a caller can never
    // produce a file the hardware rejects by forgetting a step.
    //
    // Each also runs inside [`journal_edit`], which records the bytes that
    // moved so the edit can be undone. Adding an edit method here means giving
    // it a window: anything it changes outside that window is invisible to undo
    // (see [`journal`]).

    /// Rename a kit slot (16 chars, space-padded on write).
    pub fn set_kit_name(&self, index: u32, name: String) -> Result<(), TrError> {
        self.journal_edit(
            EditKind::KitName { index },
            |b| kit_window(b, index),
            move |b| {
                let kit = nth_kit(b, index)?;
                let ok = kit.set_name(b.raw_mut(), &name);
                finish_edit(b, ok, "kit name")
            },
        )
    }

    /// Rename a pattern slot.
    pub fn set_pattern_name(&self, index: u32, name: String) -> Result<(), TrError> {
        self.journal_edit(
            EditKind::PatternName { index },
            |b| pattern_window(b, index),
            move |b| {
                let pat = nth_pattern(b, index)?;
                let ok = pat.set_name(b.raw_mut(), &name);
                finish_edit(b, ok, "pattern name")
            },
        )
    }

    /// Set a pattern's tempo. Clamped by the format layer to 40.0–300.0 BPM.
    pub fn set_pattern_tempo(&self, index: u32, bpm: f32) -> Result<(), TrError> {
        self.journal_edit(
            EditKind::PatternTempo { index },
            |b| pattern_window(b, index),
            move |b| {
                let pat = nth_pattern(b, index)?;
                let ok = pat.set_tempo_bpm(b.raw_mut(), bpm);
                finish_edit(b, ok, "pattern tempo")
            },
        )
    }

    /// Point a pattern at a different kit slot (1-based, 1–128 — the stored
    /// convention, matching [`PatternInfo::kit_ref`]).
    pub fn set_pattern_kit_ref(&self, index: u32, kit: u8) -> Result<(), TrError> {
        self.journal_edit(
            EditKind::PatternKitRef { index },
            |b| pattern_window(b, index),
            move |b| {
                let pat = nth_pattern(b, index)?;
                let ok = pat.set_kit_ref(b.raw_mut(), kit);
                finish_edit(b, ok, "pattern kit reference")
            },
        )
    }

    /// Set one step's velocity; 0 turns the step off.
    pub fn set_step(
        &self,
        pattern_index: u32,
        variation: u32,
        track: u32,
        step: u32,
        velocity: u8,
    ) -> Result<(), TrError> {
        self.journal_edit(
            EditKind::Steps {
                pattern: pattern_index,
                variation,
            },
            |b| pattern_window(b, pattern_index),
            move |b| {
                let pat = nth_pattern(b, pattern_index)?;
                check_variation(variation)?;
                let mut grid = StepGrid::read(b.raw(), &pat, variation as usize);
                grid.set(track as usize, step as usize, velocity);
                grid.apply(b.raw_mut(), &pat);
                b.recompute_header_crc();
                Ok(())
            },
        )
    }

    /// Flip one step on/off, preserving velocity where the format layer does.
    pub fn toggle_step(
        &self,
        pattern_index: u32,
        variation: u32,
        track: u32,
        step: u32,
    ) -> Result<(), TrError> {
        self.journal_edit(
            EditKind::Steps {
                pattern: pattern_index,
                variation,
            },
            |b| pattern_window(b, pattern_index),
            move |b| {
                let pat = nth_pattern(b, pattern_index)?;
                check_variation(variation)?;
                let mut grid = StepGrid::read(b.raw(), &pat, variation as usize);
                grid.toggle(track as usize, step as usize);
                grid.apply(b.raw_mut(), &pat);
                b.recompute_header_crc();
                Ok(())
            },
        )
    }

    /// Set a voice's LED colour on a kit — the colour its fader and pad light up
    /// as on the hardware. `color` indexes the twelve panel colours.
    pub fn set_voice_color(&self, kit_index: u32, voice: u32, color: u8) -> Result<(), TrError> {
        self.journal_edit(
            EditKind::VoiceColor {
                kit: kit_index,
                voice,
            },
            |b| kit_window(b, kit_index),
            move |b| {
                let kit = nth_kit(b, kit_index)?;
                let ok = kit.set_slider_color(b.raw_mut(), voice as usize, color);
                finish_edit(b, ok, "voice colour")
            },
        )
    }

    // -- undo / redo --------------------------------------------------------

    /// Begin a gesture: edits until the matching [`Self::commit_transaction`]
    /// collapse into a single undo step.
    ///
    /// Nested calls deepen the same transaction and keep the outer `label`, so
    /// a gesture that internally calls several labelled edits still reads as
    /// the gesture. Drive these from a scoped helper rather than by hand — an
    /// unmatched `begin` swallows every later edit into the wrong entry.
    pub fn begin_transaction(&self, label: String) {
        self.with_journal(|j| j.begin(label));
    }

    /// Close one level of transaction; the outermost close commits it.
    /// Committing with nothing open is a harmless no-op.
    pub fn commit_transaction(&self) {
        self.with_journal(|j| j.commit());
    }

    /// Reverse the most recent edit. Returns its label, or `None` if there was
    /// nothing to undo.
    pub fn undo(&self) -> Option<String> {
        let mut state = self.state.write().expect("backup lock poisoned");
        let State { backup, journal } = &mut *state;
        journal.undo(backup.raw_mut())
    }

    /// Reapply the most recently undone edit. Returns its label.
    pub fn redo(&self) -> Option<String> {
        let mut state = self.state.write().expect("backup lock poisoned");
        let State { backup, journal } = &mut *state;
        journal.redo(backup.raw_mut())
    }

    pub fn can_undo(&self) -> bool {
        self.with_journal(|j| j.can_undo())
    }

    pub fn can_redo(&self) -> bool {
        self.with_journal(|j| j.can_redo())
    }

    /// Label of the edit `undo` would reverse — for the Edit menu item title.
    pub fn undo_label(&self) -> Option<String> {
        self.with_journal(|j| j.undo_label())
    }

    /// Label of the edit `redo` would reapply.
    pub fn redo_label(&self) -> Option<String> {
        self.with_journal(|j| j.redo_label())
    }

    /// Applied edits, oldest first.
    pub fn history(&self) -> Vec<String> {
        self.with_journal(|j| j.history())
    }

    /// Whether the buffer differs from what was last written to disk.
    pub fn is_dirty(&self) -> bool {
        self.with_journal(|j| j.is_dirty())
    }

    // -- output -------------------------------------------------------------

    /// The full container as bytes, ready to write or send.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.read(|b| b.to_bytes())
    }

    /// Write the container to `path`.
    pub fn save(&self, path: String) -> Result<(), TrError> {
        let bytes = self.to_bytes();
        std::fs::write(&path, bytes).map_err(|e| TrError::Io {
            path: path.clone(),
            message: e.to_string(),
        })?;
        *self.path.write().expect("path lock poisoned") = Some(path);
        // Remember where in the history this file on disk corresponds to, so
        // undoing back to here reports the document as clean again.
        self.with_journal(|j| j.mark_saved());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The number of leading step tracks that actually make sound on this device.
/// A TR-6S stores its six voices in slots 0–5 and leaves 6–10 empty; a TR-8S
/// uses all eleven. See `tr_format::INST_TRACKS`.
fn voice_track_count(b: &Backup) -> usize {
    b.voice_names().len()
}

/// Panel name for a step-track slot, picking the TR-6S or TR-8S layout.
fn track_name(b: &Backup, track: usize) -> String {
    match tr_format::track_role(track) {
        Some(TrackRole::Trigger) => "TRIG".to_string(),
        // The container's own layout picks the names: a TR-6S uses slots 0–5
        // for its six voices and leaves 6–10 empty, while a TR-8S or a `.t8p`
        // pack uses all eleven.
        Some(TrackRole::Inst(i)) => b
            .voice_names()
            .get(i)
            .copied()
            .unwrap_or("--")
            .to_string(),
        _ => format!("slot {track}"),
    }
}

fn kit_info(b: &Backup, k: &Kit) -> KitInfo {
    let raw = b.raw();
    let names = b.voice_names();
    let params = k.voices_n(raw, names.len());
    let colors = k.slider_colors(raw, names.len());
    KitInfo {
        index: k.index as u32,
        name: k.name(raw),
        voices: params
            .iter()
            .enumerate()
            .map(|(i, v)| VoiceInfo {
                name: names.get(i).copied().unwrap_or("--").to_string(),
                tone_id: v.tone,
                tone_name: b.tone_name(v.tone).unwrap_or_default(),
                color: colors.get(i).copied().unwrap_or(0),
                color_name: colors
                    .get(i)
                    .and_then(|c| SLIDER_COLORS.get(*c as usize))
                    .copied()
                    .unwrap_or("")
                    .to_string(),
                tune: v.tune,
                decay: v.decay,
                level: v.level,
                gain: v.gain,
                pan: v.pan,
                reverb_send: v.reverb_send,
                delay_send: v.delay_send,
            })
            .collect(),
    }
}

fn pattern_info(b: &Backup, p: &Pattern) -> PatternInfo {
    let raw = b.raw();
    PatternInfo {
        index: p.index as u32,
        name: p.name(raw),
        tempo_bpm: p.tempo_bpm(raw),
        kit_ref: p.kit_ref(raw),
    }
}

/// Look up the nth kit cursor, or an `OutOfRange` naming what was asked for.
fn nth_kit(b: &Backup, index: u32) -> Result<Kit, TrError> {
    let kits = b.kits();
    kits.get(index as usize)
        .copied()
        .ok_or_else(|| TrError::OutOfRange {
            what: "kit".into(),
            index,
            count: kits.len() as u32,
        })
}

fn nth_pattern(b: &Backup, index: u32) -> Result<Pattern, TrError> {
    let pats = b.patterns();
    pats.get(index as usize)
        .copied()
        .ok_or_else(|| TrError::OutOfRange {
            what: "pattern".into(),
            index,
            count: pats.len() as u32,
        })
}

/// The byte range of a kit record, or an empty range if the slot is past the
/// end. Used as an edit's journal window.
fn kit_window(b: &Backup, index: u32) -> Range<usize> {
    match nth_kit(b, index) {
        Ok(k) => k.offset..(k.offset + KIT_RECORD_SIZE).min(b.raw().len()),
        Err(_) => 0..0,
    }
}

/// The byte range of a pattern record, or an empty range if out of bounds.
fn pattern_window(b: &Backup, index: u32) -> Range<usize> {
    match nth_pattern(b, index) {
        Ok(p) => p.offset..(p.offset + PATTERN_RECORD_SIZE).min(b.raw().len()),
        Err(_) => 0..0,
    }
}

fn check_variation(variation: u32) -> Result<(), TrError> {
    if variation as usize >= PATTERN_VARIATIONS {
        return Err(TrError::OutOfRange {
            what: "variation".into(),
            index: variation,
            count: PATTERN_VARIATIONS as u32,
        });
    }
    Ok(())
}

/// Turn a format-layer `bool` setter result into a `Result`, and keep the
/// header CRC honest after a successful write.
fn finish_edit(b: &mut Backup, ok: bool, what: &str) -> Result<(), TrError> {
    if !ok {
        return Err(TrError::Write {
            message: format!("{what} write was rejected (offset out of range)"),
        });
    }
    b.recompute_header_crc();
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
//
// These exercise the boundary shape, not the format — `tr-format` already
// tests offsets and masks. What is worth checking here is that the DTO
// conversions and the 0-based indexing contract hold, and that an edit leaves
// the container valid.

#[cfg(test)]
mod tests {
    use super::*;
    use tr_format::VOICES;

    /// The reference TR-6S backup, if the working copy has one. Not committed
    /// (no Roland data in the repo), so these tests skip when it is absent.
    fn fixture() -> Option<Arc<BackupFile>> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../roland_backup_dump/TR-6S/BACKUP/tr6s_bak.bin"
        );
        std::path::Path::new(path).exists().then(|| {
            BackupFile::open(path.to_string()).expect("reference backup should parse")
        })
    }

    /// A Roland Cloud `.t8p` pattern pack, if the working copy has one.
    ///
    /// These are TR-8S content and the only TR-8S data available without the
    /// hardware, so they are what exercises the eleven-voice layout. Not
    /// committed (no Roland data in the repo), so these tests skip when absent —
    /// extract with
    /// `unzip -j tr8s_kits_vol1/*.zip '*/PATTERN/*' -d tr8s_kits_vol1/extracted`.
    fn t8p_fixture() -> Option<Arc<BackupFile>> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tr8s_kits_vol1/extracted/Day&Night.t8p"
        );
        std::path::Path::new(path)
            .exists()
            .then(|| BackupFile::open(path.to_string()).expect("t8p pack should parse"))
    }

    #[test]
    fn a_t8p_pack_parses_as_a_container() {
        let Some(b) = t8p_fixture() else { return };
        let info = b.info();
        assert_eq!(info.magic, "T8P ");
        assert_eq!(info.version, 3, "pattern pack");
        assert_eq!(info.pattern_count, 2);
        assert_eq!(info.kit_count, 2);
    }

    /// Ground truth is the sidecar manifest Roland ships beside the pack:
    ///
    /// ```text
    /// 1 = "Brisk Afternoon " ; ABCDEFGH 2 Kit 1 Tempo 125.0 Motion ON
    /// 2 = "Uncertain Night " ; AB------ S Kit 2 Tempo 120.0 Motion ON
    /// ```
    #[test]
    fn pack_patterns_match_rolands_manifest() {
        let Some(b) = t8p_fixture() else { return };
        let patterns = b.patterns();

        assert_eq!(patterns[0].name, "Brisk Afternoon");
        assert_eq!(patterns[0].tempo_bpm, 125.0);
        assert_eq!(patterns[0].kit_ref, 1);

        assert_eq!(patterns[1].name, "Uncertain Night");
        assert_eq!(patterns[1].tempo_bpm, 120.0);
        assert_eq!(patterns[1].kit_ref, 2);
    }

    /// The eleven-voice TR-8S layout — the branch a TR-6S backup can never
    /// reach. The manifest lists kit 1 as:
    ///
    /// ```text
    /// BD "909 Bass2"     SD "OSC Tri1 Low"   LT/MT/HT/RS "E.Piano Mid" (x4)
    /// HC "909 Hand Clap" CH "909 Closed HH"  OH "909 Open HH"
    /// CC "909 Crash Cymbal"                  RC "Ouh"
    /// ```
    /// The LED colours are real per-kit data, not a fixed ramp: the reference
    /// backup holds 36 distinct schemes across its 128 kits, and every value in
    /// all of them is inside the twelve the panel offers.
    #[test]
    fn slider_colours_are_read_and_stay_in_the_panel_palette() {
        let Some(b) = fixture() else { return };

        let kits = b.kits();
        for kit in &kits {
            for voice in &kit.voices {
                assert!(
                    (voice.color as usize) < 12,
                    "kit {} voice {} colour {} is outside the panel palette",
                    kit.index,
                    voice.name,
                    voice.color
                );
                assert!(!voice.color_name.is_empty());
            }
        }

        // Not every kit the same — otherwise this would be reading a constant.
        let schemes: std::collections::HashSet<Vec<u8>> = kits
            .iter()
            .map(|k| k.voices.iter().map(|v| v.color).collect())
            .collect();
        assert!(schemes.len() > 1, "expected per-kit colour schemes");
    }

    #[test]
    fn the_grid_takes_its_colours_from_the_kit_the_pattern_plays() {
        let Some(b) = fixture() else { return };

        let pattern = b.pattern(0).unwrap();
        let kit = b.kit(u32::from(pattern.kit_ref) - 1).unwrap();
        let grid = b.grid(0, 0).unwrap();

        for (track, voice) in grid.tracks.iter().zip(kit.voices.iter()) {
            assert_eq!(track.color, Some(voice.color), "track {} ", track.name);
        }
        // The trigger row is not a voice and has no LED colour of its own.
        assert_eq!(grid.tracks.last().unwrap().color, None);
    }

    #[test]
    fn a_voice_colour_edit_round_trips_and_undoes() {
        let Some(b) = fixture() else { return };
        let before = b.kit(0).unwrap().voices[0].color;
        let wanted = if before == 4 { 7 } else { 4 };

        b.set_voice_color(0, 0, wanted).unwrap();
        let after = b.kit(0).unwrap();
        assert_eq!(after.voices[0].color, wanted);
        assert_eq!(after.voices[0].color_name, tr_format::SLIDER_COLORS[wanted as usize]);
        assert_eq!(b.undo_label().as_deref(), Some("Change Voice Colour"));

        b.undo();
        assert_eq!(b.kit(0).unwrap().voices[0].color, before);
    }

    /// The panel offers twelve colours; writing anything else would invent a
    /// state the hardware never produces.
    #[test]
    fn an_out_of_palette_colour_is_rejected() {
        let Some(b) = fixture() else { return };
        assert!(b.set_voice_color(0, 0, 12).is_err());
        assert!(!b.can_undo(), "a rejected edit is not an undo step");
    }

    #[test]
    fn pack_kits_expose_all_eleven_tr8s_voices() {
        let Some(b) = t8p_fixture() else { return };
        let kit = b.kit(0).unwrap();

        assert_eq!(kit.name, "Brisk Afternoon");
        assert_eq!(kit.voices.len(), 11, "a .t8p is TR-8S data");
        assert_eq!(
            kit.voices.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(),
            ["BD", "SD", "LT", "MT", "HT", "RS", "HC", "CH", "OH", "CC", "RC"],
            "slots 3-5 are MT/HT/RS on a TR-8S, not HC/CH/OH"
        );

        // The manifest's four consecutive "E.Piano Mid" voices must read as one
        // repeated tone id — the check that the voices are not merely *named*
        // right but read from the correct blocks.
        let epiano: Vec<u16> = kit.voices[2..6].iter().map(|v| v.tone_id).collect();
        assert_eq!(epiano[0], epiano[1]);
        assert_eq!(epiano[1], epiano[2]);
        assert_eq!(epiano[2], epiano[3]);

        // ...and the 909 clap/hat/crash run is four consecutive ids.
        let nine_o_nine: Vec<u16> = kit.voices[6..10].iter().map(|v| v.tone_id).collect();
        assert_eq!(
            nine_o_nine[1],
            nine_o_nine[0] + 1,
            "HC/CH/OH/CC are consecutive tones"
        );
        assert_eq!(nine_o_nine[2], nine_o_nine[1] + 1);
        assert_eq!(nine_o_nine[3], nine_o_nine[2] + 1);
    }

    /// The step grid must follow the same layout, or a pack's tracks would be
    /// labelled with TR-6S names.
    #[test]
    fn pack_grids_use_the_tr8s_track_layout() {
        let Some(b) = t8p_fixture() else { return };
        let grid = b.grid(0, 0).unwrap();

        assert_eq!(grid.tracks.iter().filter(|t| t.is_voice).count(), 11);
        assert_eq!(grid.tracks[3].name, "MT");
        assert_eq!(grid.tracks[10].name, "RC");
        assert_eq!(grid.tracks[11].name, "TRIG");
    }

    /// Editing a pack has to work like editing a backup — same journal, same
    /// byte-exact undo.
    #[test]
    fn a_pack_edits_and_undoes_like_a_backup() {
        let Some(b) = t8p_fixture() else { return };
        let original = b.to_bytes();

        b.set_pattern_name(0, "PACKED".into()).unwrap();
        assert_eq!(b.pattern(0).unwrap().name, "PACKED");

        b.undo();
        assert_eq!(b.to_bytes(), original);
    }

    #[test]
    fn variation_labels_cover_the_range() {
        assert_eq!(variation_label(0), "A");
        assert_eq!(variation_label(7), "H");
        assert_eq!(variation_label(8), "Fill 1");
        assert_eq!(variation_label(9), "Fill 2");
    }

    #[test]
    fn bad_bytes_are_a_parse_error_not_a_panic() {
        let err = BackupFile::from_bytes(vec![0u8; 8]).unwrap_err();
        assert!(matches!(err, TrError::Parse { .. }), "got {err:?}");
    }

    #[test]
    fn reads_the_reference_backup() {
        let Some(b) = fixture() else { return };
        let info = b.info();
        assert_eq!(info.magic, "TR6S");
        assert!(info.kit_count > 0 && info.pattern_count > 0);
        assert!(info.header_crc_valid, "reference backup should verify");

        // 0-based indexing: slot 0 is the first record, and the record's own
        // `index` field agrees with the position we asked for.
        let first = b.pattern(0).expect("pattern 0 exists");
        assert_eq!(first.index, 0);
        assert_eq!(b.patterns()[0].name, first.name);
    }

    #[test]
    fn grid_has_the_expected_shape() {
        let Some(b) = fixture() else { return };
        let g = b.grid(0, 0).expect("pattern 0 variation A");
        assert_eq!(g.tracks.len(), PATTERN_STEP_TRACKS);
        assert_eq!(g.variation_label, "A");
        for t in &g.tracks {
            assert_eq!(t.steps.len(), PATTERN_STEPS_PER_TRACK);
        }
        // A TR-6S exposes six audible voices; the rest are unused slots + TRIG.
        assert_eq!(g.tracks.iter().filter(|t| t.is_voice).count(), VOICES.len());
        assert_eq!(g.tracks[0].name, "BD");
    }

    #[test]
    fn out_of_range_is_reported_with_the_real_count() {
        let Some(b) = fixture() else { return };
        let count = b.info().pattern_count;
        match b.pattern(count + 5) {
            Err(TrError::OutOfRange { what, count: c, .. }) => {
                assert_eq!(what, "pattern");
                assert_eq!(c, count);
            }
            other => panic!("expected OutOfRange, got {other:?}"),
        }
        assert!(b.grid(0, PATTERN_VARIATIONS as u32).is_err());
    }

    #[test]
    fn renaming_a_pattern_round_trips_and_keeps_the_crc_valid() {
        let Some(b) = fixture() else { return };
        let original = b.pattern(0).unwrap().name;

        b.set_pattern_name(0, "GROOVEBANK".into()).unwrap();
        assert_eq!(b.pattern(0).unwrap().name, "GROOVEBANK");
        assert!(b.info().header_crc_valid, "edit must fix up the header CRC");

        // The edited buffer is itself a parseable container.
        let reparsed = BackupFile::from_bytes(b.to_bytes()).unwrap();
        assert_eq!(reparsed.pattern(0).unwrap().name, "GROOVEBANK");

        b.set_pattern_name(0, original.clone()).unwrap();
        assert_eq!(b.pattern(0).unwrap().name, original);
    }

    // -- undo / redo, against the real container ---------------------------

    #[test]
    fn undo_restores_the_container_byte_for_byte() {
        let Some(b) = fixture() else { return };
        let original = b.to_bytes();

        b.set_pattern_name(0, "GROOVEBANK".into()).unwrap();
        b.set_pattern_tempo(3, 128.0).unwrap();
        b.toggle_step(0, 0, 0, 0).unwrap();
        assert_ne!(b.to_bytes(), original);

        while b.can_undo() {
            b.undo();
        }
        // Not "the fields read back the same" — the whole 56.9 MB buffer.
        assert_eq!(b.to_bytes(), original, "undo must be byte-exact");
    }

    #[test]
    fn redo_reapplies_what_undo_reversed() {
        let Some(b) = fixture() else { return };
        let original = b.to_bytes();

        b.set_pattern_name(0, "REDO ME".into()).unwrap();
        let edited = b.to_bytes();

        assert_eq!(b.undo().as_deref(), Some("Rename Pattern"));
        assert_eq!(b.to_bytes(), original);

        assert_eq!(b.redo().as_deref(), Some("Rename Pattern"));
        assert_eq!(b.to_bytes(), edited);

        while b.can_undo() {
            b.undo();
        }
    }

    /// The invariant the whole byte-patch design rests on: no edit may ever
    /// change the container's length. If this fails, every recorded patch is
    /// silently addressing the wrong bytes.
    #[test]
    fn no_edit_changes_the_container_length() {
        let Some(b) = fixture() else { return };
        let len = b.to_bytes().len();

        b.set_pattern_name(0, "LEN".into()).unwrap();
        b.set_pattern_tempo(0, 90.0).unwrap();
        b.set_pattern_kit_ref(0, 7).unwrap();
        b.set_kit_name(0, "LEN".into()).unwrap();
        b.set_step(0, 0, 0, 0, 100).unwrap();
        b.toggle_step(0, 1, 2, 3).unwrap();
        assert_eq!(b.to_bytes().len(), len);

        while b.can_undo() {
            b.undo();
            assert_eq!(b.to_bytes().len(), len);
        }
    }

    #[test]
    fn a_gesture_is_a_single_undo_step() {
        let Some(b) = fixture() else { return };
        let original = b.to_bytes();

        b.begin_transaction("Paint Steps".into());
        for step in 0..12 {
            b.toggle_step(0, 0, 4, step).unwrap();
        }
        b.commit_transaction();

        assert_eq!(b.history(), vec!["Paint Steps"]);
        assert_eq!(b.undo_label().as_deref(), Some("Paint Steps"));

        b.undo();
        assert_eq!(b.to_bytes(), original, "the whole gesture reverses at once");
        assert!(!b.can_undo());
    }

    #[test]
    fn labels_name_the_edit_for_the_menu() {
        let Some(b) = fixture() else { return };

        b.set_kit_name(0, "LABELLED".into()).unwrap();
        assert_eq!(b.undo_label().as_deref(), Some("Rename Kit"));
        b.undo();
        assert_eq!(b.redo_label().as_deref(), Some("Rename Kit"));
        b.redo();

        b.set_pattern_tempo(0, 101.0).unwrap();
        assert_eq!(b.undo_label().as_deref(), Some("Change Tempo"));

        while b.can_undo() {
            b.undo();
        }
    }

    #[test]
    fn an_edit_that_changes_nothing_is_not_an_undo_step() {
        let Some(b) = fixture() else { return };
        let name = b.pattern(0).unwrap().name;

        b.set_pattern_name(0, name).unwrap();
        assert!(!b.can_undo(), "renaming to the same name is not an edit");
    }

    #[test]
    fn a_rejected_edit_leaves_no_undo_step() {
        let Some(b) = fixture() else { return };
        assert!(b.set_pattern_name(99_999, "NOPE".into()).is_err());
        assert!(!b.can_undo());
    }

    #[test]
    fn undoing_back_to_the_saved_point_clears_dirty() {
        let Some(b) = fixture() else { return };
        let dir = std::env::temp_dir().join("groovebank-dirty-test.bin");

        b.save(dir.to_string_lossy().to_string()).unwrap();
        assert!(!b.is_dirty());

        b.set_pattern_name(0, "DIRTY".into()).unwrap();
        assert!(b.is_dirty());

        b.undo();
        assert!(!b.is_dirty(), "back at the saved state, so not modified");

        b.redo();
        assert!(b.is_dirty());

        while b.can_undo() {
            b.undo();
        }
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn toggling_a_step_flips_exactly_that_step() {
        let Some(b) = fixture() else { return };
        let before = b.grid(0, 0).unwrap();
        let was_on = before.tracks[0].steps[0].on;

        b.toggle_step(0, 0, 0, 0).unwrap();
        let after = b.grid(0, 0).unwrap();
        assert_ne!(after.tracks[0].steps[0].on, was_on);
        assert_eq!(after.tracks[0].steps[1].on, before.tracks[0].steps[1].on);
        assert_eq!(after.tracks[1].steps[0].on, before.tracks[1].steps[0].on);

        b.toggle_step(0, 0, 0, 0).unwrap();
        assert_eq!(b.grid(0, 0).unwrap().tracks[0].steps[0].on, was_on);
    }
}
