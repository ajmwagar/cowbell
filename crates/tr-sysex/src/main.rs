//! `tr-sysex` — build and inspect Roland RQ1/DT1 SysEx for the TR-6S/TR-8S.
//!
//! Message construction and analysis only. This binary opens **no MIDI port**
//! and sends nothing to a device; it prints byte strings you can inspect, diff,
//! or hand to a separate (hardware-gated) transport later. Generating a DT1 (a
//! *write* message) still only prints — treat producing one as intent to modify
//! a device and gate actual transmission on confirming the update path.

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use tr_sysex::address::{self, RolandAddress};
use tr_sysex::{roland_checksum, DeviceConfig, CMD_DT1, CMD_RQ1};

/// A stand-in model id. The real TR-6S/TR-8S bytes are unconfirmed (see the
/// crate docs); this is obviously fake and the CLI warns when it is used.
const PLACEHOLDER_MODEL_ID: &[u8] = &[0x00];

#[derive(Parser)]
#[command(
    name = "tr-sysex",
    version,
    about = "Roland RQ1/DT1 SysEx builder/parser for the TR-6S/TR-8S (no MIDI I/O)",
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Build an RQ1 data-request (read) message and print it as hex.
    Rq1 {
        /// Address: 4 hex bytes ("10 00 00 00" / "10000000"), or a device-region
        /// name (`kit` / `pattern` / `tone` / `sys`).
        address: String,
        /// Number of bytes to request.
        #[arg(long, value_parser = parse_u32_auto)]
        size: u32,
        #[command(flatten)]
        common: Common,
    },

    /// Build a DT1 data-set (write) message and print it as hex.
    ///
    /// A DT1 is a *write*. This only prints bytes — it does not send them — but
    /// treat generating one as intent to modify a device.
    Dt1 {
        /// Address: 4 hex bytes, or `kit` / `pattern` / `tone` / `sys`.
        address: String,
        /// Data payload as hex bytes (each must be 7-bit, `<= 0x7F`).
        #[arg(long)]
        data: String,
        #[command(flatten)]
        common: Common,
    },

    /// Parse a SysEx byte string (hex, or `@path` to a binary) and describe it.
    Parse {
        /// Hex bytes ("F0 41 ...") or "@file" to read raw bytes from a file.
        input: String,
        /// Model-id field length to assume when splitting the prefix.
        #[arg(long, default_value_t = 1)]
        model_len: usize,
    },

    /// Compute the Roland checksum over a run of hex bytes (address + data).
    Checksum {
        /// Hex bytes the checksum covers.
        bytes: String,
    },
}

#[derive(Args)]
struct Common {
    /// Device id (0x00..=0x1F).
    #[arg(long, value_parser = parse_u8_auto, default_value = "0x10")]
    device: u8,
    /// Model id as hex bytes. Defaults to a PLACEHOLDER (not the real, still
    /// unconfirmed, TR-6S/TR-8S id).
    #[arg(long)]
    model: Option<String>,
}

impl Common {
    fn config(&self) -> Result<(DeviceConfig, bool)> {
        Ok(match &self.model {
            Some(s) => (DeviceConfig::new(self.device, parse_hex(s)?), false),
            None => (
                DeviceConfig::new(self.device, PLACEHOLDER_MODEL_ID.to_vec()),
                true,
            ),
        })
    }
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Cmd::Rq1 {
            address,
            size,
            common,
        } => {
            let (cfg, placeholder) = common.config()?;
            warn_placeholder(placeholder);
            println!("{}", to_hex(&cfg.build_rq1(parse_address(&address)?, size)));
        }
        Cmd::Dt1 {
            address,
            data,
            common,
        } => {
            let (cfg, placeholder) = common.config()?;
            warn_placeholder(placeholder);
            let data = parse_hex(&data)?;
            if let Some(&bad) = data.iter().find(|&&b| b > 0x7F) {
                bail!("DT1 data byte 0x{bad:02X} is not 7-bit-safe (must be <= 0x7F)");
            }
            println!(
                "{}",
                to_hex(&cfg.build_dt1(parse_address(&address)?, &data))
            );
        }
        Cmd::Parse { input, model_len } => parse_and_describe(&input, model_len)?,
        Cmd::Checksum { bytes } => println!("0x{:02X}", roland_checksum(&parse_hex(&bytes)?)),
    }
    Ok(())
}

