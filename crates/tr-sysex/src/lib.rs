//! Roland **RQ1 / DT1** SysEx for the TR-6S / TR-8S.
//!
//! Roland's editor apps talk to these devices with two universal-format SysEx
//! commands (confirmed present in TR Editor as `FKoaSendRq1` / `FKoaSendDt1` /
//! `FKoaRequestRq1Dt1` — see `docs/tr-format.md`):
//!
//! * **RQ1** (`0x11`) — *data request*: "send me N bytes starting at address A".
//!   A **read primitive**: the device answers with a DT1 carrying the data.
//! * **DT1** (`0x12`) — *data set*: "write this data at address A".
//!
//! Message framing is the classic Roland form:
//!
//! ```text
//! F0 41 <dev> <model...> <cmd> <addr...> <body...> <checksum> F7
//!  │   │    │      │       │      │          │          │      └ end of SysEx
//!  │   │    │      │       │      │          │          └ Roland checksum (1 byte)
//!  │   │    │      │       │      │          └ RQ1: 4-byte size · DT1: data bytes
//!  │   │    │      │       │      └ 4-byte address (each byte 7-bit)
//!  │   │    │      │       └ command id (RQ1=0x11, DT1=0x12)
//!  │   │    │      └ model id (device-specific, see ModelId)
//!  │   │    └ device id (0x10 default; 0x00..=0x1F)
//!  │   └ Roland manufacturer id
//!  └ SysEx start
//! ```
//!
//! ## What is confirmed vs. what is not
//!
//! - **CONFIRMED (universal Roland):** the `F0 41 … F7` framing, the RQ1/DT1
//!   command bytes, the checksum algorithm (`ML::CRolandMessage::CheckSum` =
//!   sum, negate, `& 0x7F`), and that addresses/values are carried as 7-bit
//!   bytes. This crate implements all of that exactly and tests it against the
//!   published Roland worked example.
//! - **NOT yet confirmed (device-specific):** the TR-6S/TR-8S **model id** bytes
//!   and the **full address map**. Those come from decompiling TR Editor's
//!   RQ1/DT1 senders (the `TREditor_x86_64` image in the Ghidra project) — an
//!   open task. The section *base* addresses we do know from `Script.xml`
//!   (`kit=0x03000000`, `ptn=0x04000000`) are provided as [`addr`] constants,
//!   clearly labelled. [`ModelId::TR6S_PLACEHOLDER`] is a stand-in, not a real
//!   id — do not send it to hardware expecting a reply.
//!
//! ## Scope / safety
//!
//! This crate **only builds and parses byte strings**. It performs no MIDI I/O
//! and never touches a device — consistent with the project's file-based,
//! no-write-to-device stance. Actually transmitting these over USB-MIDI (and
//! especially any DT1 *write*) is a separate, hardware-gated step.

use std::fmt;

/// Roland's MIDI manufacturer id.
pub const ROLAND_ID: u8 = 0x41;

/// SysEx delimiters.
pub const SYSEX_START: u8 = 0xF0;
pub const SYSEX_END: u8 = 0xF7;

/// Default "all devices / unit 17" device id used by most Roland editors.
pub const DEFAULT_DEVICE_ID: u8 = 0x10;

/// Command ids (the "universal system exclusive" Roland subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// RQ1 — request `size` bytes at an address.
    Rq1,
    /// DT1 — set (write) data at an address.
    Dt1,
}

impl Command {
    pub fn id(self) -> u8 {
        match self {
            Command::Rq1 => 0x11,
            Command::Dt1 => 0x12,
        }
    }

    pub fn from_id(id: u8) -> Option<Command> {
        match id {
            0x11 => Some(Command::Rq1),
            0x12 => Some(Command::Dt1),
            _ => None,
        }
    }
}

/// A Roland model identifier. Length is device-specific: legacy GS devices use a
/// single byte, modern ZEN-Core-era devices use a 4-byte id (`00 00 00 xx`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelId(pub Vec<u8>);

impl ModelId {
    /// **PLACEHOLDER, not a real id.** The genuine TR-6S model id has not been
    /// recovered yet (needs TR Editor decompilation). Using the common
    /// modern-Roland 4-byte shape so message *sizes* are realistic; the final
    /// byte is deliberately `0x00` to make it obvious this is unresolved.
    ///
    /// Tracked as an open question in `docs/sysex.md`.
    pub const TR6S_PLACEHOLDER: [u8; 4] = [0x00, 0x00, 0x00, 0x00];

    pub fn new(bytes: impl Into<Vec<u8>>) -> ModelId {
        ModelId(bytes.into())
    }
}

/// The 4-byte, 7-bit device address used by modern Roland gear. Each byte is in
/// `0x00..=0x7F`; the four together address a 28-bit space. The section base
/// addresses in [`addr`] slot straight into this from `Script.xml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Address(pub [u8; 4]);

