//! Typed device reads — browse-level fields over RQ1/DT1.
//!
//! These decode a DT1 reply into a typed value for the fields the device address
//! map ([`crate::address`]) exposes: names, the tone assignments, the pattern↔kit
//! reference, and the user category list. They read the **edit buffer** (the
//! currently-loaded kit/pattern/tone) at the `temp` addresses.
//!
//! ## Scope (and what maps to `tr-format`)
//!
//! The device parameter map is the **same schema as the backup format**,
//! re-based to the device's SysEx regions (recovered device-free from TR
//! Editor's `Script.xml` + sender code — `cowbell-uqk`). Values ride the wire
//! in the [`crate::wire`] nibble encoding. This module decodes the browse
//! fields plus **typed per-voice instrument records** ([`DeviceInstrument`] via
//! [`DeviceConfig::read_kit_instruments`] / [`DeviceConfig::write_kit_instrument`]
//! — the same knobs as `tr_format::VoiceParams`, read/written live). The kit's
//! effect sub-structs (`kitRev`/`kitDly`/`kitMfx`/`kitExtIn`) and pattern
//! variations (`ptnVar`) are addressable too ([`crate::address::kit_sub`],
//! [`crate::address::pattern_variation`]) — those are the same records
//! `tr-format` decodes, so extending typed reads/writes to them is
//! re-addressing known structs. Field values line up with `tr-format` by
//! design; nothing lossy is fabricated.

use anyhow::{ensure, Result};

use crate::address::{self, RolandAddress};
use crate::{DeviceConfig, MidiPort};

/// A base-128 (7-bits-per-byte, big-endian) value as it travels on the wire —
/// the encoding confirmed for addresses and multi-byte params.
fn decode_base128(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .fold(0u32, |acc, &b| (acc << 7) | (b & 0x7f) as u32)
}

/// The current pattern's browse fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevicePattern {
    /// Pattern name.
    pub name: String,
    /// The kit slot this pattern references (`ptn.kitReference`, a `u16` on the
    /// device).
    pub kit_reference: u16,
}

/// One instrument slot of a kit (`instCommon`), decoded from the device wire
/// form. These are the same per-voice parameters `tr_format::VoiceParams`
/// exposes in the backup — here read live over SysEx. Field widths and the
/// nibble wire encoding are `Script.xml`-attested and confirmed on real capture
/// data (see [`crate::wire`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInstrument {
    /// `INST TONE` (`0..=1023`) — indexes the same tone table as `tr_format`.
    pub tone: u16,
    /// `INST TUNE` (`0..=255`, centre 128).
    pub tune: u8,
    /// `INST DECAY` (`0..=255`).
    pub decay: u8,
    /// `INST LEVEL` (`0..=255`).
    pub level: u8,
    /// `INST GAIN` (`0..=161`).
    pub gain: u8,
    /// `INST PAN` (`0..=255`, centre 128).
    pub pan: u8,
    /// `INST REVERB SEND` (`0..=255`).
    pub reverb_send: u8,
    /// `INST DELAY SEND` (`0..=255`).
    pub delay_send: u8,
}

/// Wire length of an `instCommon` record through `DELAY SEND`: `INST TONE`
/// (`int4x4`, 4) + 7 × `int2x4` (2 each).
pub const INSTRUMENT_WIRE_LEN: u32 = 4 + 7 * 2;

impl DeviceInstrument {
    /// Decode the leading `INST TONE … DELAY SEND` fields of a kit instrument
    /// record from its wire bytes. Returns `None` if `bytes` is too short.
    pub fn from_wire(bytes: &[u8]) -> Option<DeviceInstrument> {
        use crate::wire::decode_nibbles;
        if (bytes.len() as u32) < INSTRUMENT_WIRE_LEN {
            return None;
        }
        Some(DeviceInstrument {
            tone: decode_nibbles(&bytes[0..4]) as u16,
            tune: decode_nibbles(&bytes[4..6]) as u8,
            decay: decode_nibbles(&bytes[6..8]) as u8,
            level: decode_nibbles(&bytes[8..10]) as u8,
            gain: decode_nibbles(&bytes[10..12]) as u8,
            pan: decode_nibbles(&bytes[12..14]) as u8,
            reverb_send: decode_nibbles(&bytes[14..16]) as u8,
            delay_send: decode_nibbles(&bytes[16..18]) as u8,
        })
    }

