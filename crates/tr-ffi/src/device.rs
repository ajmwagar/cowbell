//! The device plane: RQ1/DT1 over a transport the *app* owns.
//!
//! The split here is deliberate and is the whole point of the module. Rust owns
//! the **protocol** — message framing, the Roland checksum, base-128 addresses,
//! which address holds which field — and Swift owns the **wire**, because
//! CoreMIDI is a platform API with a platform lifecycle (hot-plug, background
//! and suspend, USB-C adapter chains) that has no business being reimplemented
//! in Rust.
//!
//! [`MidiTransport`] is the seam. It is a UniFFI *foreign* trait: Swift
//! implements it over SwiftMIDIIO, and [`Device`] drives `tr_sysex`'s protocol
//! through it. Because `tr_sysex::MidiPort` is already an abstraction over the
//! same two operations, the adapter below is thin — and, importantly, the
//! crate's no-hardware testability survives the crossing: a transport backed by
//! canned bytes works exactly as well as one backed by a real box, on either
//! side of the boundary.
//!
//! Everything here is the plaintext user-data plane. Nothing in `tr-sysex`
//! relates to encrypted firmware or its key.

use std::sync::{Arc, Mutex};

use tr_sysex::{
    address::{self, RolandAddress},
    DeviceConfig, MidiPort, DEFAULT_DEVICE_ID,
};

use crate::TrError;

// ---------------------------------------------------------------------------
// The seam
// ---------------------------------------------------------------------------

/// A SysEx transport, implemented by the app.
///
/// `receive` is expected to **block** until a complete `F0 … F7` message arrives
/// or `timeout_ms` elapses, and to return the message with its framing bytes
/// intact — `tr_sysex` validates them. Implementations must be safe to call
/// from a background thread; every [`Device`] method is synchronous and blocking
/// by design, so callers should not invoke them from a UI thread.
#[uniffi::export(with_foreign)]
pub trait MidiTransport: Send + Sync {
    /// Send one complete SysEx message.
    fn send(&self, sysex: Vec<u8>) -> Result<(), TrError>;

    /// Block for the next complete SysEx message, or fail on timeout.
    fn receive(&self, timeout_ms: u32) -> Result<Vec<u8>, TrError>;
}

/// Adapts the foreign transport to `tr_sysex`'s port trait.
///
/// `MidiPort` takes `&mut self` (it predates any thought of a foreign
/// implementation); the foreign trait is `&self` and `Send + Sync`. The `&mut`
/// is therefore satisfied by the caller holding the adapter exclusively, which
/// [`Device`]'s mutex guarantees.
struct ForeignPort {
    transport: Arc<dyn MidiTransport>,
    timeout_ms: u32,
}

impl MidiPort for ForeignPort {
    fn send(&mut self, sysex: &[u8]) -> anyhow::Result<()> {
        self.transport
            .send(sysex.to_vec())
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    fn recv(&mut self) -> anyhow::Result<Vec<u8>> {
        self.transport
            .receive(self.timeout_ms)
            .map_err(|e| anyhow::anyhow!("{e}"))
    }
}

// ---------------------------------------------------------------------------
// Value types
// ---------------------------------------------------------------------------

/// Which box we are talking to. Both share the SysEx model id `00 00 00 45`;
/// this only picks the constructor and labels the connection for the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TrModel {
    Tr6s,
    Tr8s,
}

/// The current pattern's browse fields, read from the edit buffer.
#[derive(Debug, Clone, uniffi::Record)]
pub struct DevicePatternInfo {
    pub name: String,
    /// The kit slot this pattern references, as stored: 1-based.
    pub kit_reference: u16,
}

/// The current tone's browse fields.
#[derive(Debug, Clone, uniffi::Record)]
pub struct DeviceToneInfo {
    pub name: String,
    pub category: u8,
    pub tone_type: u8,
}