fn warn_placeholder(used: bool) {
    if used {
        eprintln!(
            "warning: using a PLACEHOLDER model id (not the real TR-6S/TR-8S id, \
             which is unconfirmed). Pass --model once it is known. See \
             docs/device-sysex.md."
        );
    }
}

fn parse_and_describe(input: &str, model_len: usize) -> Result<()> {
    let bytes = match input.strip_prefix('@') {
        Some(path) => std::fs::read(path).with_context(|| format!("reading {path}"))?,
        None => parse_hex(input)?,
    };
    // The prefix is F0 41 <dev> <model...>; to split it we need model_len. Read
    // the device id and model-id bytes straight from the message, then let
    // DeviceConfig::parse validate framing + checksum against them.
    if bytes.len() < 3 + model_len {
        bail!("message too short for a {model_len}-byte model id");
    }
    let device_id = bytes[2];
    let model_id = bytes[3..3 + model_len].to_vec();
    let cfg = DeviceConfig::new(device_id, model_id);
    let msg = cfg.parse(&bytes)?;

    let cmd = match msg.command {
        CMD_RQ1 => "RQ1 (data request / read)",
        CMD_DT1 => "DT1 (data set / write)",
        other => bail!("unexpected command byte 0x{other:02X}"),
    };
    println!("command:   {cmd}");
    println!("device id: 0x{:02X}", msg.device_id);
    println!("model id:  {}", to_hex(&msg.model_id));
    println!("address:   {}", to_hex(&msg.address.bytes()));
    if msg.command == CMD_RQ1 {
        match msg.requested_len() {
            Some(n) => println!("request:   {n} (0x{n:X}) bytes"),
            None => println!(
                "request:   ({} body bytes, not a 4-byte length)",
                msg.data.len()
            ),
        }
    } else {
        println!("data:      {} byte(s)", msg.data.len());
        println!("           {}", to_hex(&msg.data));
    }
    println!("checksum:  valid");
    Ok(())
}

/// Resolve an address argument: a device-region name or 4 hex bytes. The names
/// map to the **device** SysEx region bases (see `address`), not the
/// editor-model ones.
fn parse_address(s: &str) -> Result<RolandAddress> {
    match s.trim().to_ascii_lowercase().as_str() {
        "kit" => Ok(address::KIT_NAME),
        "pattern" | "ptn" => Ok(address::PATTERN_NAME),
        "tone" => Ok(address::TONE_NAME),
        "sys" => Ok(address::SYS_CATEGORY_NAME.base),
        _ => {
            let b = parse_hex(s)?;
            if b.len() != 4 {
                bail!("address must be exactly 4 bytes, got {}", b.len());
            }
            Ok(RolandAddress::new([b[0], b[1], b[2], b[3]]))
        }
    }
}

/// Loose hex parse: tolerates spaces, commas, and `0x` prefixes.
fn parse_hex(s: &str) -> Result<Vec<u8>> {
    let cleaned: String = s
        .replace("0x", " ")
        .replace("0X", " ")
        .replace(',', " ")
        .split_whitespace()
        .collect();
    if cleaned.len() % 2 != 0 {
        bail!("hex string has an odd number of nibbles");
    }
    (0..cleaned.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&cleaned[i..i + 2], 16).context("invalid hex byte"))
        .collect()
}

fn parse_u8_auto(s: &str) -> Result<u8, String> {
    parse_u32_auto(s).and_then(|v| u8::try_from(v).map_err(|_| format!("{s} out of u8 range")))
}

fn parse_u32_auto(s: &str) -> Result<u32, String> {
    let t = s.trim();
    let r = t
        .strip_prefix("0x")
        .or_else(|| t.strip_prefix("0X"))
        .map(|h| u32::from_str_radix(h, 16))
        .unwrap_or_else(|| t.parse());
    r.map_err(|e| format!("invalid number `{s}`: {e}"))
}

fn to_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}
