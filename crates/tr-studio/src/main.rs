//! `tr-studio` CLI — friendly views over TR-6S/TR-8S backups.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tr_format::Backup;
use tr_studio::pattern_grid;

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
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Grid {
            backup,
            pattern,
            variation,
        } => grid(&backup, pattern, variation),
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