/// One instrument slot of a kit as it lives on the device.
#[derive(Debug, Clone, uniffi::Record)]
pub struct DeviceInstrumentInfo {
    /// Indexes the same tone table as the backup format.
    pub tone_id: u16,
    /// The full 16-byte record exactly as the device returned it. The 14 bytes
    /// after the tone id are the device's **undecoded** parameter encoding —
    /// they are handed over raw rather than forced into a lossy struct.
    pub raw: Vec<u8>,
}

// ---------------------------------------------------------------------------
// The handle
// ---------------------------------------------------------------------------

/// A connected TR, reachable over a foreign [`MidiTransport`].
///
/// Every method is a blocking round trip: an RQ1 goes out, a DT1 comes back.
/// The mutex serialises them, which is not just for memory safety — the
/// protocol has no request tags, so a reply can only be matched to a request by
/// being the next thing that arrives. Two overlapping reads would be able to
/// take each other's answers.
#[derive(uniffi::Object)]
pub struct Device {
    cfg: DeviceConfig,
    port: Mutex<ForeignPort>,
    model: TrModel,
}

impl Device {
    /// Run `f` against the protocol config and the exclusive port.
    fn with_port<T>(
        &self,
        f: impl FnOnce(&DeviceConfig, &mut ForeignPort) -> anyhow::Result<T>,
    ) -> Result<T, TrError> {
        let mut port = self.port.lock().map_err(|_| TrError::Device {
            message: "the MIDI transport lock was poisoned by an earlier panic".into(),
        })?;
        f(&self.cfg, &mut port).map_err(|e| TrError::Device {
            message: e.to_string(),
        })
    }
}

#[uniffi::export]
impl Device {
    /// Connect over `transport`.
    ///
    /// `device_id` is the unit's Utility "SysEx ID"; leave it at the default
    /// unless it was changed on the box. `timeout_ms` bounds every single reply
    /// — a device that is plugged in but not listening should fail a read in
    /// about a second, not hang the caller forever.
    #[uniffi::constructor]
    pub fn new(
        transport: Arc<dyn MidiTransport>,
        model: TrModel,
        device_id: u8,
        timeout_ms: u32,
    ) -> Arc<Self> {
        let cfg = match model {
            TrModel::Tr6s => DeviceConfig::tr6s(device_id),
            TrModel::Tr8s => DeviceConfig::tr8s(device_id),
        };
        Arc::new(Device {
            cfg,
            port: Mutex::new(ForeignPort {
                transport,
                timeout_ms,
            }),
            model,
        })
    }

    /// A `Device` with the usual defaults: SysEx ID `0x10`, one-second timeout.
    #[uniffi::constructor]
    pub fn with_defaults(transport: Arc<dyn MidiTransport>, model: TrModel) -> Arc<Self> {
        Self::new(transport, model, DEFAULT_DEVICE_ID, 1000)
    }

    pub fn model(&self) -> TrModel {
        self.model
    }

    /// Cheapest possible proof of life: read the current kit's name.
    ///
    /// If this returns a string, the whole chain worked — the port is the right
    /// one, the SysEx ID matches, the device answered an RQ1 with a DT1 at the
    /// requested address, and the checksum verified.
    pub fn ping(&self) -> Result<String, TrError> {
        self.current_kit_name()
    }

    // -- edit buffer --------------------------------------------------------

    pub fn current_kit_name(&self) -> Result<String, TrError> {
        self.with_port(|cfg, port| cfg.read_current_kit_name(port))
    }

    pub fn current_pattern(&self) -> Result<DevicePatternInfo, TrError> {
        self.with_port(|cfg, port| {
            let p = cfg.read_current_pattern(port)?;
            Ok(DevicePatternInfo {
                name: p.name,
                kit_reference: p.kit_reference,
            })
        })
    }

    pub fn current_tone(&self) -> Result<DeviceToneInfo, TrError> {
        self.with_port(|cfg, port| {
            let t = cfg.read_current_tone(port)?;
            Ok(DeviceToneInfo {
                name: t.name,
                category: t.category,
                tone_type: t.tone_type,
            })
        })
    }

    // -- persistent slots ---------------------------------------------------
    //
    // These take the same 0-based indices as the file side of the FFI, and are
    // range-checked here rather than deep in the address map so the error names
    // what was asked for.

