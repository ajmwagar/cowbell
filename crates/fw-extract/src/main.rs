//! `fw-extract` — unpack a Roland TR-6S updater package and pull out the raw
//! firmware payload for downstream analysis with `fw-analyze`.
//!
//! Roland ships the updater as a platform-specific installer:
//!   * Windows — a self-extracting `.exe` (historically InstallShield/NSIS) or
//!     an `.msi` (an OLE/CFB compound file).
//!   * macOS — a `.dmg` disk image containing a `.app`, or a flat `.pkg`
//!     (a `xar!` archive).
//!
//! Rather than reimplement every installer format, this tool:
//!   1. `identify` — sniffs the container type from magic bytes so you know
//!      which external unpacker to reach for.
//!   2. `unpack`   — shells out to the right extractor if it is installed
//!      (7z / xar / msiextract / binwalk), dumping everything to an output dir.
//!   3. `carve`    — a native fallback that scans an already-unpacked tree (or
//!      any blob) for candidate firmware payloads by size + entropy heuristics,
//!      so you can find the `.bin` even when it has no helpful extension.
//!
//! IMPORTANT: this only ever reads installer files the maintainer already has.
//! It never downloads firmware and never writes to a device.

mod carve;
mod container;
mod lzss;
mod unpack;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "fw-extract",
    version,
    about = "Unpack Roland TR-6S updater packages and carve out the firmware payload",
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Sniff an installer file and report its container type + suggested tool.
    Identify {
        /// The Roland updater package (.exe/.msi/.dmg/.pkg/...).
        package: PathBuf,
    },

    /// Shell out to the appropriate extractor to unpack the installer.
    Unpack {
        /// The Roland updater package.
        package: PathBuf,
        /// Directory to extract into (created if missing).
        #[arg(long, default_value = "extracted")]
        out: PathBuf,
        /// Print the command that would run, but do not run it.
        #[arg(long)]
        dry_run: bool,
    },

    /// Scan an unpacked tree (or a blob) for candidate firmware payloads.
    Carve {
        /// A file or directory to scan.
        path: PathBuf,
        /// Minimum candidate size in bytes.
        #[arg(long, default_value_t = 1 << 20)]
        min_size: u64,
        /// Maximum candidate size in bytes (0 = no limit).
        #[arg(long, default_value_t = 0)]
        max_size: u64,
    },

    /// Decompress a Roland `init_param` file (LZSS) to its factory-default
    /// backup image — a `tr-format`-parseable `TR6S` container.
    InitParam {
        /// An `init_param` file (e.g. `dd001_init_param.bin`).
        file: PathBuf,
        /// Where to write the decompressed backup.
        #[arg(long, default_value = "factory_backup.bin")]
        out: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Identify { package } => container::run_identify(&package),
        Command::Unpack {
            package,
            out,
            dry_run,
        } => unpack::run(&package, &out, dry_run),
        Command::Carve {
            path,
            min_size,
            max_size,
        } => carve::run(
            &path,
            min_size,
            if max_size == 0 { u64::MAX } else { max_size },
        ),
        Command::InitParam { file, out } => {
            let raw = std::fs::read(&file)
                .map_err(|e| anyhow::anyhow!("reading {}: {e}", file.display()))?;
            let backup = lzss::decompress_init_param(&raw)?;
            std::fs::write(&out, &backup)
                .map_err(|e| anyhow::anyhow!("writing {}: {e}", out.display()))?;
            let magic = String::from_utf8_lossy(&backup[..4]);
            println!(
                "decompressed {} bytes -> {} (magic {magic})",
                backup.len(),
                out.display()
            );
            Ok(())
        }
    }
}