    /// Encode this instrument to its [`INSTRUMENT_WIRE_LEN`]-byte wire form
    /// (`INST TONE … DELAY SEND`) — the inverse of [`from_wire`](Self::from_wire).
    pub fn to_wire(&self) -> Vec<u8> {
        use crate::wire::encode_nibbles;
        let mut v = Vec::with_capacity(INSTRUMENT_WIRE_LEN as usize);
        v.extend_from_slice(&encode_nibbles(self.tone as u32, 4));
        for field in [
            self.tune,
            self.decay,
            self.level,
            self.gain,
            self.pan,
            self.reverb_send,
            self.delay_send,
        ] {
            v.extend_from_slice(&encode_nibbles(field as u32, 2));
        }
        v
    }
}

/// The current tone's browse fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceTone {
    /// Tone name.
    pub name: String,
    /// `tone.category`.
    pub category: u8,
    /// `tone.type`.
    pub tone_type: u8,
}

impl DeviceConfig {
    /// Read a name field: `len` ASCII bytes at `address`, trailing spaces/NULs
    /// trimmed (matching `tr-format`'s name readers).
    pub fn read_name(
        &self,
        port: &mut dyn MidiPort,
        address: RolandAddress,
        len: u32,
    ) -> Result<String> {
        let bytes = self.read(port, address, len)?;
        Ok(String::from_utf8_lossy(&bytes)
            .trim_end_matches([' ', '\0'])
            .to_string())
    }

    /// Read a single byte at `address`.
    fn read_u8(&self, port: &mut dyn MidiPort, address: RolandAddress) -> Result<u8> {
        let bytes = self.read(port, address, 1)?;
        ensure!(!bytes.is_empty(), "empty DT1 reply for a 1-byte read");
        Ok(bytes[0])
    }

    /// The current (edit-buffer) kit name.
    pub fn read_current_kit_name(&self, port: &mut dyn MidiPort) -> Result<String> {
        self.read_name(port, address::KIT_NAME, 16)
    }

    /// The current pattern's name and kit reference.
    pub fn read_current_pattern(&self, port: &mut dyn MidiPort) -> Result<DevicePattern> {
        let name = self.read_name(port, address::PATTERN_NAME, 16)?;
        let kit = decode_base128(&self.read(port, address::PATTERN_KIT_REFERENCE, 2)?);
        Ok(DevicePattern {
            name,
            kit_reference: kit as u16,
        })
    }

    /// The current tone's name, category, and type.
    pub fn read_current_tone(&self, port: &mut dyn MidiPort) -> Result<DeviceTone> {
        Ok(DeviceTone {
            name: self.read_name(port, address::TONE_NAME, 16)?,
            category: self.read_u8(port, address::TONE_CATEGORY)?,
            tone_type: self.read_u8(port, address::TONE_TYPE)?,
        })
    }

    /// The 11 per-voice instrument records of **persistent kit slot** `kit`
    /// (`0..128`). Each is a 16-byte record; the tone id is decoded, the rest is
    /// returned raw (its field layout is not yet mapped — see the module scope
    /// note). Returns `None` if `kit >= 128`.
    pub fn read_kit_instruments(
        &self,
        port: &mut dyn MidiPort,
        kit: u32,
    ) -> Result<Option<Vec<DeviceInstrument>>> {
        let Some(field) = address::kit_instruments(kit) else {
            return Ok(None);
        };
        let mut out = Vec::with_capacity(field.count as usize);
        for i in 0..field.count {
            let addr = field.nth(i).expect("i < count");
            let bytes = self.read(port, addr, INSTRUMENT_WIRE_LEN)?;
            let voice = DeviceInstrument::from_wire(&bytes).ok_or_else(|| {
                anyhow::anyhow!(
                    "instrument record {i}: expected {INSTRUMENT_WIRE_LEN} wire bytes, got {}",
                    bytes.len()
                )
            })?;
            out.push(voice);
        }
        Ok(Some(out))
    }