    /// Name of persistent kit slot `index` (0-based).
    pub fn kit_name(&self, index: u32) -> Result<String, TrError> {
        let addr = address::kit_name(index).ok_or_else(|| TrError::OutOfRange {
            what: "kit".into(),
            index,
            count: address::KIT_COUNT,
        })?;
        self.with_port(|cfg, port| cfg.read_name(port, addr, 16))
    }

    /// Name of persistent pattern slot `index` (0-based).
    pub fn pattern_name(&self, index: u32) -> Result<String, TrError> {
        let addr = address::pattern_name(index).ok_or_else(|| TrError::OutOfRange {
            what: "pattern".into(),
            index,
            count: address::PATTERN_COUNT,
        })?;
        self.with_port(|cfg, port| cfg.read_name(port, addr, 16))
    }

    /// The 11 per-voice instrument records of persistent kit slot `index`.
    pub fn kit_instruments(&self, index: u32) -> Result<Vec<DeviceInstrumentInfo>, TrError> {
        let found = self.with_port(|cfg, port| cfg.read_kit_instruments(port, index))?;
        let records = found.ok_or_else(|| TrError::OutOfRange {
            what: "kit".into(),
            index,
            count: address::KIT_COUNT,
        })?;
        Ok(records
            .into_iter()
            .map(|i| DeviceInstrumentInfo {
                tone_id: i.tone_id,
                raw: i.raw.to_vec(),
            })
            .collect())
    }

    /// All 32 user category names.
    pub fn category_names(&self) -> Result<Vec<String>, TrError> {
        self.with_port(|cfg, port| cfg.read_category_names(port))
    }

    // -- escape hatch -------------------------------------------------------

