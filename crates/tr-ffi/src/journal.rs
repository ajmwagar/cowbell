//! Session-wide undo/redo: a journal of the bytes each edit changed.
//!
//! ## Why bytes rather than commands
//!
//! Measured against the reference TR-6S backup, a rename changes **10 bytes**
//! and a step toggle changes **1** — in a single contiguous run, inside a
//! 56.9 MB container. So recording what actually moved is nearly free, while
//! snapshotting the buffer would cost 56.9 MB per undo level.
//!
//! The alternative was typed, invertible command objects. They lose here as the
//! *mechanism*: every edit method would need a hand-written inverse, and an
//! inverse that drifts from its `apply` is a bug you find months later with a
//! corrupted file. A byte patch cannot drift — undo is "write the bytes back",
//! and it is the same three lines no matter what the edit was. What commands
//! were genuinely better at, naming the edit, is recovered by carrying an
//! [`EditKind`] and a label alongside the bytes.
//!
//! ## The invariant this rests on
//!
//! **The container never changes length**, so a byte offset recorded today is
//! still that byte tomorrow. Two things enforce it, and neither is a comment:
//! every edit reaches the buffer through `Backup::raw_mut`, which hands out a
//! `&mut [u8]` and therefore *cannot* resize it; and the one API that replaces
//! bulk content, `Backup::replace_payload`, rejects a length change outright.
//!
//! Every patch here becomes invalid the moment that stops holding, so
//! `BackupFile`'s tests assert the container's length across the whole public
//! edit surface rather than assuming it.

use std::ops::Range;

// ---------------------------------------------------------------------------
// Value types
// ---------------------------------------------------------------------------

/// A contiguous run of bytes an edit changed, and what was there before.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct EditSpan {
    pub offset: u64,
    /// The bytes before the edit — what `undo` writes back.
    pub was: Vec<u8>,
    /// The bytes after the edit — what `redo` writes back.
    pub now: Vec<u8>,
}

/// What an edit was *about*.
///
/// This is descriptive only: undo and redo never consult it. It exists so the
/// app can name an entry in the Edit menu or a history list without the core
/// having to know how slots are displayed (patterns are shown bank-addressed,
/// `1-01`, which is the app's business — see the crate docs on indexing).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum EditKind {
    PatternName { index: u32 },
    PatternTempo { index: u32 },
    PatternKitRef { index: u32 },
    KitName { index: u32 },
    VoiceColor { kit: u32, voice: u32 },
    Steps { pattern: u32, variation: u32 },
    /// A transaction that spanned more than one kind of edit.
    Composite,
}

impl EditKind {
    /// A plain verb phrase, with no slot numbering in it.
    pub fn default_label(&self) -> String {
        match self {
            EditKind::PatternName { .. } => "Rename Pattern",
            EditKind::PatternTempo { .. } => "Change Tempo",
            EditKind::PatternKitRef { .. } => "Change Kit Reference",
            EditKind::KitName { .. } => "Rename Kit",
            EditKind::VoiceColor { .. } => "Change Voice Colour",
            EditKind::Steps { .. } => "Edit Steps",
            EditKind::Composite => "Edit",
        }
        .to_string()
    }
}

/// One undoable step.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct EditRecord {
    /// Human-readable, for "Undo <label>".
    pub label: String,
    pub kind: EditKind,
    pub spans: Vec<EditSpan>,
}

impl EditRecord {
    fn byte_cost(&self) -> usize {
        self.spans.iter().map(|s| s.was.len() + s.now.len()).sum()
    }
}

// ---------------------------------------------------------------------------
// Limits
// ---------------------------------------------------------------------------

/// Maximum entries kept before the oldest is dropped.
pub const MAX_ENTRIES: usize = 500;

/// Maximum stored patch bytes before the oldest entries are dropped.
///
/// A single edit is a handful of bytes, so this is really a bound on the
/// expensive case: a morpher run that rewrites whole pattern records (24,504
/// bytes each) would otherwise accumulate without limit.
pub const MAX_BYTES: usize = 16 * 1024 * 1024;

/// Runs closer together than this are merged into one span. A step-grid write
/// touches scattered bytes inside one record; merging small gaps keeps that
/// from becoming dozens of tiny spans, at the cost of storing a few unchanged
/// bytes between them.
const MERGE_GAP: usize = 16;

// ---------------------------------------------------------------------------
// The journal
// ---------------------------------------------------------------------------

/// An in-progress transaction: edits made while one is open coalesce into it.
#[derive(Debug)]
struct OpenTransaction {
    label: String,
    kind: Option<EditKind>,
    spans: Vec<EditSpan>,
}

