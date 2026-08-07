//! Sliding-window Shannon entropy scan.
//!
//! High, flat entropy (~8.0 bits/byte) across a region is a strong hint that
//! the region is compressed or encrypted; low, structured entropy suggests
//! code, tables, or padding. This is the first thing to run on an unknown
//! `.bin` to find the boundaries worth carving.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Shannon entropy in bits/byte for a slice (0.0..=8.0).
pub fn shannon(bytes: &[u8]) -> f64 {
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

pub fn run(image: &Path, window: usize, step: usize) -> Result<()> {
    anyhow::ensure!(window > 0, "window must be > 0");
    anyhow::ensure!(step > 0, "step must be > 0");

    let data = fs::read(image).with_context(|| format!("reading {}", image.display()))?;

    println!(
        "# entropy scan of {} ({} bytes), window={} step={}",
        image.display(),
        data.len(),
        window,
        step
    );
    println!("# offset       entropy  bar");

    let mut offset = 0usize;
    while offset < data.len() {
        let end = (offset + window).min(data.len());
        let h = shannon(&data[offset..end]);
        let bars = (h / 8.0 * 40.0).round() as usize;
        println!("0x{:08x}  {:6.3}  {}", offset, h, "#".repeat(bars));
        offset += step;
    }
    Ok(())
}
