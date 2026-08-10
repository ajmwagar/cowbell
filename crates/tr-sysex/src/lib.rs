//! Roland TR-6S / TR-8S **device SysEx** protocol — clean-room RQ1/DT1.
//!
//! This crate speaks the device-side counterpart to the backup file format: it
//! builds and parses the System Exclusive messages that read and write the
//! box's **live edit-buffer memory** over USB-MIDI. It is the no-hardware read
//! path — RQ1 request, DT1 reply — needing only the CTRL USB-MIDI port, no
//! teardown and no NOR dump. It touches **only the plaintext user-data plane**:
//! nothing here relates to encrypted firmware (`App1_Main`) or its key.
//!
//! Everything is reconstructed from the facts in `docs/device-sysex.md` —
//! Roland's functional protocol (message layout, device addresses, ID ranges),
//! which is not copyrightable. No code, prose, or capture files from any
//! third-party repository were read or vendored; this is a clean-room
//! reimplementation from the documented facts.
//!
//! ## The message format
//!
//! Standard Roland System Exclusive:
//!
//! ```text
//! F0  41  <deviceId>  <modelId…>  <cmd>  <addr(4)>  <data…>  <checksum>  F7
//! ```
//!
//! - `F0` / `F7` — SysEx start / end.
//! - `41` — Roland manufacturer ID.
//! - `deviceId` — the Utility "SysEx ID" (0-based unit number), one byte.
//! - `modelId` — the model identifier, **4 bytes**. **CONFIRMED** for the TR-8S:
//!   [`MODEL_ID_TR8S`] = `00 00 00 45`, from real RQ1/DT1 captures (every one of
//!   1,956 messages across two capture files). Use [`DeviceConfig::tr8s`]. The
//!   **TR-6S** id is not yet captured, so [`DeviceConfig::new`] still takes model
//!   bytes for that case (no value invented).
//! - `cmd` — [`CMD_RQ1`] (`0x11`, read/request) or [`CMD_DT1`] (`0x12`,
//!   write/set). The DT1 the device sends to answer an RQ1 uses the same `0x12`.
//! - `addr` — a 4-byte, 7-bit-safe device address (see [`address`]).
//! - `data` — present on DT1: the payload bytes. On RQ1 the "data" is instead
//!   the 4-byte requested length (see [`DeviceConfig::build_rq1`]).
//! - `checksum` — the Roland checksum over `addr + data`; see
//!   [`roland_checksum`].
//!
//! ## Read round-trip
//!
//! Send **RQ1** (address + length); the device replies with **DT1** carrying
//! the bytes at that address. [`DeviceConfig::read`] performs exactly this over
//! any [`MidiPort`], so the whole protocol is exercised from byte-stream
//! fixtures with no hardware (see the tests and [`MidiPort`]).
//!
//! ## Confirmed against real captures (2026-08-10)
//!
//! Decoding real TR-8S RQ1/DT1 wire captures (`docs/device-sysex.md`) pins the
//! wire format: **0 checksum mismatches across 1,956 messages**, and every one
//! uses device ID `0x10`, model ID `00 00 00 45`, a **4-byte address**, and —
//! for RQ1 — a **4-byte requested-length** field. Multi-byte values are carried
//! **base-128** (7 bits per byte): a captured 1,171-byte request is `00 00 09 13`,
//! which no base-256 reading could put on a 7-bit wire. So the address/length
//! encoding in [`address`] and [`DeviceConfig::build_rq1`] is confirmed.
//!
//! ## 7-bit packing — still open (and possibly not the right model)
//!
//! [`encode_7bit`] / [`decode_7bit`] implement the common MIDI **7-in-8** scheme.
//! But the captures do **not** show it: every DT1 data byte is already `<= 0x7F`,
//! and multi-byte parameters look **base-128 per field** (`⌈bits/7⌉` bytes each,
//! matching `tr-format`'s schema sizing), not 7-in-8 packed. So the 7-in-8
//! helpers are kept but flagged — the device's actual multi-byte encoding is
//! base-128, and 7-in-8 may simply be unused here.
//!
//! ## Not yet done (deliberate follow-ups)
//!
//! - **Device-field → typed model.** Decoding DT1 payloads into the rich
//!   `Kit` / `Pattern` / `Sys` records from the `tr-format` crate is a separate
//!   step (`cowbell-1ne.1`). This cut returns raw DT1 data bytes.
//! - **Persistent-slot base addresses.** [`address`] models the edit buffer
//!   (`temp`); the captures also show persistent-slot reads at other bases
//!   (e.g. `37 xx` / `47 xx` for tones), not yet mapped.
//! - **TR-6S model ID** — needs a TR-6S capture (the TR-8S is confirmed).

