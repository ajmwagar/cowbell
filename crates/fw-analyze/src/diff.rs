//! Diff of two firmware images.
//!
//! Two modes:
//!
//! * **Byte mode** (default) — reports contiguous runs of differing bytes
//!   rather than one line per byte. Useful for comparing two firmware
//!   revisions (what changed between updates?) or a dump against a
//!   known-good reference.
//! * **Block-aligned ("ECB") mode** (`--block N`, N>0) — treats each image as
//!   a sequence of fixed-size blocks and reports the ECB signatures the
//!   TR-6S payload analysis relies on: per-image duplicate-block ratio,
//!   position-independent (set) block overlap, positional (same-offset)
//!   overlap, and the longest run of same-offset identical blocks. A long
//!   same-offset identical run across two images is the tell-tale of a
//!   shared ECB key (see `docs/firmware-format.md`).
//!
//! Both modes honour `--offset O`, which starts the comparison at byte
//! offset `O` in each image so callers can skip the ~96-byte plaintext
//! header and compare only the encrypted body.

use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

pub fn run(a: &Path, b: &Path, max_runs: usize, block: usize, offset: u64) -> Result<()> {
    let da = fs::read(a).with_context(|| format!("reading {}", a.display()))?;
    let db = fs::read(b).with_context(|| format!("reading {}", b.display()))?;

    let off = offset as usize;
    // Slice off the leading header region; tolerate an offset past EOF.
    let sa: &[u8] = if off <= da.len() { &da[off..] } else { &[] };
    let sb: &[u8] = if off <= db.len() { &db[off..] } else { &[] };

    if block == 0 {
        byte_diff(a, b, &da, &db, off, max_runs);
    } else {
        anyhow::ensure!(block > 0, "block size must be > 0");
        println!("# block-diff");
        println!("#   a = {} ({} bytes)", a.display(), da.len());
        println!("#   b = {} ({} bytes)", b.display(), db.len());
        println!("#   block = {block}  offset = 0x{off:x} ({off})");
        if da.len() != db.len() {
            println!(
                "# NOTE: file sizes differ by {} bytes",
                (da.len() as i64 - db.len() as i64).abs()
            );
        }
        let m = block_diff(sa, sb, block, offset, max_runs);
        m.print();
    }
    Ok(())
}

/// Contiguous differing-run reporter (the original byte-level diff). Operates
/// on the post-offset slices; printed offsets are in original file
/// coordinates (offset added back), so with the default `--offset 0` the
/// output is identical to the historical behaviour.
fn byte_diff(a: &Path, b: &Path, da: &[u8], db: &[u8], off: usize, max_runs: usize) {
    let sa: &[u8] = if off <= da.len() { &da[off..] } else { &[] };
    let sb: &[u8] = if off <= db.len() { &db[off..] } else { &[] };

    println!("# diff");
    println!("#   a = {} ({} bytes)", a.display(), da.len());
    println!("#   b = {} ({} bytes)", b.display(), db.len());
    if off != 0 {
        println!("#   offset = 0x{off:x} ({off})");
    }
    if da.len() != db.len() {
        println!(
            "# NOTE: sizes differ by {} bytes",
            (da.len() as i64 - db.len() as i64).abs()
        );
    }

    let common = sa.len().min(sb.len());
    let mut runs = 0usize;
    let mut diff_bytes = 0usize;
    let mut i = 0usize;

    while i < common {
        if sa[i] != sb[i] {
            let start = i;
            while i < common && sa[i] != sb[i] {
                i += 1;
            }
            diff_bytes += i - start;
            if runs < max_runs {
                println!(
                    "0x{:08x}  len {:<6}  a[0]=0x{:02x} b[0]=0x{:02x}",
                    off + start,
                    i - start,
                    sa[start],
                    sb[start]
                );
            } else if runs == max_runs {
                println!("... (further runs suppressed; raise --max-runs to see them)");
            }
            runs += 1;
        } else {
            i += 1;
        }
    }

    if sa.len() != sb.len() {
        let tail = sa.len().max(sb.len()) - common;
        diff_bytes += tail;
        println!(
            "0x{:08x}  len {:<6}  (trailing bytes present in only one image)",
            off + common,
            tail
        );
    }

    println!(
        "# {} differing run(s), {} differing byte(s) over {} common byte(s)",
        runs, diff_bytes, common
    );
}

/// A contiguous run of same-offset byte-identical blocks. `start` is in
/// original file coordinates (i.e. `--offset` already added back); `len` is
/// in bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub start: usize,
    pub len: usize,
}

