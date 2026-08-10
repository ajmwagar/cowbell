//! `tr-studio` CLI — friendly views over TR-6S/TR-8S backups.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tr_format::Backup;
use tr_studio::{pattern_grid, variation_motion};

#[derive(Parser)]
#[command(name = "tr-studio", version, about = "Read and build TR-6S/TR-8S patterns", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Render a pattern's step grid (six voices) for one variation.
    Grid {
        /// A TR backup file.
        backup: PathBuf,
        /// 1-based pattern number.
        pattern: usize,
        /// Variation: 0=A … 7=H, 8/9=fills.
        #[arg(default_value_t = 0)]
        variation: usize,
    },
    /// Show a pattern variation's recorded motion (per-step parameter lanes).
    Motion {
        /// A TR backup file.
        backup: PathBuf,
        /// 1-based pattern number.
        pattern: usize,
        /// Variation: 0=A … 7=H, 8/9=fills.
        #[arg(default_value_t = 0)]
        variation: usize,
    },
    /// Chop a break WAV into N slices and write one WAV per slice (the
    /// pre-sliced breakbeat path). Import the slices to tone slots, then wire
    /// them with the library's `apply_breakbeat`.
    Slice {
        /// A 16-bit PCM WAV of the break.
        wav: PathBuf,
        /// Number of slices.
        slices: usize,
        /// Directory to write the slice WAVs into (created if missing).
        #[arg(long, default_value = "slices")]
        out: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Grid {
            backup,
            pattern,
            variation,
        } => grid(&backup, pattern, variation),
        Command::Motion {
            backup,
            pattern,
            variation,
        } => motion(&backup, pattern, variation),
        Command::Slice { wav, slices, out } => slice_cmd(&wav, slices, &out),
    }
}

fn slice_cmd(wav_path: &std::path::Path, n: usize, out: &std::path::Path) -> Result<()> {
    use tr_studio::{export_slices, slice_grid, Wav, MAX_SLICES};
    let bytes =
        std::fs::read(wav_path).with_context(|| format!("reading {}", wav_path.display()))?;
    let wav = Wav::read(&bytes).context("parsing WAV (needs 16-bit PCM)")?;
    let plan = slice_grid(&wav, n)?;
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let files = export_slices(&wav, &plan);
    let secs = plan.total_frames as f32 / plan.sample_rate as f32;
    println!(
        "{:.2}s break, {} ch @ {} Hz -> {} slices",
        secs,
        plan.channels,
        plan.sample_rate,
        plan.slices.len()
    );
    for ((name, data), s) in files.iter().zip(&plan.slices) {
        let path = out.join(name);
        std::fs::write(&path, data).with_context(|| format!("writing {}", path.display()))?;
        let ms = s.len_frames() as f32 / plan.sample_rate as f32 * 1000.0;
        println!(
            "  {name}  frames {}..{} ({ms:.0} ms)",
            s.start_frame, s.end_frame
        );
    }
    if n > MAX_SLICES {
        println!(
            "note: a kit sequences at most {MAX_SLICES} slices; the extra {} are for manual use.",
            n - MAX_SLICES
        );
    }
    println!("wrote {} slice WAVs to {}", files.len(), out.display());
    Ok(())
}

fn motion(path: &std::path::Path, number: usize, variation: usize) -> Result<()> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let b = Backup::parse(bytes)?;
    let p = *b
        .patterns()
        .get(number.wrapping_sub(1))
        .with_context(|| format!("pattern {number} out of range"))?;
    let raw = b.raw();
    let var = "ABCDEFGH".chars().nth(variation).unwrap_or('?');
    println!(
        "Pattern {number}: {}   [Variation {var}] motion",
        p.name(raw)
    );
    let planes = variation_motion(raw, &p, variation);
    if planes.is_empty() {
        println!("  (no motion recorded)");
        return Ok(());
    }
    for m in planes {
        println!("{}:", slot_label(m.slot));
        print!("{}", m.render());
    }
    Ok(())
}

/// A human label for a motion slot: the TR-6S voice it belongs to, or the plane.
fn slot_label(slot: usize) -> String {
    match tr_format::track_role(slot) {
        Some(tr_format::TrackRole::Motion(i)) => tr_format::VOICES
            .get(i)
            .map_or_else(|| format!("INST{:02}", i + 1), |v| v.to_string()),
        Some(tr_format::TrackRole::OtherMotion(0)) => "DELAY".to_string(),
        Some(tr_format::TrackRole::OtherMotion(_)) => "REVERB / MFX".to_string(),
        _ => format!("slot {slot}"),
    }
}

fn grid(path: &std::path::Path, number: usize, variation: usize) -> Result<()> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let b = Backup::parse(bytes)?;
    let p = *b
        .patterns()
        .get(number.wrapping_sub(1))
        .with_context(|| format!("pattern {number} out of range"))?;
    let raw = b.raw();
    let var = "ABCDEFGH".chars().nth(variation).unwrap_or('?');
    println!(
        "Pattern {number}: {}   {:.1} BPM   Kit {}   [Variation {var}]",
        p.name(raw),
        p.tempo_bpm(raw),
        p.kit_ref(raw)
    );
    let g = pattern_grid(&b, number, variation).context("reading grid")?;
    print!("{}", g.render());
    Ok(())
}