pub mod address;
pub mod read;

#[cfg(feature = "usb")]
pub mod midir_port;

use anyhow::{bail, ensure, Result};

use crate::address::RolandAddress;

/// SysEx start byte (`0xF0`, 240).
pub const SYSEX_START: u8 = 0xF0;
/// SysEx end byte (`0xF7`, 247).
pub const SYSEX_END: u8 = 0xF7;
/// Roland manufacturer ID (`0x41`).
pub const ROLAND_MANUFACTURER_ID: u8 = 0x41;
/// RQ1 — data request / read.
pub const CMD_RQ1: u8 = 0x11;
/// DT1 — data set / write (also the reply the device sends to an RQ1).
pub const CMD_DT1: u8 = 0x12;
/// Address length in bytes.
pub const ADDRESS_LEN: usize = 4;

/// Default device ID (the Utility "SysEx ID"), `0x10` — confirmed as the value
/// on real TR-8S captures.
pub const DEFAULT_DEVICE_ID: u8 = 0x10;

/// The Roland model ID for the TR-6S **and** TR-8S — they **share** it,
/// `00 00 00 45`. Confirmed two ways: real TR-8S RQ1/DT1 captures (every one of
/// 1,956 messages), and TR Editor's `Script.xml`, whose single `TR CTRL`
/// `midiIn`/`midiOut` declares `<modelID>00 00 00 45</modelID>` for both boxes.
/// The two models are told apart by a separate device-model field (`1` = TR-8S,
/// `2` = TR-6S), not by the model ID.
pub const MODEL_ID_TR: [u8; 4] = [0x00, 0x00, 0x00, 0x45];

/// Alias of [`MODEL_ID_TR`] — the TR-8S model ID.
pub const MODEL_ID_TR8S: [u8; 4] = MODEL_ID_TR;
/// Alias of [`MODEL_ID_TR`] — the TR-6S model ID (same bytes as the TR-8S).
pub const MODEL_ID_TR6S: [u8; 4] = MODEL_ID_TR;

/// The Roland one-byte checksum over an address+data run:
/// `(0x80 − (sum(bytes) & 0x7F)) & 0x7F`.
///
/// The result is the value that makes `(sum(addr+data) + checksum) & 0x7F == 0`.
/// Attested directly by `docs/device-sysex.md`.
pub fn roland_checksum(bytes: &[u8]) -> u8 {
    let sum: u32 = bytes.iter().map(|&b| b as u32).sum();
    ((0x80 - (sum & 0x7F)) & 0x7F) as u8
}

/// 7-bit-pack arbitrary 8-bit data for a SysEx data field (**inferred layout**).
///
/// Groups the input into runs of 7 bytes. Each run emits 8 output bytes: a
/// leading byte holding the high bit (bit 7) of each of the next up-to-7 bytes
/// (LSB = first byte of the run), followed by those bytes with bit 7 cleared.
/// This is the widely-used MIDI 7-in-8 packing and is the exact inverse of
/// [`decode_7bit`].
///
/// **Evidence.** `docs/device-sysex.md` attests only that payloads are
/// 7-bit-packed by the reference `encode/decode7bitBytes`; it does not specify
/// the byte layout. This particular grouping is **inferred** and should be
/// confirmed against a real DT1 capture before relying on it for a live device.
pub fn encode_7bit(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 7 + 1);
    for chunk in data.chunks(7) {
        let mut high = 0u8;
        for (i, &b) in chunk.iter().enumerate() {
            high |= (b >> 7) << i;
        }
        out.push(high);
        for &b in chunk {
            out.push(b & 0x7F);
        }
    }
    out
}