/// Everything the block-aligned diff computes, so tests can assert on values
/// rather than parsing printed output. `print()` renders it.
#[derive(Debug, Clone)]
pub struct BlockDiff {
    pub block_size: usize,
    pub offset: u64,
    /// Post-offset byte lengths actually analysed.
    pub len_a: usize,
    pub len_b: usize,
    /// Whole-block counts within each image's post-offset region.
    pub blocks_a: usize,
    pub blocks_b: usize,
    /// Distinct block values per image.
    pub unique_a: usize,
    pub unique_b: usize,
    /// Most common block value and its occurrence count, per image.
    pub top_a: Option<(Vec<u8>, usize)>,
    pub top_b: Option<(Vec<u8>, usize)>,
    /// Distinct block VALUES present in both images (position-independent).
    pub shared_values: usize,
    /// Distinct block values present in only one image.
    pub a_only_values: usize,
    pub b_only_values: usize,
    /// Aligned block positions over the common length.
    pub common_blocks: usize,
    /// Blocks byte-identical at the SAME offset.
    pub positional_matches: usize,
    /// Longest same-offset identical run (None if there is none).
    pub longest_run: Option<Run>,
    /// Runs sorted longest-first, capped at `max_runs`.
    pub top_runs: Vec<Run>,
}

impl BlockDiff {
    /// Duplicate-block ratio for image A, in percent.
    pub fn dup_ratio_a(&self) -> f64 {
        dup_ratio(self.blocks_a, self.unique_a)
    }
    /// Duplicate-block ratio for image B, in percent.
    pub fn dup_ratio_b(&self) -> f64 {
        dup_ratio(self.blocks_b, self.unique_b)
    }
    /// Duplicate instances (total blocks minus unique) for image A.
    pub fn dup_a(&self) -> usize {
        self.blocks_a - self.unique_a
    }
    /// Duplicate instances for image B.
    pub fn dup_b(&self) -> usize {
        self.blocks_b - self.unique_b
    }
    /// Same-offset positional overlap, in percent of the common length.
    pub fn positional_ratio(&self) -> f64 {
        if self.common_blocks == 0 {
            0.0
        } else {
            self.positional_matches as f64 / self.common_blocks as f64 * 100.0
        }
    }

    fn print(&self) {
        let common_bytes = self.common_blocks * self.block_size;

        println!(
            "# analysing {} / {} bytes (a / b) from offset 0x{:x}",
            self.len_a, self.len_b, self.offset
        );
        let tail_a = self.len_a % self.block_size;
        let tail_b = self.len_b % self.block_size;
        if tail_a != 0 || tail_b != 0 {
            println!(
                "# NOTE: trailing partial bytes ignored: {} in a, {} in b",
                tail_a, tail_b
            );
        }

        println!("# -- duplicate-block ratio (ECB signature) --");
        println!(
            "#   a: {} blocks, {} unique, {} dup ({:.2}%)",
            self.blocks_a,
            self.unique_a,
            self.dup_a(),
            self.dup_ratio_a()
        );
        if let Some((v, c)) = &self.top_a {
            println!("#      top block {} x{}", hex(v), c);
        }
        println!(
            "#   b: {} blocks, {} unique, {} dup ({:.2}%)",
            self.blocks_b,
            self.unique_b,
            self.dup_b(),
            self.dup_ratio_b()
        );
        if let Some((v, c)) = &self.top_b {
            println!("#      top block {} x{}", hex(v), c);
        }

        println!("# -- set overlap (position-independent) --");
        println!(
            "#   {} distinct block value(s) present in both",
            self.shared_values
        );
        println!(
            "#   {} value(s) only in a, {} value(s) only in b",
            self.a_only_values, self.b_only_values
        );

        println!("# -- positional overlap (same offset) --");
        println!(
            "#   {} of {} aligned block(s) identical at same offset ({:.2}%) over {} common byte(s)",
            self.positional_matches,
            self.common_blocks,
            self.positional_ratio(),
            common_bytes
        );

        println!("# -- longest same-offset identical run(s) --");
        match &self.longest_run {
            Some(r) => println!(
                "#   longest: 0x{:08x} len {} bytes ({} block(s))",
                r.start,
                r.len,
                r.len / self.block_size
            ),
            None => println!("#   (no same-offset identical blocks)"),
        }
        for r in &self.top_runs {
            println!(
                "0x{:08x}  len {:<8}  ({} block(s))",
                r.start,
                r.len,
                r.len / self.block_size
            );
        }
    }
}

fn dup_ratio(blocks: usize, unique: usize) -> f64 {
    if blocks == 0 {
        0.0
    } else {
        (blocks - unique) as f64 / blocks as f64 * 100.0
    }
}

