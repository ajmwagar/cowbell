//! `tr-sysex` — build and inspect Roland RQ1/DT1 SysEx for the TR-6S/TR-8S.
//!
//! Message construction and analysis only. This binary never opens a MIDI port
//! and never sends anything to a device; it prints byte strings you can inspect,
//! diff, or hand to a separate (hardware-gated) transport later.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tr_sysex::{addr, decode_size4, roland_checksum, Address, Command, Message, ModelId};

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
        /// Address as 4 hex bytes, e.g. "03 00 00 00" or "03000000".
        /// Special names `kit` / `pattern` use the known Script.xml bases.
        address: String,
        /// Number of bytes to request.
        #[arg(long, value_parser = parse_u32_auto)]
        size: u32,
        #[command(flatten)]
        common: Common,
    },

    /// Build a DT1 data-set (write) message and print it as hex.
    ///
    /// NOTE: a DT1 is a *write*. This only prints bytes — it does not send them —
    /// but treat generating one as intent to modify a device and gate actual
    /// transmission on confirming the update path is safe.
    Dt1 {
        /// Address as 4 hex bytes or `kit` / `pattern`.
        address: String,
        /// Data payload as hex bytes (each must be 7-bit / <= 0x7F).
        #[arg(long)]
        data: String,
        #[command(flatten)]
        common: Common,
    },

    /// Parse a SysEx byte string (hex, or @path to a binary file) and describe it.
    Parse {
        /// Hex bytes ("F0 41 ...") or "@file" to read raw bytes from a file.
        input: String,
        /// Model-id field length to assume (1 legacy, 4 modern).
        #[arg(long, default_value_t = 4)]
        model_len: usize,
    },

    /// Compute the Roland checksum over a run of hex bytes (address + data).
    Checksum {
        /// Hex bytes the checksum covers.
        bytes: String,
    },
}

#[derive(clap::Args)]
struct Common {
    /// Device id (0x00..=0x1F).
    #[arg(long, value_parser = parse_u8_auto, default_value = "0x10")]
    device: u8,
    /// Model id as hex bytes. Defaults to the TR-6S placeholder (UNCONFIRMED).
    #[arg(long)]
    model: Option<String>,
}

impl Common {
    fn model_id(&self) -> Result<ModelId> {
        match &self.model {
            Some(s) => Ok(ModelId::new(parse_hex(s)?)),
            None => Ok(ModelId::new(ModelId::TR6S_PLACEHOLDER.to_vec())),
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Cmd::Rq1 {
            address,
            size,
            common,
        } => {
            let msg = Message::rq1(
                common.device,
                common.model_id()?,
                parse_address(&address)?,
                size,
            );
            print_built(&msg)?;
        }
        Cmd::Dt1 {
            address,
            data,
            common,
        } => {
            let data = parse_hex(&data)?;
            let msg = Message::dt1(
                common.device,
                common.model_id()?,
                parse_address(&address)?,
                data,
            );
            print_built(&msg)?;
        }
        Cmd::Parse { input, model_len } => parse_and_describe(&input, model_len)?,
        Cmd::Checksum { bytes } => {
            let b = parse_hex(&bytes)?;
            println!("0x{:02X}", roland_checksum(&b));
        }
    }
    Ok(())
}

fn print_built(msg: &Message) -> Result<()> {
    if msg.model_id.0 == ModelId::TR6S_PLACEHOLDER {
        eprintln!(
            "warning: using the PLACEHOLDER model id (not the real TR-6S id, which \
             is still unresolved). Pass --model once it is known. See docs/sysex.md."
        );
    }
    let bytes = msg.encode().context("encoding message")?;
    println!("{}", to_hex(&bytes));
    Ok(())
}

fn parse_and_describe(input: &str, model_len: usize) -> Result<()> {
    let bytes = if let Some(path) = input.strip_prefix('@') {
        std::fs::read(path).with_context(|| format!("reading {path}"))?
    } else {
        parse_hex(input)?
    };

    let msg = Message::parse(&bytes, model_len).map_err(|e| anyhow::anyhow!("{e}"))?;
    let cmd = match msg.command {
        Command::Rq1 => "RQ1 (data request / read)",
        Command::Dt1 => "DT1 (data set / write)",
    };
    println!("command:   {cmd}");
    println!("device id: 0x{:02X}", msg.device_id);
    println!("model id:  {}", to_hex(&msg.model_id.0));
    println!("address:   {}", msg.address);
    match msg.command {
        Command::Rq1 => {
            let sz = decode_size4([msg.body[0], msg.body[1], msg.body[2], msg.body[3]]);
            println!("request:   {sz} (0x{sz:X}) bytes");
        }
        Command::Dt1 => {
            println!("data:      {} byte(s)", msg.body.len());
            println!("           {}", to_hex(&msg.body));
        }
    }
    println!("checksum:  0x{:02X} (valid)", msg.checksum());
    Ok(())
}

fn parse_address(s: &str) -> Result<Address> {
    match s.trim().to_ascii_lowercase().as_str() {
        "kit" => Ok(addr::KIT),
        "pattern" | "ptn" => Ok(addr::PATTERN),
        _ => {
            let b = parse_hex(s)?;
            anyhow::ensure!(
                b.len() == 4,
                "address must be exactly 4 bytes, got {}",
                b.len()
            );
            Address::from_bytes([b[0], b[1], b[2], b[3]]).map_err(|e| anyhow::anyhow!("{e}"))
        }
    }
}

/// Parse a loose hex string: tolerates spaces, commas, and `0x` prefixes.
fn parse_hex(s: &str) -> Result<Vec<u8>> {
    let cleaned: String = s
        .replace("0x", " ")
        .replace("0X", " ")
        .replace(',', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("");
    anyhow::ensure!(
        cleaned.len() % 2 == 0,
        "hex string has an odd number of nibbles"
    );
    (0..cleaned.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&cleaned[i..i + 2], 16).context("invalid hex byte"))
        .collect()
}

fn parse_u8_auto(s: &str) -> Result<u8, String> {
    parse_u32_auto(s).and_then(|v| u8::try_from(v).map_err(|_| format!("{s} out of range for u8")))
}

fn parse_u32_auto(s: &str) -> Result<u32, String> {
    let t = s.trim();
    let r = if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        u32::from_str_radix(h, 16)
    } else {
        t.parse::<u32>()
    };
    r.map_err(|e| format!("invalid number `{s}`: {e}"))
}

fn to_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}