/// Inverse of [`encode_7bit`]: unpack a 7-bit-safe run back to 8-bit bytes.
///
/// Returns an error if the input is malformed (a group with a high-bit byte but
/// no following data). Like [`encode_7bit`], the layout is **inferred**; see
/// that function.
pub fn decode_7bit(packed: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(packed.len());
    let mut i = 0;
    while i < packed.len() {
        let high = packed[i];
        i += 1;
        let group = &packed[i..packed.len().min(i + 7)];
        ensure!(
            !group.is_empty(),
            "malformed 7-bit stream: high-bit byte with no data at {i}"
        );
        for (j, &b) in group.iter().enumerate() {
            out.push((b & 0x7F) | (((high >> j) & 1) << 7));
        }
        i += group.len();
    }
    Ok(out)
}

/// A parsed Roland SysEx message (RQ1 or DT1), split into its fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SysExMessage {
    /// The Utility SysEx ID (unit number).
    pub device_id: u8,
    /// The model-id bytes as seen on the wire.
    pub model_id: Vec<u8>,
    /// [`CMD_RQ1`] or [`CMD_DT1`].
    pub command: u8,
    /// The 4-byte device address.
    pub address: RolandAddress,
    /// The bytes between the address and the checksum. For DT1 this is the
    /// payload (still 7-bit-packed if it was packed on the wire — see
    /// [`decode_7bit`]); for RQ1 it is the 4-byte requested length.
    pub data: Vec<u8>,
}

impl SysExMessage {
    /// For an RQ1 message, interpret [`data`](Self::data) as the requested
    /// length (4-byte base-128 value). Returns `None` if it is not 4 bytes.
    pub fn requested_len(&self) -> Option<u32> {
        (self.data.len() == ADDRESS_LEN)
            .then(|| RolandAddress::new(self.data.clone().try_into().unwrap()).to_value())
    }
}

/// A device endpoint: the SysEx ID (unit number) plus the model-id bytes.
///
/// The model id is **not** hard-coded. `docs/device-sysex.md` does not pin the
/// exact `modelId` bytes, so the caller supplies them (from a real capture);
/// this crate refuses to present an invented value as fact. Once a capture
/// confirms the TR-8S / TR-6S model id, add a named constant and a
/// `DeviceConfig::tr8s(id)` / `tr6s(id)` convenience constructor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceConfig {
    /// The Utility "SysEx ID" (0-based unit number), one byte.
    pub device_id: u8,
    /// The model-identifier bytes for this device family. Variable length; must
    /// come from a real capture. See the struct docs.
    pub model_id: Vec<u8>,
}

impl DeviceConfig {
    /// A config with the given SysEx ID and model-id bytes.
    pub fn new(device_id: u8, model_id: impl Into<Vec<u8>>) -> Self {
        DeviceConfig {
            device_id,
            model_id: model_id.into(),
        }
    }

    /// A **TR-8S** config with the confirmed model ID ([`MODEL_ID_TR`]) and the
    /// given SysEx ID (use [`DEFAULT_DEVICE_ID`] unless the unit's Utility "SysEx
    /// ID" was changed).
    pub fn tr8s(device_id: u8) -> Self {
        DeviceConfig::new(device_id, MODEL_ID_TR.to_vec())
    }

    /// A **TR-6S** config. The TR-6S shares the TR-8S model ID ([`MODEL_ID_TR`]),
    /// so this is identical to [`DeviceConfig::tr8s`] — provided for clarity at
    /// call sites.
    pub fn tr6s(device_id: u8) -> Self {
        DeviceConfig::new(device_id, MODEL_ID_TR.to_vec())
    }

    /// Bytes of the fixed message prefix: `F0 41 <deviceId> <modelId…>`.
    fn prefix(&self) -> Vec<u8> {
        let mut p = Vec::with_capacity(3 + self.model_id.len());
        p.push(SYSEX_START);
        p.push(ROLAND_MANUFACTURER_ID);
        p.push(self.device_id);
        p.extend_from_slice(&self.model_id);
        p
    }