/// Lowercase hex of a block value.
fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Core block-aligned computation. `sa`/`sb` are the post-offset slices;
/// `offset` is added back into run start coordinates. Returns metrics; does
/// no I/O or printing (so it is unit-testable on synthetic data).
pub fn block_diff(sa: &[u8], sb: &[u8], block: usize, offset: u64, max_runs: usize) -> BlockDiff {
    debug_assert!(block > 0);
    let blocks_a = sa.len() / block;
    let blocks_b = sb.len() / block;

    // Per-value occurrence counts (whole blocks only; trailing partial bytes
    // are ignored for the ratio, and noted separately by the caller).
    let counts_a = block_counts(sa, blocks_a, block);
    let counts_b = block_counts(sb, blocks_b, block);

    let unique_a = counts_a.len();
    let unique_b = counts_b.len();

    let top_a = top_block(&counts_a);
    let top_b = top_block(&counts_b);

    // Set-based (position-independent) overlap. Intersect the smaller key set
    // against the larger for a cheap membership test.
    let set_a: HashSet<&[u8]> = counts_a.keys().copied().collect();
    let set_b: HashSet<&[u8]> = counts_b.keys().copied().collect();
    let shared_values = if set_a.len() <= set_b.len() {
        set_a.iter().filter(|k| set_b.contains(*k)).count()
    } else {
        set_b.iter().filter(|k| set_a.contains(*k)).count()
    };
    let a_only_values = unique_a - shared_values;
    let b_only_values = unique_b - shared_values;

    // Positional (same-offset) overlap and same-offset identical runs.
    let common_blocks = blocks_a.min(blocks_b);
    let mut runs: Vec<Run> = Vec::new();
    let mut positional_matches = 0usize;
    let mut i = 0usize;
    while i < common_blocks {
        if block_at(sa, i, block) == block_at(sb, i, block) {
            let start_block = i;
            while i < common_blocks && block_at(sa, i, block) == block_at(sb, i, block) {
                i += 1;
            }
            let len_blocks = i - start_block;
            positional_matches += len_blocks;
            runs.push(Run {
                start: offset as usize + start_block * block,
                len: len_blocks * block,
            });
        } else {
            i += 1;
        }
    }

    runs.sort_by(|x, y| y.len.cmp(&x.len).then(x.start.cmp(&y.start)));
    let longest_run = runs.first().cloned();
    let mut top_runs = runs;
    top_runs.truncate(max_runs);

    BlockDiff {
        block_size: block,
        offset,
        len_a: sa.len(),
        len_b: sb.len(),
        blocks_a,
        blocks_b,
        unique_a,
        unique_b,
        top_a,
        top_b,
        shared_values,
        a_only_values,
        b_only_values,
        common_blocks,
        positional_matches,
        longest_run,
        top_runs,
    }
}

fn block_at(data: &[u8], i: usize, block: usize) -> &[u8] {
    &data[i * block..(i + 1) * block]
}

fn block_counts(data: &[u8], n_blocks: usize, block: usize) -> HashMap<&[u8], usize> {
    let mut counts: HashMap<&[u8], usize> = HashMap::with_capacity(n_blocks);
    for i in 0..n_blocks {
        *counts.entry(block_at(data, i, block)).or_insert(0) += 1;
    }
    counts
}

