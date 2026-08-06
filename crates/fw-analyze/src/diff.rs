//! Byte-level diff of two firmware images.
//!
//! Useful for comparing two firmware revisions (what changed between updates?)
//! or a dump against a known-good reference. Reports contiguous runs of
//! differing bytes rather than one line per byte.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

pub fn run(a: &Path, b: &Path, max_runs: usize) -> Result<()> {
    let da = fs::read(a).with_context(|| format!("reading {}", a.display()))?;
    let db = fs::read(b).with_context(|| format!("reading {}", b.display()))?;

    println!("# diff");
    println!("#   a = {} ({} bytes)", a.display(), da.len());
    println!("#   b = {} ({} bytes)", b.display(), db.len());
    if da.len() != db.len() {
        println!(
            "# NOTE: sizes differ by {} bytes",
            (da.len() as i64 - db.len() as i64).abs()
        );
    }

    let common = da.len().min(db.len());
    let mut runs = 0usize;
    let mut diff_bytes = 0usize;
    let mut i = 0usize;

    while i < common {
        if da[i] != db[i] {
            let start = i;
            while i < common && da[i] != db[i] {
                i += 1;
            }
            diff_bytes += i - start;
            if runs < max_runs {
                println!(
                    "0x{:08x}  len {:<6}  a[0]=0x{:02x} b[0]=0x{:02x}",
                    start,
                    i - start,
                    da[start],
                    db[start]
                );
            } else if runs == max_runs {
                println!("... (further runs suppressed; raise --max-runs to see them)");
            }
            runs += 1;
        } else {
            i += 1;
        }
    }

    if da.len() != db.len() {
        let tail = da.len().max(db.len()) - common;
        diff_bytes += tail;
        println!(
            "0x{:08x}  len {:<6}  (trailing bytes present in only one image)",
            common, tail
        );
    }

    println!(
        "# {} differing run(s), {} differing byte(s) over {} common byte(s)",
        runs, diff_bytes, common
    );
    Ok(())
}
