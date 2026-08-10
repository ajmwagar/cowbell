//! Typed device reads — browse-level fields over RQ1/DT1.
//!
//! These decode a DT1 reply into a typed value for the fields the device address
//! map ([`crate::address`]) exposes: names, the tone assignments, the pattern↔kit
//! reference, and the user category list. They read the **edit buffer** (the
//! currently-loaded kit/pattern/tone) at the `temp` addresses.
//!
//! ## Scope (and what maps to `tr-format`)
//!
//! The device map covers **summary/browse** fields plus the raw per-voice
//! instrument records, but not yet the full decoded parameter set. The
//! "send pattern/kit" capture located each kit's **11 × 16-byte instrument
//! blocks** at known addresses ([`crate::address::KIT_INSTRUMENT`],
//! surfaced by [`DeviceConfig::read_kit_instruments`]) — but the 14 param bytes
//! after the tone id are still an **undecoded device encoding**; the backup's
//! `VoiceParams` is the decoded reference for the same knobs. Pattern **step
//! words** and **FX** blocks have no confirmed device addresses (that capture
//! transferred only pattern headers — name + kit reference). So a *full*
//! byte-exact [`tr_format::Kit`]/`Pattern`/`Sys` reconstruction over SysEx is
//! **not yet possible**; it needs the instrument-block field decode and a
//! step/FX capture (tracked separately). Where a field does correspond to a
//! `tr-format` concept the types line up (a device category name is the same
//! string as `tr_format::Sys::category_name`, a tone id indexes the same tone
//! table), but the values here are plain strings/ints/bytes on purpose — no
//! lossy struct is fabricated from partial data.

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

/// One instrument slot of a kit, as it lives on the device: a 16-byte record
/// whose first two bytes are the tone id. The remaining bytes are the device's
/// (still undecoded) encoding of the per-voice parameters — kept raw rather than
/// forced into a lossy struct. See [`crate::address::KIT_INSTRUMENT`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInstrument {
    /// `kit.toneId` — indexes the same tone table as `tr_format`.
    pub tone_id: u16,
    /// The full 16-byte record exactly as returned by the device.
    pub raw: [u8; 16],
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
            let bytes = self.read(port, addr, field.size)?;
            ensure!(
                bytes.len() == 16,
                "instrument record {i}: expected 16 bytes, got {}",
                bytes.len()
            );
            let mut raw = [0u8; 16];
            raw.copy_from_slice(&bytes);
            out.push(DeviceInstrument {
                tone_id: decode_base128(&raw[0..2]) as u16,
                raw,
            });
        }
        Ok(Some(out))
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
    fn reads_kit_instruments_decoding_tone_ids() {
        let cfg = DeviceConfig::tr8s(0x10);
        // Answer each instrument read with a 16-byte record whose first two
        // bytes encode tone id = (address byte 2 - 0x10) in base-128.
        let mut port = dev(|addr, _len| {
            let inst = (addr.bytes()[2] - 0x10) as u16;
            let mut v = vec![0u8; 16];
            v[0] = (inst >> 7) as u8;
            v[1] = (inst & 0x7f) as u8;
            v[15] = 0xAB; // a param byte we keep raw
            v
        });
        let insts = cfg.read_kit_instruments(&mut port, 126).unwrap().unwrap();
        assert_eq!(insts.len(), 11);
        assert_eq!(
            insts.iter().map(|d| d.tone_id).collect::<Vec<_>>(),
            (0..11).collect::<Vec<_>>()
        );
        assert!(insts.iter().all(|d| d.raw[15] == 0xAB));
        // Out-of-range kit yields None, not an error.
        assert!(cfg.read_kit_instruments(&mut port, 128).unwrap().is_none());
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