    /// Write one instrument slot (`inst < 11`) of kit `kit` (`0..128`) — sends a
    /// DT1 with the instrument's [`INSTRUMENT_WIRE_LEN`]-byte wire form to its
    /// device address. Returns `false` if `kit`/`inst` is out of range.
    /// (Whether the device persists this to flash vs the edit buffer depends on
    /// the addressed slot — see [`crate::address`].)
    pub fn write_kit_instrument(
        &self,
        port: &mut dyn MidiPort,
        kit: u32,
        inst: u32,
        voice: &DeviceInstrument,
    ) -> Result<bool> {
        let Some(addr) = address::kit_instruments(kit).and_then(|f| f.nth(inst)) else {
            return Ok(false);
        };
        self.write(port, addr, &voice.to_wire())?;
        Ok(true)
    }

    /// All user category names (`sys.categoryName`, 32 × 16 bytes).
    pub fn read_category_names(&self, port: &mut dyn MidiPort) -> Result<Vec<String>> {
        let f = address::SYS_CATEGORY_NAME;
        (0..f.count)
            .map(|i| {
                let addr = f.nth(i).expect("i < count");
                self.read_name(port, addr, f.size)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CMD_DT1;

    /// A mock port that answers each RQ1 with a DT1 for the same address,
    /// carrying bytes supplied by a closure `(address, len) -> data`. Exercises
    /// the full RQ1 -> DT1 round-trip from byte fixtures, no hardware.
    struct MockDevice<F: Fn(RolandAddress, u32) -> Vec<u8>> {
        cfg: DeviceConfig,
        answer: F,
        last_reply: Vec<u8>,
    }

    impl<F: Fn(RolandAddress, u32) -> Vec<u8>> MidiPort for MockDevice<F> {
        fn send(&mut self, sysex: &[u8]) -> Result<()> {
            // parse the RQ1 to learn the address + requested length, then stage
            // the matching DT1 reply.
            let rq = self.cfg.parse(sysex)?;
            let len = rq.requested_len().unwrap_or(0);
            let data = (self.answer)(rq.address, len);
            self.last_reply = self.cfg.build_dt1(rq.address, &data);
            Ok(())
        }
        fn recv(&mut self) -> Result<Vec<u8>> {
            Ok(self.last_reply.clone())
        }
    }

    fn dev<F: Fn(RolandAddress, u32) -> Vec<u8>>(answer: F) -> MockDevice<F> {
        MockDevice {
            cfg: DeviceConfig::tr8s(crate::DEFAULT_DEVICE_ID),
            answer,
            last_reply: Vec::new(),
        }
    }

    #[test]
    fn reads_a_name_and_trims_padding() {
        let cfg = DeviceConfig::tr8s(0x10);
        let mut port = dev(|_addr, len| {
            let mut v = b"TR-808_Kit".to_vec();
            v.resize(len as usize, b' '); // space-padded to the field width
            v
        });
        assert_eq!(cfg.read_current_kit_name(&mut port).unwrap(), "TR-808_Kit");
    }

    #[test]
    fn reads_pattern_ref_as_base128_u16() {
        let cfg = DeviceConfig::tr8s(0x10);
        let mut port = dev(|addr, len| {
            if addr == address::PATTERN_KIT_REFERENCE {
                vec![0x01, 0x00] // base-128: (1<<7)|0 = 128
            } else {
                let mut v = b"Beat".to_vec();
                v.resize(len as usize, b' ');
                v
            }
        });
        let p = cfg.read_current_pattern(&mut port).unwrap();
        assert_eq!(p.name, "Beat");
        assert_eq!(p.kit_reference, 128);
    }

    #[test]
    fn reads_tone_meta_and_category_list() {
        let cfg = DeviceConfig::tr8s(0x10);
        let mut port = dev(|addr, len| {
            if addr == address::TONE_CATEGORY {
                vec![3]
            } else if addr == address::TONE_TYPE {
                vec![1]
            } else {
                let mut v = b"909 Bass".to_vec();
                v.resize(len as usize, 0);
                v
            }
        });
        let t = cfg.read_current_tone(&mut port).unwrap();
        assert_eq!(
            t,
            DeviceTone {
                name: "909 Bass".into(),
                category: 3,
                tone_type: 1
            }
        );

        // 32 category names come back, each read at its own strided address.
        let mut port2 = dev(|_addr, len| {
            let mut v = b"USER".to_vec();
            v.resize(len as usize, b' ');
            v
        });
        let names = cfg.read_category_names(&mut port2).unwrap();
        assert_eq!(names.len(), 32);
        assert!(names.iter().all(|n| n == "USER"));
    }

    #[test]
    fn reads_kit_instruments_decoding_the_wire_form() {
        let cfg = DeviceConfig::tr8s(0x10);
        // Reply with the real "KiNK 1" instrument-0 wire bytes for every slot:
        // level 0f 0f = 255, pan 08 00 = 128, gain 05 03 = 83, tone 00 00 0c 01.
        let real0 = [
            0x00, 0x00, 0x0c, 0x01, // INST TONE = 193
            0x07, 0x02, // TUNE = 114
            0x08, 0x01, // DECAY = 129
            0x0f, 0x0f, // LEVEL = 255
            0x05, 0x03, // GAIN = 83
            0x08, 0x00, // PAN = 128
            0x00, 0x00, // REVERB SEND = 0
            0x02, 0x0e, // DELAY SEND = 46
        ];
        let mut port = dev(move |_addr, len| real0[..len as usize].to_vec());
        let insts = cfg.read_kit_instruments(&mut port, 126).unwrap().unwrap();
        assert_eq!(insts.len(), 11);
        let v = &insts[0];
        assert_eq!(v.tone, 193);
        assert_eq!(
            (v.tune, v.decay, v.level, v.gain, v.pan),
            (114, 129, 255, 83, 128)
        );
        assert_eq!((v.reverb_send, v.delay_send), (0, 46));
        // Out-of-range kit yields None, not an error.
        assert!(cfg.read_kit_instruments(&mut port, 128).unwrap().is_none());
    }

    #[test]
    fn instrument_wire_round_trips_and_writes_a_dt1() {
        let cfg = DeviceConfig::tr8s(0x10);
        let v = DeviceInstrument {
            tone: 512,
            tune: 200,
            decay: 64,
            level: 255,
            gain: 81,
            pan: 128,
            reverb_send: 30,
            delay_send: 46,
        };
        // to_wire -> from_wire is lossless, and 7-bit-safe.
        let wire = v.to_wire();
        assert_eq!(wire.len(), INSTRUMENT_WIRE_LEN as usize);
        assert!(wire.iter().all(|&b| b <= 0x0f));
        assert_eq!(DeviceInstrument::from_wire(&wire).unwrap(), v);

        // write_kit_instrument sends a DT1 our parser accepts, at kit 5 inst 2's
        // address, carrying the encoded voice.
        let mut port = dev(|_a, _l| vec![]);
        assert!(cfg.write_kit_instrument(&mut port, 5, 2, &v).unwrap());
        let sent = cfg.parse(&port.last_reply).unwrap();
        assert_eq!(sent.command, CMD_DT1);
        assert_eq!(
            sent.address,
            address::kit_instruments(5).unwrap().nth(2).unwrap()
        );
        assert!(!cfg.write_kit_instrument(&mut port, 5, 99, &v).unwrap()); // inst OOR
    }

    #[test]
    fn round_trip_uses_a_real_dt1_shape() {
        // Sanity: the mock's staged reply really is a DT1 our parser accepts.
        let cfg = DeviceConfig::tr8s(0x10);
        let mut port = dev(|_a, l| vec![0u8; l as usize]);
        let _ = cfg.read_current_kit_name(&mut port).unwrap();
        assert_eq!(cfg.parse(&port.last_reply).unwrap().command, CMD_DT1);
    }
}
