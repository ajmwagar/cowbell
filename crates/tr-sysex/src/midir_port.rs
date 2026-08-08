//! Real-hardware [`MidiPort`](crate::MidiPort) over `midir` — **feature-gated**.
//!
//! This module only compiles with `--features usb`. The default build and
//! `cargo test` never touch `midir` and never need a device; the protocol is
//! fully exercised from byte-stream fixtures in `lib.rs`.
//!
//! It is a thin adapter: it opens a `midir` output (for RQ1/DT1 sends) and a
//! `midir` input (for the DT1 replies), buffering complete SysEx messages
//! (`F0 … F7`) received on the input into a queue that [`MidirPort::recv`]
//! drains.
//!
//! Only the standard-library and `midir` APIs are used; no third-party protocol
//! code was consulted. The adapter has **not** been exercised against real
//! hardware from this crate — see the crate-level "confirm end to end" note.

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};

use crate::{MidiPort, SYSEX_END, SYSEX_START};

/// A [`MidiPort`] backed by a pair of `midir` connections (out for sends, in
/// for replies).
pub struct MidirPort {
    out: MidiOutputConnection,
    /// Kept alive so the input callback keeps firing; not read directly.
    _in: MidiInputConnection<()>,
    rx: Receiver<Vec<u8>>,
    /// How long [`recv`](MidiPort::recv) waits for a reply before erroring.
    timeout: Duration,
}

impl MidirPort {
    /// Default reply timeout.
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);

    /// Open by matching a substring against the output and input port names
    /// (e.g. `"TR-8S"`). Picks the first port whose name contains `name_hint`.
    pub fn open(name_hint: &str) -> Result<MidirPort> {
        let midi_out = MidiOutput::new("tr-sysex out").context("create MIDI output")?;
        let out_port = midi_out
            .ports()
            .into_iter()
            .find(|p| {
                midi_out
                    .port_name(p)
                    .map(|n| n.contains(name_hint))
                    .unwrap_or(false)
            })
            .ok_or_else(|| anyhow!("no MIDI output port matching {name_hint:?}"))?;
        let out = midi_out
            .connect(&out_port, "tr-sysex-out")
            .map_err(|e| anyhow!("connect MIDI output: {e}"))?;

        let midi_in = MidiInput::new("tr-sysex in").context("create MIDI input")?;
        let in_port = midi_in
            .ports()
            .into_iter()
            .find(|p| {
                midi_in
                    .port_name(p)
                    .map(|n| n.contains(name_hint))
                    .unwrap_or(false)
            })
            .ok_or_else(|| anyhow!("no MIDI input port matching {name_hint:?}"))?;

        let (tx, rx) = std::sync::mpsc::channel();
        // Reassemble complete SysEx messages before forwarding. USB-MIDI may
        // deliver a message in one callback, but buffer defensively.
        let mut buf: Vec<u8> = Vec::new();
        let conn_in = midi_in
            .connect(
                &in_port,
                "tr-sysex-in",
                move |_stamp, bytes, _| {
                    for &b in bytes {
                        if b == SYSEX_START {
                            buf.clear();
                            buf.push(b);
                        } else if !buf.is_empty() {
                            buf.push(b);
                            if b == SYSEX_END {
                                let _ = tx.send(std::mem::take(&mut buf));
                            }
                        }
                    }
                },
                (),
            )
            .map_err(|e| anyhow!("connect MIDI input: {e}"))?;

        Ok(MidirPort {
            out,
            _in: conn_in,
            rx,
            timeout: Self::DEFAULT_TIMEOUT,
        })
    }

    /// Override the reply timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

impl MidiPort for MidirPort {
    fn send(&mut self, sysex: &[u8]) -> Result<()> {
        self.out
            .send(sysex)
            .map_err(|e| anyhow!("MIDI send failed: {e}"))
    }

    fn recv(&mut self) -> Result<Vec<u8>> {
        match self.rx.recv_timeout(self.timeout) {
            Ok(msg) => Ok(msg),
            Err(RecvTimeoutError::Timeout) => {
                bail!("timed out waiting {:?} for a SysEx reply", self.timeout)
            }
            Err(RecvTimeoutError::Disconnected) => bail!("MIDI input disconnected"),
        }
    }
}