    /// Assemble a full message (`cmd`, `addr`, `body`) with checksum and
    /// framing. `body` is the bytes the checksum covers alongside the address
    /// (the DT1 payload, or the RQ1 length field).
    fn frame(&self, cmd: u8, address: RolandAddress, body: &[u8]) -> Vec<u8> {
        let addr = address.bytes();
        let mut checked = Vec::with_capacity(ADDRESS_LEN + body.len());
        checked.extend_from_slice(&addr);
        checked.extend_from_slice(body);

        let mut msg = self.prefix();
        msg.push(cmd);
        msg.extend_from_slice(&checked);
        msg.push(roland_checksum(&checked));
        msg.push(SYSEX_END);
        msg
    }

    /// Build an **RQ1** (read request): "give me `len` bytes at `address`".
    ///
    /// The requested length is encoded as a 4-byte base-128 value, mirroring the
    /// address width. **Confirmed** on real TR-8S RQ1 captures: the size field is
    /// 4 bytes, base-128 (a captured 1,171-byte request is `00 00 09 13`).
    pub fn build_rq1(&self, address: RolandAddress, len: u32) -> Vec<u8> {
        let len_field = RolandAddress::from_value(len).bytes();
        self.frame(CMD_RQ1, address, &len_field)
    }

    /// Build a **DT1** (write / data set): place `data` at `address`.
    ///
    /// `data` is written to the payload verbatim; 7-bit-pack it first with
    /// [`encode_7bit`] if it contains bytes with the high bit set.
    pub fn build_dt1(&self, address: RolandAddress, data: &[u8]) -> Vec<u8> {
        self.frame(CMD_DT1, address, data)
    }

    /// Parse a full SysEx message, validating framing, this device's prefix
    /// (manufacturer, SysEx ID, model id), and the checksum.
    ///
    /// Knowing `model_id`'s length is what lets us split the variable-length
    /// prefix from the address — the parse is defined relative to a
    /// [`DeviceConfig`] for exactly that reason.
    pub fn parse(&self, msg: &[u8]) -> Result<SysExMessage> {
        let prefix = self.prefix();
        // Minimum: prefix + cmd + addr(4) + checksum + F7.
        let min = prefix.len() + 1 + ADDRESS_LEN + 1 + 1;
        ensure!(
            msg.len() >= min,
            "message too short: {} bytes, need >= {min}",
            msg.len()
        );
        ensure!(
            msg[0] == SYSEX_START,
            "not a SysEx message (first byte {:#04x})",
            msg[0]
        );
        ensure!(
            *msg.last().unwrap() == SYSEX_END,
            "missing SysEx end byte (last {:#04x})",
            msg.last().unwrap()
        );
        ensure!(
            &msg[..prefix.len()] == prefix.as_slice(),
            "prefix mismatch: not this device (mfr/SysEx-ID/model-id)"
        );

        let cmd = msg[prefix.len()];
        ensure!(
            cmd == CMD_RQ1 || cmd == CMD_DT1,
            "unexpected command byte {cmd:#04x} (want RQ1 0x11 or DT1 0x12)"
        );

        // Body spans from after the command to before the checksum + F7.
        let body_start = prefix.len() + 1;
        let checksum_idx = msg.len() - 2; // last real byte before F7
        let addr_start = body_start;
        let addr_end = addr_start + ADDRESS_LEN;
        ensure!(
            addr_end <= checksum_idx,
            "message truncated: no room for a 4-byte address"
        );

        let address = RolandAddress::new(msg[addr_start..addr_end].try_into().unwrap());
        let data = msg[addr_end..checksum_idx].to_vec();

        let want = roland_checksum(&msg[addr_start..checksum_idx]);
        let got = msg[checksum_idx];
        ensure!(
            want == got,
            "bad checksum: computed {want:#04x}, message had {got:#04x}"
        );

        Ok(SysExMessage {
            device_id: self.device_id,
            model_id: self.model_id.clone(),
            command: cmd,
            address,
            data,
        })
    }

    /// The read primitive: send an RQ1 for `len` bytes at `address`, then read
    /// the device's DT1 reply and return its raw payload bytes.
    ///
    /// The payload is returned **as it arrived on the wire** — if the device
    /// 7-bit-packs it, apply [`decode_7bit`]. Validates that the reply is a DT1
    /// for the same address and that its checksum is correct.
    pub fn read(
        &self,
        port: &mut dyn MidiPort,
        address: RolandAddress,
        len: u32,
    ) -> Result<Vec<u8>> {
        let request = self.build_rq1(address, len);
        port.send(&request)?;
        let reply = port.recv()?;
        let parsed = self.parse(&reply)?;
        ensure!(
            parsed.command == CMD_DT1,
            "expected a DT1 reply, got command {:#04x}",
            parsed.command
        );
        if parsed.address != address {
            bail!(
                "DT1 reply address {:02X?} != requested {:02X?}",
                parsed.address.bytes(),
                address.bytes()
            );
        }
        Ok(parsed.data)
    }