#[derive(Debug, Default)]
pub struct Journal {
    /// Applied edits, oldest first. The top is what `undo` reverses.
    done: Vec<EditRecord>,
    /// Edits reversed by `undo`, most recent first.
    undone: Vec<EditRecord>,
    open: Option<OpenTransaction>,
    /// Nesting depth of `begin_transaction` calls.
    depth: u32,
    /// `done.len()` at the last save, if the saved point is still in range.
    saved_at: Option<usize>,
    /// Running total of `done`'s patch bytes.
    bytes: usize,
}

impl Journal {
    /// Open a transaction, or deepen an already-open one.
    ///
    /// Nested calls keep the **outer** label: a gesture that internally performs
    /// several labelled edits should still read as the gesture.
    pub fn begin(&mut self, label: String) {
        self.depth += 1;
        if self.open.is_none() {
            self.open = Some(OpenTransaction {
                label,
                kind: None,
                spans: Vec::new(),
            });
        }
    }

    /// Close one level of transaction; the outermost close commits.
    ///
    /// Committing with nothing open is deliberately a no-op rather than an
    /// error — an unbalanced `commit` should not be able to take down the app,
    /// and the app drives these from a scoped helper anyway.
    pub fn commit(&mut self) {
        if self.depth == 0 {
            return;
        }
        self.depth -= 1;
        if self.depth > 0 {
            return;
        }
        let Some(open) = self.open.take() else { return };
        if open.spans.is_empty() {
            return; // a transaction that changed nothing is not an undo step
        }
        let kind = open.kind.unwrap_or(EditKind::Composite);
        self.push(EditRecord {
            label: open.label,
            kind,
            spans: open.spans,
        });
    }

    /// Record what changed between `snapshots` and the buffer's current state.
    ///
    /// Snapshotting and diffing are separate calls rather than one closure
    /// because the edit in between needs the *whole* `Backup` — its record
    /// cursors and its CRC fixup — while the journal only needs the bytes. One
    /// combined call would have to hold the buffer mutably across the edit,
    /// which the borrow checker rightly refuses.
    ///
    /// A diff that finds nothing produces no undo step, so renaming something
    /// to the name it already had is not an entry.
    pub fn record_diff(
        &mut self,
        raw: &[u8],
        kind: EditKind,
        snapshots: &[(usize, Vec<u8>)],
    ) {
        let mut spans = Vec::new();
        for (start, before) in snapshots {
            spans.extend(diff_runs(*start, before, raw));
        }
        if spans.is_empty() {
            return;
        }

        match &mut self.open {
            Some(open) => {
                // Kinds only survive if the whole transaction agrees.
                open.kind = match open.kind.take() {
                    None => Some(kind),
                    Some(k) if k == kind => Some(k),
                    Some(_) => Some(EditKind::Composite),
                };
                open.spans.extend(spans);
            }
            None => {
                self.push(EditRecord {
                    label: kind.default_label(),
                    kind,
                    spans,
                });
            }
        }
    }

    /// Reverse the most recent edit, returning its label.
    pub fn undo(&mut self, raw: &mut [u8]) -> Option<String> {
        let record = self.done.pop()?;
        self.bytes = self.bytes.saturating_sub(record.byte_cost());

        // Reverse order: spans within one record may overlap if a transaction
        // touched the same bytes twice, and the earliest `was` must land last.
        for span in record.spans.iter().rev() {
            write_span(raw, span.offset, &span.was);
        }

        let label = record.label.clone();
        self.undone.push(record);
        self.clamp_saved();
        Some(label)
    }

    /// Reapply the most recently undone edit, returning its label.
    pub fn redo(&mut self, raw: &mut [u8]) -> Option<String> {
        let record = self.undone.pop()?;
        for span in &record.spans {
            write_span(raw, span.offset, &span.now);
        }
        let label = record.label.clone();
        self.bytes += record.byte_cost();
        self.done.push(record);
        Some(label)
    }

    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    pub fn undo_label(&self) -> Option<String> {
        self.done.last().map(|r| r.label.clone())
    }

    pub fn redo_label(&self) -> Option<String> {
        self.undone.last().map(|r| r.label.clone())
    }

    /// Applied edits, oldest first.
    pub fn history(&self) -> Vec<String> {
        self.done.iter().map(|r| r.label.clone()).collect()
    }

    /// Whether the buffer differs from what was last saved.
    ///
    /// Undoing back to the saved point clears this, which is what makes "undo
    /// everything I did" leave a document that is honestly unmodified.
    pub fn is_dirty(&self) -> bool {
        self.saved_at != Some(self.done.len())
    }

    pub fn mark_saved(&mut self) {
        self.saved_at = Some(self.done.len());
    }

    // -- internals ----------------------------------------------------------

    fn push(&mut self, record: EditRecord) {
        // Anything undone is unreachable once a new edit lands.
        self.undone.clear();
        self.bytes += record.byte_cost();
        self.done.push(record);
        self.trim();
    }

