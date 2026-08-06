//! Native fallback: find candidate firmware payloads by size + entropy.
//!
//! After an installer is unpacked, the firmware is usually one large, dense
//! blob sitting among many small resource files. This walks a file or tree and
//! ranks candidates: big enough, high average entropy (payloads are typically
//! packed/compressed), and — as a bonus signal — a size near a power-of-two
//! flash geometry. It carves nothing destructively; it just points you at the
//! files worth feeding to `fw-analyze`.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

struct Candidate {
    path: PathBuf,
    size: u64,
    entropy: f64,
    round_size: bool,
}

pub fn run(path: &Path, min_size: u64, max_size: u64) -> Result<()> {
    let mut candidates = Vec::new();
    let mut files_seen = 0usize;
    walk(path, &mut |p| {
        files_seen += 1;
        if let Ok(md) = fs::metadata(p) {
            let size = md.len();
            if size >= min_size && size <= max_size {
                if let Ok(entropy) = sampled_entropy(p) {
                    candidates.push(Candidate {
                        path: p.to_path_buf(),
                        size,
                        entropy,
                        round_size: looks_like_flash_size(size),
                    });
                }
            }
        }
    })?;

    // Rank: round flash-like sizes first, then by entropy, then by size.
    candidates.sort_by(|a, b| {
        b.round_size
            .cmp(&a.round_size)
            .then(
                b.entropy
                    .partial_cmp(&a.entropy)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(b.size.cmp(&a.size))
    });

    println!(
        "# scanned {files_seen} file(s); {} candidate(s) in [{}..{}] bytes",
        candidates.len(),
        min_size,
        if max_size == u64::MAX {
            "∞".to_string()
        } else {
            max_size.to_string()
        }
    );
    if candidates.is_empty() {
        println!("# nothing matched — widen --min-size/--max-size, or the payload may be nested.");
        return Ok(());
    }

    println!("# rank  entropy  flash?  size         path");
    for (i, c) in candidates.iter().enumerate() {
        println!(
            "  {:>3}   {:5.3}   {:<5}   {:>10}   {}",
            i + 1,
            c.entropy,
            if c.round_size { "yes" } else { "no" },
            c.size,
            c.path.display()
        );
    }
    println!("# feed the top candidate to: fw-analyze entropy / inspect / hexdump");
    Ok(())
}

/// Depth-first walk; calls `f` for every regular file under `root`.
fn walk(root: &Path, f: &mut impl FnMut(&Path)) -> Result<()> {
    let md = fs::metadata(root).with_context(|| format!("stat {}", root.display()))?;
    if md.is_file() {
        f(root);
        return Ok(());
    }
    if md.is_dir() {
        let mut entries: Vec<_> = fs::read_dir(root)
            .with_context(|| format!("reading dir {}", root.display()))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .collect();
        entries.sort();
        for p in entries {
            // Best-effort: skip entries we can't traverse rather than aborting.
            let _ = walk(&p, f);
        }
    }
    Ok(())
}

/// Shannon entropy over a bounded sample (head+tail) so we don't read gigabytes.
fn sampled_entropy(path: &Path) -> Result<f64> {
    use std::io::{Seek, SeekFrom};
    const SAMPLE: usize = 64 * 1024;
    let mut f = fs::File::open(path)?;
    let len = f.metadata()?.len();

    let mut buf = Vec::new();
    let head_n = (len as usize).min(SAMPLE);
    buf.resize(head_n, 0);
    read_exact_up_to(&mut f, &mut buf)?;

    if len as usize > SAMPLE * 2 {
        f.seek(SeekFrom::End(-(SAMPLE as i64)))?;
        let mut tail = vec![0u8; SAMPLE];
        let got = read_exact_up_to(&mut f, &mut tail)?;
        tail.truncate(got);
        buf.extend_from_slice(&tail);
    }

    Ok(shannon(&buf))
}

fn read_exact_up_to(r: &mut impl std::io::Read, buf: &mut [u8]) -> Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

fn shannon(bytes: &[u8]) -> f64 {
    if bytes.is_empty() {
        return 0.0;
    }
    let mut counts = [0u64; 256];
    for &b in bytes {
        counts[b as usize] += 1;
    }
    let len = bytes.len() as f64;
    let mut h = 0.0;
    for &c in counts.iter() {
        if c == 0 {
            continue;
        }
        let p = c as f64 / len;
        h -= p * p.log2();
    }
    h
}

/// True if the size is a whole number of common flash-part sizes (helps spot a
/// full-image dump vs. a fragment). The TR-6S carries a 64MB NOR part, so this
/// flags 64MB and its neighbours.
fn looks_like_flash_size(size: u64) -> bool {
    const MB: u64 = 1024 * 1024;
    [8, 16, 32, 64, 128].iter().any(|&m| size == m * MB)
}