impl Address {
    /// Build from four raw bytes, validating the 7-bit constraint.
    pub fn from_bytes(b: [u8; 4]) -> Result<Address, SysexError> {
        for &x in &b {
            if x > 0x7F {
                return Err(SysexError::NotSevenBit(x));
            }
        }
        Ok(Address(b))
    }

    /// Interpret a plain 28-bit integer as a 4×7-bit Roland address.
    ///
    /// This matches how `Script.xml` writes section addresses: `0x03000000` for
    /// the kit section is the address bytes `[0x03, 0x00, 0x00, 0x00]` — i.e.
    /// each hex pair `00` is one 7-bit field, not a byte-packed integer. Values
    /// with any nibble > `0x7F` per field are rejected.
    pub fn from_script_u32(v: u32) -> Result<Address, SysexError> {
        let b = [
            ((v >> 24) & 0xFF) as u8,
            ((v >> 16) & 0xFF) as u8,
            ((v >> 8) & 0xFF) as u8,
            (v & 0xFF) as u8,
        ];
        Address::from_bytes(b)
    }

    /// Add a byte offset with correct 7-bit carry (Roland addresses carry at
    /// 128, not 256). Returns `None` on overflow past the 4-field space.
    pub fn offset(self, mut delta: u32) -> Option<Address> {
        let mut b = self.0;
        for i in (0..4).rev() {
            let sum = b[i] as u32 + (delta & 0x7F);
            b[i] = (sum & 0x7F) as u8;
            delta = (delta >> 7) + (sum >> 7);
            if delta == 0 {
                return Some(Address(b));
            }
        }
        if delta == 0 {
            Some(Address(b))
        } else {
            None
        }
    }

    pub fn bytes(self) -> [u8; 4] {
        self.0
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:02X} {:02X} {:02X} {:02X}",
            self.0[0], self.0[1], self.0[2], self.0[3]
        )
    }
}

/// Errors from building or parsing a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SysexError {
    /// A byte that must be 7-bit (address/data/checksum) exceeded `0x7F`.
    NotSevenBit(u8),
    TooShort,
    BadStart(u8),
    BadEnd(u8),
    NotRoland(u8),
    UnknownCommand(u8),
    /// Stored checksum did not match the recomputed one (`expected`, `found`).
    BadChecksum {
        expected: u8,
        found: u8,
    },
    /// RQ1 body must be exactly a 4-byte size request.
    BadRequestSize(usize),
}

impl fmt::Display for SysexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SysexError::NotSevenBit(b) => write!(f, "byte 0x{b:02X} is not 7-bit (>0x7F)"),
            SysexError::TooShort => write!(f, "message too short to be a valid Roland SysEx"),
            SysexError::BadStart(b) => write!(f, "expected F0 start, found 0x{b:02X}"),
            SysexError::BadEnd(b) => write!(f, "expected F7 end, found 0x{b:02X}"),
            SysexError::NotRoland(b) => write!(f, "expected Roland id 0x41, found 0x{b:02X}"),
            SysexError::UnknownCommand(b) => {
                write!(f, "unknown command id 0x{b:02X} (want RQ1 0x11 / DT1 0x12)")
            }
            SysexError::BadChecksum { expected, found } => {
                write!(
                    f,
                    "checksum mismatch: expected 0x{expected:02X}, found 0x{found:02X}"
                )
            }
            SysexError::BadRequestSize(n) => {
                write!(f, "RQ1 body must be 4 bytes (size), found {n}")
            }
        }
    }
}

impl std::error::Error for SysexError {}

/// The Roland checksum: sum the address + body bytes, then the checksum is the
/// value that makes `(sum + checksum) % 128 == 0`. Equivalent to
/// `(128 - (sum % 128)) % 128`. This is exactly `ML::CRolandMessage::CheckSum`
/// (sum the regions, negate, mask to 7 bits) seen in TR Editor.
pub fn roland_checksum(bytes: &[u8]) -> u8 {
    let sum: u32 = bytes.iter().map(|&b| b as u32).sum();
    ((128 - (sum % 128)) % 128) as u8
}

/// A parsed / buildable RQ1 or DT1 message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub device_id: u8,
    pub model_id: ModelId,
    pub command: Command,
    pub address: Address,
    /// For RQ1 this is the 4-byte requested size; for DT1 it is the data bytes.
    pub body: Vec<u8>,
}

impl Message {
    /// Build an RQ1 data-request for `size` bytes at `address`.
    ///
    /// `size` is encoded as a 4×7-bit field, the same shape as the address.
    pub fn rq1(device_id: u8, model_id: ModelId, address: Address, size: u32) -> Message {
        Message {
            device_id,
            model_id,
            command: Command::Rq1,
            address,
            body: encode_size4(size).to_vec(),
        }
    }