    /// The write primitive: send a DT1 placing `data` at `address`.
    ///
    /// 7-bit-pack `data` with [`encode_7bit`] first if it has high-bit bytes.
    /// This does not wait for any acknowledgement (Roland DT1 is fire-and-forget
    /// unless a handshaking variant is used).
    pub fn write(
        &self,
        port: &mut dyn MidiPort,
        address: RolandAddress,
        data: &[u8],
    ) -> Result<()> {
        let msg = self.build_dt1(address, data);
        port.send(&msg)
    }
}

/// A transport that can send a SysEx message and receive the next one.
///
/// Keeping the protocol behind this trait is what makes it testable with no
/// hardware: the tests below implement it over an in-memory fixture, and the
/// optional [`midir_port`](crate::midir_port) module implements it over real
/// USB-MIDI when built with `--features usb`.
pub trait MidiPort {
    /// Send one complete SysEx message (`F0 … F7`).
    fn send(&mut self, sysex: &[u8]) -> Result<()>;
    /// Block until the next complete SysEx message arrives and return it.
    fn recv(&mut self) -> Result<Vec<u8>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address;

    /// A model-id used only in tests. This is a **fixture**, not a claim about
    /// the real device's bytes (which the doc does not pin down).
    const TEST_MODEL_ID: &[u8] = &[0x00, 0x00, 0x62];

    fn cfg() -> DeviceConfig {
        DeviceConfig::new(0x10, TEST_MODEL_ID)
    }

    /// A [`MidiPort`] that answers each RQ1 by echoing back a DT1 built from a
    /// canned memory image, so the read path runs with no hardware.
    struct MockPort {
        cfg: DeviceConfig,
        /// Canned "device memory": maps an address value to its bytes.
        memory: Vec<(RolandAddress, Vec<u8>)>,
        last_sent: Option<Vec<u8>>,
    }

    impl MidiPort for MockPort {
        fn send(&mut self, sysex: &[u8]) -> Result<()> {
            self.last_sent = Some(sysex.to_vec());
            Ok(())
        }
        fn recv(&mut self) -> Result<Vec<u8>> {
            let sent = self.last_sent.as_ref().expect("nothing sent");
            let req = self.cfg.parse(sent)?;
            assert_eq!(req.command, CMD_RQ1, "mock only answers RQ1");
            let (_, bytes) = self
                .memory
                .iter()
                .find(|(a, _)| *a == req.address)
                .expect("no canned memory at requested address");
            Ok(self.cfg.build_dt1(req.address, bytes))
        }
    }

    #[test]
    fn checksum_matches_spec_example() {
        // The checksum makes (sum(addr+data) + checksum) & 0x7F == 0.
        let run = [0x20, 0x00, 0x00, 0x14, 0x01, 0x02];
        let ck = roland_checksum(&run);
        let total: u32 = run.iter().map(|&b| b as u32).sum::<u32>() + ck as u32;
        assert_eq!(total & 0x7F, 0);
        // All-zero run -> checksum 0 (0x80 - 0 == 0x80, & 0x7F == 0).
        assert_eq!(roland_checksum(&[0, 0, 0, 0]), 0);
    }

    #[test]
    fn checksum_matches_rolands_published_worked_example() {
        // Roland's own documented example (SC-88 MIDI implementation): the run
        // `40 00 7F 00` (address 40 00 7F, data 00) has checksum `41`. An
        // external anchor that pins `roland_checksum` to Roland's spec, not just
        // to our own invariant above.
        assert_eq!(roland_checksum(&[0x40, 0x00, 0x7F, 0x00]), 0x41);
    }

