//! `tr-format` — inspect Roland TR-6S/TR-8S backup containers.
//!
//! Plaintext user data only. Reads a `*_bak.bin` backup and lists its sections;
//! the library guarantees a byte-exact round-trip, so this is a safe base for a
//! FOSS librarian. No firmware, no decryption, never writes to a device.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tr_format::Backup;

#[derive(Parser)]
#[command(
    name = "tr-format",
    version,
    about = "Inspect Roland TR-6S/TR-8S backup containers (plaintext user data)",
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the file header (magic, version) and section directory.
    Info {
        /// A TR backup file (e.g. tr6s_bak.bin).
        backup: PathBuf,
    },
    /// Verify that the backup round-trips byte-for-byte through the parser.
    Verify {
        /// A TR backup file.
        backup: PathBuf,
    },
    /// List the kit slots (index + name) in the backup.
    Kits {
        /// A TR backup file.
        backup: PathBuf,
        /// Include empty (unnamed) slots.
        #[arg(long)]
        all: bool,
    },
    /// Show one kit: name + its 6 voices with resolved tone names.
    Kit {
        /// A TR backup file.
        backup: PathBuf,
        /// 1-based kit slot number.
        number: usize,
    },
    /// List patterns (index, name, tempo, kit reference).
    Patterns {
        /// A TR backup file.
        backup: PathBuf,
        /// Include empty (unnamed) slots.
        #[arg(long)]
        all: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Info { backup } => info(&backup),
        Command::Verify { backup } => verify(&backup),
        Command::Kits { backup, all } => kits(&backup, all),
        Command::Kit { backup, number } => kit(&backup, number),
        Command::Patterns { backup, all } => patterns(&backup, all),
    }
}

fn patterns(path: &std::path::Path, all: bool) -> Result<()> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let b = Backup::parse(bytes)?;
    let raw = b.raw();
    let mut shown = 0;
    println!("  {:>3}  {:<18} {:>7}  KIT", "#", "NAME", "TEMPO");
    for p in b.patterns() {
        let name = p.name(raw);
        if name.is_empty() && !all {
            continue;
        }
        println!(
            "  {:>3}  {:<18} {:>6.1}  {:>3}",
            p.index + 1,
            name,
            p.tempo_bpm(raw),
            p.kit_ref(raw)
        );
        shown += 1;
    }
    println!("{shown} pattern(s)");
    Ok(())
}

fn kit(path: &std::path::Path, number: usize) -> Result<()> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let b = Backup::parse(bytes)?;
    let raw = b.raw();
    let kits = b.kits();
    let k = kits
        .get(number.wrapping_sub(1))
        .with_context(|| format!("kit {number} out of range (1..={})", kits.len()))?;
    println!("Kit {number}: {}", k.name(raw));
    println!(
        "  {:<3} {:>4} {:<18} {:>4} {:>5} {:>4} {:>4} {:>4} {:>4}",
        "V", "tone", "name", "tune", "decay", "lvl", "gain", "pan", "rev/dly"
    );
    let names = b.voice_names();
    for (voice, vp) in names.iter().zip(k.voices_n(raw, names.len())) {
        let tone = b.tone_name(vp.tone).unwrap_or_default();
        println!(
            "  {:<3} {:>4} {:<18} {:>4} {:>5} {:>4} {:>4} {:>4} {:>2}/{:<2}",
            voice,
            vp.tone,
            tone,
            vp.tune,
            vp.decay,
            vp.level,
            vp.gain,
            vp.pan,
            vp.reverb_send,
            vp.delay_send
        );
    }
    Ok(())
}

fn kits(path: &std::path::Path, all: bool) -> Result<()> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let b = Backup::parse(bytes)?;
    let raw = b.raw();
    let mut shown = 0;
    for k in b.kits() {
        let name = k.name(raw);
        if name.is_empty() && !all {
            continue;
        }
        println!("  {:>3}  {}", k.index + 1, name);
        shown += 1;
    }
    println!("{} kit slot(s)", shown);
    Ok(())
}

fn info(path: &std::path::Path) -> Result<()> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let total = bytes.len();
    let b = Backup::parse(bytes)?;
    println!("file:    {}", path.display());
    println!(
        "magic:   {}   version: {}   size: {} bytes",
        String::from_utf8_lossy(&b.magic()),
        b.version(),
        total
    );
    println!("sections:");
    println!("  {:<6} {:>10} {:>12}  SHAPE", "TAG", "OFFSET", "PAYLOAD");
    for s in b.sections() {
        let shape = match s.array_shape(b.raw()) {
            Some((count, rec)) => format!("{count} records x {rec} bytes"),
            None => String::new(),
        };
        println!(
            "  {:<6} 0x{:08x} {:>12}  {}",
            s.tag_str(),
            s.header_offset,
            s.payload_len,
            shape
        );
    }
    Ok(())
}

fn verify(path: &std::path::Path) -> Result<()> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let original = bytes.clone();
    let b = Backup::parse(bytes)?;
    if b.to_bytes() == original {
        println!("OK: {} round-trips byte-for-byte", path.display());
        Ok(())
    } else {
        anyhow::bail!("round-trip MISMATCH for {}", path.display());
    }
}