/// Most common block value and its count. Ties break on the lexicographically
/// smallest value for determinism.
fn top_block(counts: &HashMap<&[u8], usize>) -> Option<(Vec<u8>, usize)> {
    counts
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(k, v)| (k.to_vec(), *v))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Block size used across the synthetic tests.
    const BS: usize = 4;

    // Helper: build a byte vector from a list of 4-byte block "tags". Each tag
    // byte is repeated to fill the block, giving fully controlled distinct
    // block values.
    fn img(tags: &[u8]) -> Vec<u8> {
        let mut v = Vec::with_capacity(tags.len() * BS);
        for &t in tags {
            v.extend_from_slice(&[t; BS]);
        }
        v
    }

    #[test]
    fn duplicate_ratio_counts_repeats() {
        // A: blocks 1,1,1,2 -> 4 blocks, 2 unique, 2 dup instances (50%).
        let a = img(&[1, 1, 1, 2]);
        let b = img(&[9, 9]); // 2 blocks, 1 unique, 1 dup (50%).
        let m = block_diff(&a, &b, BS, 0, 64);

        assert_eq!(m.blocks_a, 4);
        assert_eq!(m.unique_a, 2);
        assert_eq!(m.dup_a(), 2);
        assert!((m.dup_ratio_a() - 50.0).abs() < 1e-9);

        assert_eq!(m.blocks_b, 2);
        assert_eq!(m.unique_b, 1);
        assert!((m.dup_ratio_b() - 50.0).abs() < 1e-9);

        // Most common block of A is value 1, appearing 3 times.
        let (val, cnt) = m.top_a.clone().unwrap();
        assert_eq!(val, vec![1u8; BS]);
        assert_eq!(cnt, 3);
    }

    #[test]
    fn set_overlap_is_position_independent() {
        // A has values {1,2,3}; B has values {3,1,4} at different positions.
        // Shared distinct values: {1,3} -> 2. A-only {2}, B-only {4}.
        let a = img(&[1, 2, 3]);
        let b = img(&[3, 1, 4]);
        let m = block_diff(&a, &b, BS, 0, 64);

        assert_eq!(m.unique_a, 3);
        assert_eq!(m.unique_b, 3);
        assert_eq!(m.shared_values, 2);
        assert_eq!(m.a_only_values, 1);
        assert_eq!(m.b_only_values, 1);
    }

    #[test]
    fn positional_overlap_and_ratio() {
        // Aligned positions: 0:same 1:diff 2:same 3:diff  -> 2 of 4 = 50%.
        let a = img(&[7, 1, 7, 1]);
        let b = img(&[7, 2, 7, 2]);
        let m = block_diff(&a, &b, BS, 0, 64);

        assert_eq!(m.common_blocks, 4);
        assert_eq!(m.positional_matches, 2);
        assert!((m.positional_ratio() - 50.0).abs() < 1e-9);
    }

    #[test]
    fn longest_run_offset_and_length_in_original_coords() {
        // Positions:      0    1    2    3    4    5
        // A:              5    5    5    9    1    1
        // B:              5    5    5    8    1    1
        // Same-offset identical runs: blocks 0..3 (len 3) and blocks 4..6 (len 2).
        // With --offset 16, the long run starts at original byte 16 + 0*4 = 16,
        // length 3*4 = 12 bytes. The short run starts at 16 + 4*4 = 32.
        let a = img(&[5, 5, 5, 9, 1, 1]);
        let b = img(&[5, 5, 5, 8, 1, 1]);
        let offset = 16u64;
        let m = block_diff(&a, &b, BS, offset, 64);

        assert_eq!(m.positional_matches, 5); // 3 + 2 blocks
        let longest = m.longest_run.clone().unwrap();
        assert_eq!(longest.start, 16);
        assert_eq!(longest.len, 12);

        // Two runs, longest first, both listed under a generous cap.
        assert_eq!(m.top_runs.len(), 2);
        assert_eq!(m.top_runs[0], Run { start: 16, len: 12 });
        assert_eq!(m.top_runs[1], Run { start: 32, len: 8 });
    }

    #[test]
    fn max_runs_caps_listed_runs_but_not_longest() {
        // Three separate 1-block runs (positions 0, 2, 4), all length 4 bytes.
        let a = img(&[3, 0, 3, 0, 3]);
        let b = img(&[3, 1, 3, 1, 3]);
        let m = block_diff(&a, &b, BS, 0, 2);

        assert_eq!(m.positional_matches, 3);
        assert!(m.longest_run.is_some());
        // Cap of 2 applies to the listed runs.
        assert_eq!(m.top_runs.len(), 2);
    }

    #[test]
    fn differing_lengths_compare_over_common_region() {
        // A is longer than B; extra trailing block in A must not crash and is
        // excluded from positional comparison.
        let a = img(&[1, 2, 3, 4]); // 4 blocks
        let b = img(&[1, 2]); // 2 blocks
        let m = block_diff(&a, &b, BS, 0, 64);

        assert_eq!(m.blocks_a, 4);
        assert_eq!(m.blocks_b, 2);
        assert_eq!(m.common_blocks, 2);
        assert_eq!(m.positional_matches, 2);
        assert_eq!(m.longest_run.unwrap().len, 8);
    }

    #[test]
    fn trailing_partial_bytes_are_ignored_not_crashed() {
        // 9 bytes at block size 4 -> 2 whole blocks, 1 trailing byte.
        let a = vec![0u8; 9];
        let b = vec![0u8; 9];
        let m = block_diff(&a, &b, BS, 0, 64);
        assert_eq!(m.blocks_a, 2);
        assert_eq!(m.blocks_b, 2);
        assert_eq!(m.common_blocks, 2);
        assert_eq!(m.positional_matches, 2);
    }
}