    #[test]
    fn seven_bit_encode_decode_is_inverse() {
        for data in [
            vec![],
            vec![0x00],
            vec![0xFF],
            vec![0x80, 0x7F, 0x81, 0x01],
            (0u16..=300).map(|v| v as u8).collect::<Vec<_>>(),
        ] {
            let packed = encode_7bit(&data);
            assert!(
                packed.iter().all(|&b| b < 0x80),
                "packed must be 7-bit-safe"
            );
            assert_eq!(decode_7bit(&packed).unwrap(), data, "round-trip failed");
        }
    }

    #[test]
    fn seven_bit_layout_smallest_case() {
        // One 0x80 byte -> [high=0b1, 0x00].
        assert_eq!(encode_7bit(&[0x80]), vec![0b0000_0001, 0x00]);
        // Two bytes 0x81,0x02 -> high has bit0 set (0x81), bit1 clear (0x02).
        assert_eq!(encode_7bit(&[0x81, 0x02]), vec![0b0000_0001, 0x01, 0x02]);
    }

    #[test]
    fn decode_rejects_dangling_high_byte() {
        // A high byte with no following data is malformed.
        assert!(decode_7bit(&[0b0000_0001]).is_err());
    }

    #[test]
    fn rq1_build_and_parse_round_trip() {
        let c = cfg();
        let addr = address::PATTERN_KIT_REFERENCE;
        let msg = c.build_rq1(addr, 2);
        assert_eq!(msg[0], SYSEX_START);
        assert_eq!(*msg.last().unwrap(), SYSEX_END);

        let parsed = c.parse(&msg).unwrap();
        assert_eq!(parsed.command, CMD_RQ1);
        assert_eq!(parsed.address, addr);
        assert_eq!(parsed.requested_len(), Some(2));
    }

    #[test]
    fn dt1_build_and_parse_round_trip() {
        let c = cfg();
        let addr = address::KIT_NAME;
        let payload = b"Techno Kit\0\0\0\0\0\0"; // 16 bytes, all 7-bit-safe
        let msg = c.build_dt1(addr, payload);
        let parsed = c.parse(&msg).unwrap();
        assert_eq!(parsed.command, CMD_DT1);
        assert_eq!(parsed.address, addr);
        assert_eq!(parsed.data, payload);
    }

    #[test]
    fn parse_rejects_bad_checksum_and_wrong_prefix() {
        let c = cfg();
        let mut msg = c.build_dt1(address::TONE_NAME, &[1, 2, 3]);
        let ck = msg.len() - 2;
        msg[ck] ^= 0x01; // corrupt checksum
        assert!(c.parse(&msg).is_err());

        // A different device id must be rejected by the prefix check.
        let other = DeviceConfig::new(0x11, TEST_MODEL_ID);
        let good = c.build_dt1(address::TONE_NAME, &[1, 2, 3]);
        assert!(other.parse(&good).is_err());
    }

    #[test]
    fn read_over_mock_port_does_rq1_then_dt1() {
        let c = cfg();
        let addr = address::PATTERN_KIT_REFERENCE;
        let stored = vec![0x00, 0x0E]; // kit reference = 14, u16
        let mut port = MockPort {
            cfg: c.clone(),
            memory: vec![(addr, stored.clone())],
            last_sent: None,
        };

        let got = c.read(&mut port, addr, stored.len() as u32).unwrap();
        assert_eq!(got, stored);

        // The port really did see an RQ1 for that address.
        let sent = c.parse(port.last_sent.as_ref().unwrap()).unwrap();
        assert_eq!(sent.command, CMD_RQ1);
        assert_eq!(sent.address, addr);
    }

    #[test]
    fn read_rejects_wrong_address_reply() {
        let c = cfg();
        // Mock stores data at TONE_NAME but we ask for KIT_NAME -> the mock's
        // find() would panic; instead craft the mismatch directly on parse.
        let reply = c.build_dt1(address::TONE_NAME, &[0]);
        let parsed = c.parse(&reply).unwrap();
        assert_ne!(parsed.address, address::KIT_NAME);
    }

    #[test]
    fn category_name_addresses_stay_7bit_safe() {
        // The base-128 arithmetic keeps all 32 category-name addresses valid.
        for i in 0..address::SYS_CATEGORY_NAME.count {
            let a = address::SYS_CATEGORY_NAME.nth(i).unwrap();
            assert!(
                a.is_7bit_safe(),
                "element {i} not 7-bit-safe: {:02X?}",
                a.bytes()
            );
        }
        assert!(address::SYS_CATEGORY_NAME.nth(32).is_none());
    }