    /// Build a DT1 data-set writing `data` at `address`. Every data byte must be
    /// 7-bit; call [`Message::checked`] or [`Message::encode`] which validate.
    pub fn dt1(
        device_id: u8,
        model_id: ModelId,
        address: Address,
        data: impl Into<Vec<u8>>,
    ) -> Message {
        Message {
            device_id,
            model_id,
            command: Command::Dt1,
            address,
            body: data.into(),
        }
    }

    /// The bytes the checksum is computed over: address followed by body.
    fn checksum_region(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(4 + self.body.len());
        v.extend_from_slice(&self.address.0);
        v.extend_from_slice(&self.body);
        v
    }

    /// The checksum this message will carry.
    pub fn checksum(&self) -> u8 {
        roland_checksum(&self.checksum_region())
    }

    /// Validate 7-bit constraints on address + body without encoding.
    pub fn checked(&self) -> Result<(), SysexError> {
        for &b in self.address.0.iter().chain(self.body.iter()) {
            if b > 0x7F {
                return Err(SysexError::NotSevenBit(b));
            }
        }
        if self.command == Command::Rq1 && self.body.len() != 4 {
            return Err(SysexError::BadRequestSize(self.body.len()));
        }
        Ok(())
    }

    /// Encode to the full `F0 … F7` byte string.
    pub fn encode(&self) -> Result<Vec<u8>, SysexError> {
        self.checked()?;
        let mut out = Vec::new();
        out.push(SYSEX_START);
        out.push(ROLAND_ID);
        out.push(self.device_id);
        out.extend_from_slice(&self.model_id.0);
        out.push(self.command.id());
        out.extend_from_slice(&self.address.0);
        out.extend_from_slice(&self.body);
        out.push(self.checksum());
        out.push(SYSEX_END);
        Ok(out)
    }

    /// Parse a full SysEx byte string. `model_id_len` disambiguates the model-id
    /// field length (1 for legacy, 4 for modern); pass the length you expect.
    pub fn parse(bytes: &[u8], model_id_len: usize) -> Result<Message, SysexError> {
        // Minimum: F0 41 dev [model..] cmd addr(4) checksum F7
        let fixed = 2 /*F0 41*/ + 1 /*dev*/ + model_id_len + 1 /*cmd*/ + 4 /*addr*/ + 1 /*sum*/ + 1 /*F7*/;
        if bytes.len() < fixed {
            return Err(SysexError::TooShort);
        }
        if bytes[0] != SYSEX_START {
            return Err(SysexError::BadStart(bytes[0]));
        }
        if *bytes.last().unwrap() != SYSEX_END {
            return Err(SysexError::BadEnd(*bytes.last().unwrap()));
        }
        if bytes[1] != ROLAND_ID {
            return Err(SysexError::NotRoland(bytes[1]));
        }

        let mut i = 2;
        let device_id = bytes[i];
        i += 1;
        let model_id = ModelId(bytes[i..i + model_id_len].to_vec());
        i += model_id_len;
        let command = Command::from_id(bytes[i]).ok_or(SysexError::UnknownCommand(bytes[i]))?;
        i += 1;

        let addr = Address::from_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]])?;
        i += 4;

        // Body runs from here up to the checksum byte (second-to-last).
        let sum_idx = bytes.len() - 2;
        let body = bytes[i..sum_idx].to_vec();
        let found = bytes[sum_idx];

        let msg = Message {
            device_id,
            model_id,
            command,
            address: addr,
            body,
        };
        let expected = msg.checksum();
        if expected != found {
            return Err(SysexError::BadChecksum { expected, found });
        }
        msg.checked()?;
        Ok(msg)
    }
}

/// Encode a size/length as Roland's 4×7-bit big-endian field.
pub fn encode_size4(v: u32) -> [u8; 4] {
    [
        ((v >> 21) & 0x7F) as u8,
        ((v >> 14) & 0x7F) as u8,
        ((v >> 7) & 0x7F) as u8,
        (v & 0x7F) as u8,
    ]
}

/// Inverse of [`encode_size4`].
pub fn decode_size4(b: [u8; 4]) -> u32 {
    ((b[0] as u32) << 21) | ((b[1] as u32) << 14) | ((b[2] as u32) << 7) | (b[3] as u32)
}

/// Encode an unsigned integer into `n` 7-bit big-endian bytes — the packing used
/// for multi-byte device parameters. `n` should come from
/// [`tr_format::schema_value_size`] for a given `Script.xml` field, which is why
/// this crate depends on `tr-format`: both agree on how a value spans 7-bit MIDI.
pub fn encode_value_7bit(mut v: u32, n: usize) -> Vec<u8> {
    let mut out = vec![0u8; n];
    for slot in out.iter_mut().rev() {
        *slot = (v & 0x7F) as u8;
        v >>= 7;
    }
    out
}