    /// Read `len` raw bytes at a 4-byte device address.
    ///
    /// The address map is still being filled in, and this is what lets the app
    /// (or a debug console) reach an address the typed accessors do not cover
    /// yet, without a new FFI method per discovery. The payload comes back
    /// exactly as it arrived on the wire.
    pub fn read_raw(&self, address: Vec<u8>, len: u32) -> Result<Vec<u8>, TrError> {
        let bytes: [u8; 4] = address.as_slice().try_into().map_err(|_| TrError::Write {
            message: format!("a device address is 4 bytes, got {}", address.len()),
        })?;
        let addr = RolandAddress::new(bytes);
        self.with_port(|cfg, port| cfg.read(port, addr, len))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
//
// These prove the *adapter* — that a foreign transport drives the real protocol
// correctly — by implementing `MidiTransport` in Rust over canned device memory.
// The Swift side has an equivalent test using the same idea; between them, the
// round trip is covered on both sides of the boundary with no hardware.

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use tr_sysex::{CMD_RQ1, MODEL_ID_TR};

    /// A transport that answers each RQ1 with a DT1 built from canned memory,
    /// the same trick `tr-sysex`'s own tests use — but reached through the
    /// foreign-trait seam rather than `MidiPort` directly.
    struct FakeTransport {
        cfg: DeviceConfig,
        memory: Vec<(RolandAddress, Vec<u8>)>,
        last_sent: StdMutex<Option<Vec<u8>>>,
        /// Every request seen, so tests can assert on what went out.
        sent_count: StdMutex<u32>,
    }

    impl FakeTransport {
        fn new(memory: Vec<(RolandAddress, Vec<u8>)>) -> Arc<Self> {
            Arc::new(FakeTransport {
                cfg: DeviceConfig::tr8s(DEFAULT_DEVICE_ID),
                memory,
                last_sent: StdMutex::new(None),
                sent_count: StdMutex::new(0),
            })
        }
    }

    impl MidiTransport for FakeTransport {
        fn send(&self, sysex: Vec<u8>) -> Result<(), TrError> {
            *self.last_sent.lock().unwrap() = Some(sysex);
            *self.sent_count.lock().unwrap() += 1;
            Ok(())
        }

        fn receive(&self, _timeout_ms: u32) -> Result<Vec<u8>, TrError> {
            let sent = self.last_sent.lock().unwrap().clone().ok_or(TrError::Device {
                message: "receive before send".into(),
            })?;
            let req = self.cfg.parse(&sent).map_err(|e| TrError::Device {
                message: e.to_string(),
            })?;
            assert_eq!(req.command, CMD_RQ1, "fake only answers RQ1");

            let (_, bytes) = self
                .memory
                .iter()
                .find(|(a, _)| *a == req.address)
                .ok_or_else(|| TrError::Device {
                    message: format!("no canned memory at {:02X?}", req.address.bytes()),
                })?;
            Ok(self.cfg.build_dt1(req.address, bytes))
        }
    }

    /// 16 name bytes, space-padded the way the device stores them.
    fn name_bytes(s: &str) -> Vec<u8> {
        let mut v = s.as_bytes().to_vec();
        v.resize(16, b' ');
        v
    }

    #[test]
    fn reads_the_current_kit_name_through_a_foreign_transport() {
        let fake = FakeTransport::new(vec![(address::KIT_NAME, name_bytes("909 BASE"))]);
        let device = Device::with_defaults(fake.clone(), TrModel::Tr8s);

        assert_eq!(device.current_kit_name().unwrap(), "909 BASE");
        // Trailing pad must be trimmed, not returned.
        assert_eq!(*fake.sent_count.lock().unwrap(), 1, "one RQ1 per read");
    }

    #[test]
    fn ping_proves_the_whole_chain() {
        let fake = FakeTransport::new(vec![(address::KIT_NAME, name_bytes("MECHANOID"))]);
        let device = Device::with_defaults(fake, TrModel::Tr6s);
        assert_eq!(device.ping().unwrap(), "MECHANOID");
    }

    #[test]
    fn the_request_on_the_wire_is_a_well_formed_rq1() {
        let fake = FakeTransport::new(vec![(address::KIT_NAME, name_bytes("X"))]);
        let device = Device::with_defaults(fake.clone(), TrModel::Tr8s);
        device.current_kit_name().unwrap();

        let sent = fake.last_sent.lock().unwrap().clone().unwrap();
        assert_eq!(sent[0], 0xF0);
        assert_eq!(sent[1], 0x41, "Roland manufacturer id");
        assert_eq!(sent[2], DEFAULT_DEVICE_ID);
        assert_eq!(&sent[3..7], &MODEL_ID_TR);
        assert_eq!(sent[7], CMD_RQ1);
        assert_eq!(*sent.last().unwrap(), 0xF7);
    }

    #[test]
    fn persistent_slot_indices_are_zero_based_and_range_checked() {
        // Slot 0 must address the same place the edit-buffer kit name does —
        // the address map's per-slot offset is what this is checking.
        let slot0 = address::kit_name(0).unwrap();
        let fake = FakeTransport::new(vec![(slot0, name_bytes("SLOT ZERO"))]);
        let device = Device::with_defaults(fake, TrModel::Tr8s);

        assert_eq!(device.kit_name(0).unwrap(), "SLOT ZERO");

        match device.kit_name(address::KIT_COUNT) {
            Err(TrError::OutOfRange { what, count, .. }) => {
                assert_eq!(what, "kit");
                assert_eq!(count, address::KIT_COUNT);
            }
            other => panic!("expected OutOfRange, got {other:?}"),
        }
    }

    #[test]
    fn a_transport_failure_surfaces_as_a_device_error() {
        // Empty memory: the fake cannot answer, so the read must fail cleanly
        // rather than hang or panic across the boundary.
        let fake = FakeTransport::new(vec![]);
        let device = Device::with_defaults(fake, TrModel::Tr8s);

        match device.current_kit_name() {
            Err(TrError::Device { message }) => {
                assert!(message.contains("no canned memory"), "got {message}");
            }
            other => panic!("expected Device error, got {other:?}"),
        }
    }

    #[test]
    fn read_raw_rejects_a_malformed_address() {
        let fake = FakeTransport::new(vec![]);
        let device = Device::with_defaults(fake, TrModel::Tr8s);
        assert!(device.read_raw(vec![0x10, 0x00], 4).is_err());
    }
}
