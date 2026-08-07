//! Block-cipher mode analysis: is a ciphertext region ECB, and could a
//! keystream explain it instead?
//!
//! Repeated ciphertext blocks are the ECB fingerprint. But repetition alone does
//! not prove ECB — a *repeating XOR pad* produces repeats too, and if the scheme
//! were really a keystream mode (CTR/OFB/CFB) or a short pad, then known
//! plaintext would hand you the keystream and the whole image with it. This
//! command separates those cases with the **offset-independence** test:
//!
//! - Under a keystream or pad of period `P`, two ciphertext blocks are identical
//!   only if the plaintext matches **and** their byte offsets are congruent
//!   mod `P`. Every gap between occurrences is then a multiple of `P`.
//! - Under ECB, `C = E_K(P)` depends only on the block's contents, so identical
//!   plaintext repeats at **arbitrary** offsets and the gcd of those gaps
//!   collapses to the block size.
//!
//! So `gcd == block_size` excludes every period larger than one block, and the
//! handful of remaining periods are excluded directly: a pad of period `p` that
//! divides the block would make the repeated block itself `block_size/p`
//! identical chunks, and a full-block pad would collapse the image's entropy
//! when XOR-ed out.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::entropy::shannon;

/// Greatest common divisor.
fn gcd(a: usize, b: usize) -> usize {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// Duplicate-block statistics at one block size.
fn block_stats(data: &[u8], size: usize) -> (usize, usize) {
    let mut seen: HashMap<&[u8], usize> = HashMap::new();
    let n = data.len() / size;
    for i in 0..n {
        *seen.entry(&data[i * size..(i + 1) * size]).or_insert(0) += 1;
    }
    (n, seen.len())
}

pub fn run(image: &Path, offset: u64, len: Option<u64>, block: usize) -> Result<()> {
    anyhow::ensure!(block > 0, "block size must be > 0");
    let data = fs::read(image).with_context(|| format!("reading {}", image.display()))?;
    let start = offset as usize;
    anyhow::ensure!(start < data.len(), "offset past end of file");
    let end = len.map_or(data.len(), |l| (start + l as usize).min(data.len()));
    let region = &data[start..end];

    println!(
        "# block-mode analysis of {} [0x{:x}..0x{:x}] = {} bytes",
        image.display(),
        start,
        end,
        region.len()
    );
    println!(
        "# length divisible by {block}: {}",
        region.len() % block == 0
    );

    // --- duplicate rates: the block size that maximises duplication is the
    // cipher's block size; a larger size showing merely concatenated pairs means
    // the fundamental block is the smaller one.
    println!("\n## duplicate-block rates");
    println!(
        "  {:>6}  {:>10}  {:>10}  {:>12}",
        "size", "blocks", "unique", "duplicates"
    );
    for size in [block, block * 2, block * 4] {
        if region.len() < size {
            continue;
        }
        let (total, unique) = block_stats(region, size);
        let dups = total - unique;
        println!(
            "  {:>6}  {:>10}  {:>10}  {:>7} ({:5.2}%)",
            size,
            total,
            unique,
            dups,
            100.0 * dups as f64 / total as f64
        );
    }

    // --- the repeated blocks themselves
    let mut counts: HashMap<&[u8], usize> = HashMap::new();
    let n = region.len() / block;
    for i in 0..n {
        *counts
            .entry(&region[i * block..(i + 1) * block])
            .or_insert(0) += 1;
    }
    let mut ranked: Vec<_> = counts.iter().map(|(b, c)| (*b, *c)).collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));

    println!("\n## most frequent blocks");
    for (b, c) in ranked.iter().take(6) {
        println!("  {}  x{}", hex(b), c);
    }

    let Some(&(top, ntop)) = ranked.first() else {
        println!("\n(no blocks to analyse)");
        return Ok(());
    };
    if ntop < 3 {
        println!("\nno block repeats often enough for the offset test.");
        return Ok(());
    }

    // --- offset independence
    let offs: Vec<usize> = (0..n)
        .filter(|&i| &region[i * block..(i + 1) * block] == top)
        .map(|i| i * block)
        .collect();
    let diffs: Vec<usize> = offs.windows(2).map(|w| w[1] - w[0]).collect();
    let g = diffs.iter().copied().fold(0usize, gcd);
    let not_double = diffs.iter().filter(|d| *d % (block * 2) != 0).count();

    println!("\n## offset-independence test");
    println!("  dominant block {} occurs {ntop}x", hex(top));
    println!("  gcd of gaps between occurrences: {g}");
    println!(
        "  gaps not a multiple of {}: {} of {}",
        block * 2,
        not_double,
        diffs.len()
    );
    if g == block {
        println!(
            "  => the block repeats at ARBITRARY offsets, so no keystream or pad of\n     \
             period > {block} can explain it. Consistent with ECB."
        );
    } else {
        println!("  => gaps share period {g}; a keystream/pad of that period is NOT excluded.");
    }

    // --- close the remaining periods (those dividing the block size)
    println!("\n## remaining periods (must divide {block})");
    let mut p = 1;
    while p < block {
        if block % p == 0 {
            let chunks: std::collections::HashSet<&[u8]> =
                (0..block / p).map(|i| &top[i * p..(i + 1) * p]).collect();
            println!(
                "  period {p:>3}: dominant block is {} chunks of {p} B, {} distinct => {}",
                block / p,
                chunks.len(),
                if chunks.len() > 1 {
                    "EXCLUDED (chunks differ)"
                } else {
                    "possible — investigate"
                }
            );
        }
        p *= 2;
    }

    // A full-block pad would BE the dominant block; XOR-ing it out at every
    // alignment would collapse the image to structure if the guess were right.
    println!("  period {block:>3}: XOR the dominant block out as a repeating pad —");
    println!("            a real pad would collapse entropy and expose structure");
    let base = shannon(region);
    println!("            entropy as-is: {base:.4}/8");
    for align in 0..block {
        let mut rotated = Vec::with_capacity(block);
        rotated.extend_from_slice(&top[align..]);
        rotated.extend_from_slice(&top[..align]);
        let xored: Vec<u8> = region
            .iter()
            .zip(rotated.iter().cycle())
            .map(|(a, b)| a ^ b)
            .collect();
        let zeros = xored.iter().filter(|&&b| b == 0).count();
        println!(
            "            alignment {align}: entropy {:.4}  zero bytes {:5.2}%",
            shannon(&xored),
            100.0 * zeros as f64 / xored.len() as f64
        );
    }

    // --- how far a codebook could ever get you
    let top10: usize = ranked.iter().take(10).map(|(_, c)| *c).sum();
    let once = ranked.iter().filter(|(_, c)| *c == 1).count();
    println!("\n## codebook reach");
    println!(
        "  the 10 most frequent block values cover {top10} of {n} blocks ({:.2}%)",
        100.0 * top10 as f64 / n as f64
    );
    println!(
        "  block values occurring exactly once: {once} of {} unique ({:.1}%)",
        ranked.len(),
        100.0 * once as f64 / ranked.len() as f64
    );
    println!(
        "  => known-plaintext pairs decrypt only the blocks you already hold; the\n     \
         rest of the image is untouched by them."
    );
    Ok(())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gcd_basics() {
        assert_eq!(gcd(8, 8), 8);
        assert_eq!(gcd(24, 16), 8);
        assert_eq!(gcd(0, 8), 8);
    }

    #[test]
    fn ecb_like_data_has_gcd_of_one_block() {
        // Same "plaintext" encrypted to the same block at offsets that are NOT a
        // common multiple of anything larger than the block size.
        let fill = [0x6b, 0x4c, 0x9a, 0x85, 0x2c, 0x73, 0x28, 0x31];
        let mut data = Vec::new();
        for i in 0..12 {
            if i == 3 || i == 4 || i == 7 || i == 11 {
                data.extend_from_slice(&[i as u8; 8]);
            } else {
                data.extend_from_slice(&fill);
            }
        }
        let offs: Vec<usize> = (0..data.len() / 8)
            .filter(|&i| data[i * 8..i * 8 + 8] == fill)
            .map(|i| i * 8)
            .collect();
        let diffs: Vec<usize> = offs.windows(2).map(|w| w[1] - w[0]).collect();
        assert_eq!(diffs.iter().copied().fold(0usize, gcd), 8);
    }

    #[test]
    fn keystream_like_data_has_a_larger_gcd() {
        // A repeating 32-byte pad over constant plaintext: the "repeat" can only
        // land on offsets congruent mod 32, so the gcd exposes the period.
        let pad: Vec<u8> = (0..32u8).collect();
        let data: Vec<u8> = (0..32 * 8).map(|i| pad[i % 32]).collect();
        let first = &data[0..8];
        let offs: Vec<usize> = (0..data.len() / 8)
            .filter(|&i| &data[i * 8..i * 8 + 8] == first)
            .map(|i| i * 8)
            .collect();
        let diffs: Vec<usize> = offs.windows(2).map(|w| w[1] - w[0]).collect();
        assert_eq!(diffs.iter().copied().fold(0usize, gcd), 32);
    }

    #[test]
    fn block_stats_counts_duplicates() {
        let data = [
            1u8, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1,
        ];
        let (total, unique) = block_stats(&data, 8);
        assert_eq!((total, unique), (3, 2));
    }
}