    /// Drop oldest entries until both bounds are satisfied.
    fn trim(&mut self) {
        while self.done.len() > MAX_ENTRIES || (self.bytes > MAX_BYTES && self.done.len() > 1) {
            let dropped = self.done.remove(0);
            self.bytes = self.bytes.saturating_sub(dropped.byte_cost());
            match self.saved_at {
                // The saved point moved down with everything else...
                Some(n) if n > 0 => self.saved_at = Some(n - 1),
                // ...unless it was the entry we just dropped, in which case the
                // document can no longer be proven clean.
                Some(_) => self.saved_at = None,
                None => {}
            }
        }
    }

    fn clamp_saved(&mut self) {
        if let Some(n) = self.saved_at {
            if n > self.done.len() + self.undone.len() {
                self.saved_at = None;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Diffing
// ---------------------------------------------------------------------------

/// Copy the byte ranges an edit is about to touch, to diff against afterwards.
///
/// Ranges are clamped to the buffer, so a caller cannot record a window past
/// the end of a short file.
pub fn snapshot(raw: &[u8], windows: &[Range<usize>]) -> Vec<(usize, Vec<u8>)> {
    windows
        .iter()
        .map(|w| {
            let clamped = w.start.min(raw.len())..w.end.min(raw.len());
            (clamped.start, raw[clamped].to_vec())
        })
        .collect()
}

/// Changed runs between `before` and the same window of `after`, merged across
/// gaps shorter than [`MERGE_GAP`].
fn diff_runs(start: usize, before: &[u8], after: &[u8]) -> Vec<EditSpan> {
    let mut spans: Vec<EditSpan> = Vec::new();
    let mut i = 0;

    while i < before.len() {
        if before[i] == after[start + i] {
            i += 1;
            continue;
        }

        let run_start = i;
        let mut run_end = i + 1;
        let mut j = run_end;
        // Extend while the next difference is close enough to be worth merging.
        while j < before.len() {
            if before[j] != after[start + j] {
                run_end = j + 1;
                j = run_end;
            } else if j - run_end < MERGE_GAP {
                j += 1;
            } else {
                break;
            }
        }

        spans.push(EditSpan {
            offset: (start + run_start) as u64,
            was: before[run_start..run_end].to_vec(),
            now: after[start + run_start..start + run_end].to_vec(),
        });
        i = run_end;
    }

    spans
}

fn write_span(raw: &mut [u8], offset: u64, bytes: &[u8]) {
    let start = offset as usize;
    let end = start + bytes.len();
    if end <= raw.len() {
        raw[start..end].copy_from_slice(bytes);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(bytes: &[u8]) -> Vec<u8> {
        bytes.to_vec()
    }

    #[test]
    fn diff_finds_a_single_run() {
        let before = buf(&[1, 2, 3, 4, 5]);
        let after = buf(&[1, 9, 9, 4, 5]);
        let spans = diff_runs(0, &before, &after);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].offset, 1);
        assert_eq!(spans[0].was, vec![2, 3]);
        assert_eq!(spans[0].now, vec![9, 9]);
    }

    #[test]
    fn diff_merges_near_runs_and_splits_far_ones() {
        // Two differences 2 bytes apart merge; a third 40 bytes away does not.
        let mut before = vec![0u8; 64];
        let mut after = before.clone();
        after[0] = 1;
        after[3] = 1;
        after[50] = 1;

        let spans = diff_runs(0, &before, &after);
        assert_eq!(spans.len(), 2, "got {spans:?}");
        assert_eq!(spans[0].offset, 0);
        assert_eq!(spans[0].now, vec![1, 0, 0, 1]);
        assert_eq!(spans[1].offset, 50);

        // And the merged span round-trips exactly.
        before[0] = 0;
        let mut restored = after.clone();
        for s in spans.iter().rev() {
            write_span(&mut restored, s.offset, &s.was);
        }
        assert_eq!(restored, before);
    }

    /// Journal an edit against a toy buffer, mirroring what `BackupFile` does:
    /// snapshot, mutate, diff.
    fn edit(j: &mut Journal, raw: &mut [u8], kind: EditKind, f: impl FnOnce(&mut [u8])) {
        let snaps = snapshot(raw, &[0..raw.len()]);
        f(raw);
        j.record_diff(raw, kind, &snaps);
    }

    #[test]
    fn undo_restores_and_redo_reapplies() {
        let mut raw = vec![0u8; 32];
        let mut j = Journal::default();
        let original = raw.clone();

        edit(&mut j, &mut raw, EditKind::KitName { index: 0 }, |b| b[4] = 7);
        let edited = raw.clone();

        assert!(j.can_undo());
        assert_eq!(j.undo_label().as_deref(), Some("Rename Kit"));

        assert_eq!(j.undo(&mut raw).as_deref(), Some("Rename Kit"));
        assert_eq!(raw, original, "undo must restore byte for byte");

        assert!(j.can_redo());
        assert_eq!(j.redo(&mut raw).as_deref(), Some("Rename Kit"));
        assert_eq!(raw, edited);
    }

    #[test]
    fn a_no_op_edit_is_not_an_undo_step() {
        let mut raw = vec![0u8; 16];
        let mut j = Journal::default();
        edit(&mut j, &mut raw, EditKind::KitName { index: 0 }, |_| {});
        assert!(!j.can_undo());
    }

    #[test]
    fn a_transaction_is_one_undo_step() {
        let mut raw = vec![0u8; 32];
        let mut j = Journal::default();
        let original = raw.clone();

        j.begin("Paint Steps".into());
        for i in 0..12 {
            edit(&mut j, &mut raw, EditKind::Steps { pattern: 0, variation: 0 }, |b| {
                b[i] = 1
            });
        }
        j.commit();

        assert_eq!(j.history(), vec!["Paint Steps"]);
        j.undo(&mut raw);
        assert_eq!(raw, original, "the whole gesture reverses at once");
    }

    #[test]
    fn nested_transactions_keep_the_outer_label() {
        let mut raw = vec![0u8; 16];
        let mut j = Journal::default();

        j.begin("Outer".into());
        j.begin("Inner".into());
        edit(&mut j, &mut raw, EditKind::KitName { index: 0 }, |b| b[0] = 1);
        j.commit(); // inner: does not commit
        assert!(!j.can_undo(), "inner commit must not close the transaction");
        j.commit(); // outer: commits

        assert_eq!(j.history(), vec!["Outer"]);
    }

    #[test]
    fn a_transaction_of_mixed_kinds_is_composite() {
        let mut raw = vec![0u8; 16];
        let mut j = Journal::default();

        j.begin("Mixed".into());
        edit(&mut j, &mut raw, EditKind::KitName { index: 0 }, |b| b[0] = 1);
        edit(&mut j, &mut raw, EditKind::PatternTempo { index: 0 }, |b| b[8] = 1);
        j.commit();

        assert_eq!(j.done[0].kind, EditKind::Composite);
    }

    #[test]
    fn an_unbalanced_commit_is_harmless() {
        let mut j = Journal::default();
        j.commit();
        j.commit();
        assert!(!j.can_undo());
    }

    #[test]
    fn a_new_edit_discards_the_redo_stack() {
        let mut raw = vec![0u8; 16];
        let mut j = Journal::default();

        edit(&mut j, &mut raw, EditKind::KitName { index: 0 }, |b| b[0] = 1);
        j.undo(&mut raw);
        assert!(j.can_redo());

        edit(&mut j, &mut raw, EditKind::KitName { index: 1 }, |b| b[1] = 1);
        assert!(!j.can_redo(), "the undone branch is unreachable now");
    }

    #[test]
    fn dirty_tracking_follows_the_saved_point() {
        let mut raw = vec![0u8; 16];
        let mut j = Journal::default();

        j.mark_saved();
        assert!(!j.is_dirty());

        edit(&mut j, &mut raw, EditKind::KitName { index: 0 }, |b| b[0] = 1);
        assert!(j.is_dirty());

        // Undoing back to where we saved makes the document honestly clean.
        j.undo(&mut raw);
        assert!(!j.is_dirty());

        j.redo(&mut raw);
        assert!(j.is_dirty());
    }

    #[test]
    fn overlapping_spans_in_one_transaction_unwind_in_order() {
        // Two edits to the same byte inside one gesture: undo must land on the
        // value from before the gesture, not the value between the two edits.
        let mut raw = vec![0u8; 8];
        let mut j = Journal::default();

        j.begin("Twice".into());
        edit(&mut j, &mut raw, EditKind::Steps { pattern: 0, variation: 0 }, |b| b[0] = 1);
        edit(&mut j, &mut raw, EditKind::Steps { pattern: 0, variation: 0 }, |b| b[0] = 2);
        j.commit();

        assert_eq!(raw[0], 2);
        j.undo(&mut raw);
        assert_eq!(raw[0], 0, "must unwind to the pre-gesture value");
        j.redo(&mut raw);
        assert_eq!(raw[0], 2);
    }

    #[test]
    fn the_entry_bound_drops_the_oldest() {
        let mut raw = vec![0u8; MAX_ENTRIES + 10];
        let mut j = Journal::default();

        for i in 0..(MAX_ENTRIES + 5) {
            edit(&mut j, &mut raw, EditKind::KitName { index: i as u32 }, |b| {
                b[i] = 1
            });
        }
        assert_eq!(j.done.len(), MAX_ENTRIES);
    }

}
