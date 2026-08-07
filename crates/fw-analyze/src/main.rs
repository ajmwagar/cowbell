//! `fw-analyze` — firmware image analysis CLI for the cowbell TR-6S project.
//!
//! This tool is deliberately native-Rust for the simple, deterministic work
//! (entropy, hex dump, checksum brute-forcing, diffing) and shells out to
//! `binwalk` for the heavy signature/entropy carving when it is installed.
//!
//! Nothing here writes to a device. It is read-only analysis of image files
//! the maintainer has dumped from hardware they own.

mod blockmode;
mod checksum;
mod diff;
mod entropy;
mod hexdump;
mod magic;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "fw-analyze",
    version,
    about = "Firmware image analysis for the cowbell TR-6S research project",
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Sliding-window Shannon entropy scan (spots compressed/encrypted regions).
    Entropy {
        /// Firmware image to scan.
        image: PathBuf,
        /// Window size in bytes.
        #[arg(long, default_value_t = 4096)]
        window: usize,
        /// Step between windows in bytes (defaults to the window size).
        #[arg(long)]
        step: Option<usize>,
    },

    /// Inspect the leading bytes and match against a table of known magics.
    Inspect {
        /// Firmware image to inspect.
        image: PathBuf,
        /// Also scan the whole file for embedded magics, not just offset 0.
        #[arg(long)]
        scan: bool,
    },

    /// Classic `hexdump -C`-style canonical hex + ASCII dump.
    Hexdump {
        /// Firmware image to dump.
        image: PathBuf,
        /// Start offset in bytes.
        #[arg(long, default_value_t = 0)]
        offset: u64,
        /// Number of bytes to dump (default: whole file).
        #[arg(long)]
        len: Option<u64>,
    },

    /// Compute or brute-force common checksums/CRCs over a file or range.
    Checksum {
        /// Firmware image.
        image: PathBuf,
        /// Region to checksum, start offset.
        #[arg(long, default_value_t = 0)]
        offset: u64,
        /// Region length (default: to end of file).
        #[arg(long)]
        len: Option<u64>,
        /// A known/expected value (hex, e.g. 0x1a2b) to search algorithms for.
        #[arg(long, value_parser = parse_hex_u32)]
        expect: Option<u32>,
    },

    /// Diff two firmware images: byte-level runs, or block-aligned ("ECB").
    ///
    /// Default (byte mode) reports contiguous runs of differing bytes. With
    /// `--block N` (N>0) it instead runs block-aligned analysis at block size
    /// N (typical 8): per-image duplicate-block ratio, set-based and
    /// positional block overlap, and the longest same-offset identical run —
    /// the signatures that reveal shared ECB-encrypted content and a shared
    /// key. `--offset O` starts both images at byte O first (e.g. to skip the
    /// ~96-byte plaintext header and compare only the encrypted body).
    Diff {
        /// First image.
        a: PathBuf,
        /// Second image.
        b: PathBuf,
        /// Cap the number of differing (or identical-run) entries printed.
        #[arg(long, default_value_t = 64)]
        max_runs: usize,
        /// Block size in bytes for block-aligned analysis. 0 = byte diff.
        #[arg(long, default_value_t = 0)]
        block: usize,
        /// Start both images at this byte offset before comparing.
        #[arg(long, default_value_t = 0)]
        offset: u64,
    },

    /// Block-cipher mode analysis: ECB fingerprint vs. a keystream/XOR pad.
    Blockmode {
        /// Firmware image (or carved ciphertext region).
        image: PathBuf,
        /// Start offset of the ciphertext region.
        #[arg(long, default_value_t = 0)]
        offset: u64,
        /// Length of the region (default: to end of file).
        #[arg(long)]
        len: Option<u64>,
        /// Candidate cipher block size in bytes.
        #[arg(long, default_value_t = 8)]
        block: usize,
    },

    /// Shell out to `binwalk` if it is installed (signature scan by default).
    Binwalk {
        /// Firmware image.
        image: PathBuf,
        /// Extra args passed through to binwalk verbatim.
        #[arg(last = true)]
        args: Vec<String>,
    },
}

fn parse_hex_u32(s: &str) -> Result<u32, String> {
    let t = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    u32::from_str_radix(t, 16).map_err(|e| format!("invalid hex value `{s}`: {e}"))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Entropy {
            image,
            window,
            step,
        } => entropy::run(&image, window, step.unwrap_or(window)),
        Command::Inspect { image, scan } => magic::run(&image, scan),
        Command::Blockmode {
            image,
            offset,
            len,
            block,
        } => blockmode::run(&image, offset, len, block),
        Command::Hexdump { image, offset, len } => hexdump::run(&image, offset, len),
        Command::Checksum {
            image,
            offset,
            len,
            expect,
        } => checksum::run(&image, offset, len, expect),
        Command::Diff {
            a,
            b,
            max_runs,
            block,
            offset,
        } => diff::run(&a, &b, max_runs, block, offset),
        Command::Binwalk { image, args } => run_binwalk(&image, &args),
    }
}

/// Thin wrapper over the `binwalk` binary. We intentionally do not reimplement
/// binwalk's signature database; we just make it convenient inside the same CLI.
fn run_binwalk(image: &std::path::Path, extra: &[String]) -> Result<()> {
    use std::process::Command as PCommand;

    let mut cmd = PCommand::new("binwalk");
    if extra.is_empty() {
        // Default: signature scan.
        cmd.arg(image);
    } else {
        cmd.args(extra).arg(image);
    }

    match cmd.status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => anyhow::bail!("binwalk exited with {status}"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            anyhow::bail!(
                "binwalk not found on PATH. Install it (e.g. `pipx install binwalk` \
                 or your distro package) or use the native `entropy`/`inspect` \
                 subcommands instead."
            )
        }
        Err(e) => Err(e.into()),
    }
}