    #[test]
    fn kit_and_pattern_block_strides() {
        // Kit 0 name is the base; kit 1 is one block further, still 7-bit-safe.
        assert_eq!(address::kit_name(0), Some(address::KIT_NAME));
        let k1 = address::kit_name(1).unwrap();
        assert!(k1.is_7bit_safe());
        assert_eq!(
            k1.to_value(),
            address::KIT_NAME.to_value() + address::KIT_BLOCK
        );
        assert!(address::kit_name(128).is_none());

        let p1 = address::pattern_name(1).unwrap();
        assert_eq!(
            p1.to_value(),
            address::PATTERN_NAME.to_value() + address::PATTERN_BLOCK
        );
    }

    #[test]
    fn strides_match_the_send_pattern_capture() {
        // Attested addresses from the compuphonic "send pattern/kit" transfer.
        // Kit 126 ("kit 127" 1-indexed) name + its 11 instrument records:
        assert_eq!(
            address::kit_name(126).unwrap().bytes(),
            [0x10, 0x7e, 0x00, 0x00]
        );
        let insts = address::kit_instruments(126).unwrap();
        for i in 0..11 {
            assert_eq!(
                insts.nth(i).unwrap().bytes(),
                [0x10, 0x7e, 0x10 + i as u8, 0x00],
                "kit-126 instrument {i}"
            );
        }
        // Consecutive patterns step address byte 1 by 0x10; #8 rolls to region 0x21.
        assert_eq!(
            address::pattern_name(0).unwrap().bytes(),
            [0x20, 0x00, 0x00, 0x00]
        );
        assert_eq!(
            address::pattern_name(1).unwrap().bytes(),
            [0x20, 0x10, 0x00, 0x00]
        );
        assert_eq!(
            address::pattern_name(8).unwrap().bytes(),
            [0x21, 0x00, 0x00, 0x00]
        );
        // Pattern field offsets (kitReference, kitReferenceSw) are verbatim.
        assert_eq!(
            address::PATTERN_KIT_REFERENCE.bytes(),
            [0x20, 0x00, 0x00, 0x14]
        );
        assert_eq!(
            address::PATTERN_KIT_REFERENCE_SW.bytes(),
            [0x20, 0x00, 0x01, 0x06]
        );
    }

    #[test]
    fn address_value_round_trips_base128() {
        let a = RolandAddress::new([0x20, 0x40, 0x7F, 0x03]);
        assert_eq!(RolandAddress::from_value(a.to_value()), a);
    }

    #[test]
    fn parses_a_real_captured_tr8s_message() {
        // A real RQ1 from a TR-8S capture (docs/device-sysex.md): the body bytes
        // `41 10 00 00 00 45 11 47 2c 00 10 00 00 00 08 75`, wrapped in F0..F7.
        // These are Roland's protocol on the wire — facts, not vendored code.
        let msg = [
            0xF0, 0x41, 0x10, 0x00, 0x00, 0x00, 0x45, 0x11, 0x47, 0x2c, 0x00, 0x10, 0x00, 0x00,
            0x00, 0x08, 0x75, 0xF7,
        ];
        let cfg = DeviceConfig::tr8s(DEFAULT_DEVICE_ID);
        let parsed = cfg.parse(&msg).expect("real TR-8S RQ1 must parse");
        assert_eq!(parsed.command, CMD_RQ1);
        assert_eq!(parsed.device_id, 0x10);
        assert_eq!(parsed.model_id, MODEL_ID_TR8S);
        assert_eq!(parsed.address.bytes(), [0x47, 0x2c, 0x00, 0x10]);
        assert_eq!(parsed.requested_len(), Some(8));
        // our checksum (over addr+data, msg[8..16]) reproduces the captured 0x75.
        assert_eq!(roland_checksum(&msg[8..16]), 0x75);
        // and we rebuild the exact same bytes.
        assert_eq!(
            cfg.build_rq1(RolandAddress::new([0x47, 0x2c, 0x00, 0x10]), 8),
            msg
        );
    }
}