/// Inverse of [`encode_value_7bit`].
pub fn decode_value_7bit(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .fold(0u32, |acc, &b| (acc << 7) | (b as u32 & 0x7F))
}

/// Known section base addresses, from `Script.xml` (see `docs/tr-format.md`).
/// These are the *only* addresses currently confirmed; the per-field addresses
/// within each section come from the same schema and are resolved by callers via
/// `tr-format`.
pub mod addr {
    use super::Address;

    /// Kit section base — `Script.xml` `kit = 03 00 00 00`.
    pub const KIT: Address = Address([0x03, 0x00, 0x00, 0x00]);
    /// Pattern section base — `Script.xml` `ptn = 04 00 00 00`.
    pub const PATTERN: Address = Address([0x04, 0x00, 0x00, 0x00]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_matches_roland_worked_example() {
        // Roland's own documented example (SC-88 manual): address 40 00 7F,
        // data 00 -> checksum 41. The algorithm is identical regardless of
        // address width, so this pins roland_checksum exactly.
        assert_eq!(roland_checksum(&[0x40, 0x00, 0x7F, 0x00]), 0x41);
    }

    #[test]
    fn checksum_makes_total_multiple_of_128() {
        let region = [0x03, 0x00, 0x10, 0x2A, 0x55, 0x7F];
        let ck = roland_checksum(&region);
        let total: u32 = region.iter().map(|&b| b as u32).sum::<u32>() + ck as u32;
        assert_eq!(total % 128, 0);
    }

    #[test]
    fn rq1_roundtrip() {
        let m = Message::rq1(
            DEFAULT_DEVICE_ID,
            ModelId::new(ModelId::TR6S_PLACEHOLDER.to_vec()),
            addr::KIT,
            0x520, // one kit record
        );
        let bytes = m.encode().unwrap();
        assert_eq!(bytes[0], SYSEX_START);
        assert_eq!(bytes[1], ROLAND_ID);
        assert_eq!(*bytes.last().unwrap(), SYSEX_END);
        let back = Message::parse(&bytes, 4).unwrap();
        assert_eq!(back, m);
        assert_eq!(back.command, Command::Rq1);
        assert_eq!(
            decode_size4([back.body[0], back.body[1], back.body[2], back.body[3]]),
            0x520
        );
    }

    #[test]
    fn dt1_roundtrip() {
        let m = Message::dt1(
            DEFAULT_DEVICE_ID,
            ModelId::new(vec![0x00, 0x00, 0x00, 0x42]),
            addr::PATTERN,
            vec![0x01, 0x02, 0x03, 0x7F],
        );
        let bytes = m.encode().unwrap();
        let back = Message::parse(&bytes, 4).unwrap();
        assert_eq!(back, m);
        assert_eq!(back.command, Command::Dt1);
    }

    #[test]
    fn parse_rejects_bad_checksum() {
        let m = Message::rq1(0x10, ModelId::new(vec![0, 0, 0, 0]), addr::KIT, 8);
        let mut bytes = m.encode().unwrap();
        let sum_idx = bytes.len() - 2;
        bytes[sum_idx] ^= 0x01; // corrupt checksum
        match Message::parse(&bytes, 4) {
            Err(SysexError::BadChecksum { .. }) => {}
            other => panic!("expected BadChecksum, got {other:?}"),
        }
    }

    #[test]
    fn encode_rejects_non_7bit_data() {
        let m = Message::dt1(0x10, ModelId::new(vec![0]), addr::KIT, vec![0x80]);
        assert_eq!(m.encode(), Err(SysexError::NotSevenBit(0x80)));
    }

    #[test]
    fn address_7bit_carry() {
        // 0x00 0x00 0x00 0x7F + 1 -> 0x00 0x00 0x01 0x00 (carry at 128).
        let a = Address([0x00, 0x00, 0x00, 0x7F]).offset(1).unwrap();
        assert_eq!(a.bytes(), [0x00, 0x00, 0x01, 0x00]);
    }

    #[test]
    fn size4_roundtrip() {
        for v in [0u32, 1, 0x7F, 0x80, 0x520, 0x0FFF_FFFF] {
            assert_eq!(decode_size4(encode_size4(v)), v);
        }
    }

    #[test]
    fn value_7bit_matches_tr_format_sizing() {
        // A 0–1023 field ("int4x4") is 2 bytes per tr-format's schema rule; a
        // value must round-trip through that width.
        let n = tr_format::schema_value_size("int4x4", 1023).unwrap();
        assert_eq!(n, 2);
        assert_eq!(decode_value_7bit(&encode_value_7bit(1000, n)), 1000);
    }
}
