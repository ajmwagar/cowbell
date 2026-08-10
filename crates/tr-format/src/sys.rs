//! The `SYS ` section — global system parameters.
//!
//! **Status: confirmed.** Every field below is named from TR Editor's schema
//! (`Script.xml`, `structType` `sysGeneral` / `sysCategory` / `sysSound` /
//! `sysMidi`, reached from the model path `fm.SYS.*`) and validated against two
//! independent corpora:
//!
//! 1. a v1.51 TR-6S SD-card backup, and
//! 2. the factory-default backup image carried (LZSS-compressed) inside the
//!    v2.00 firmware's `dd001_init_param.bin`.
//!
//! The `SYS ` chunk — its 16-byte header **and** all 752 payload bytes — is
//! **byte-identical** between the two. That fixes the layout as stable from
//! v1.51 to v2.00 and shows the reference backup's system settings were never
//! changed from factory; it is *not* two independent value samples, so the
//! field map rests on the schema plus the fact that 68 of the 88 ranged fields
//! land exactly on their schema default and **none** falls out of range.
//! See `docs/tr-format.md` for the evidence.
//!
//! ## Framing
//!
//! `SYS ` is an array section with the same shape as `PTN `/`KIT `, holding
//! exactly **one** record:
//!
//! ```text
//! payload+0x00  u32 count       = 1
//! payload+0x04  u32 record_size = 0x2F0 (752)   <- equals the whole payload
//! payload+0x08  8 B  section token (see docs; not decoded)
//! payload+0x10  736 B body: sysGeneral | sysCategory | sysSound | sysMidi
//! ```
//!
//! i.e. the "array preamble" *is* record 0's 16-byte header, exactly as for
//! patterns and kits — `SYS ` is not a special case.
//!
//! ## Body map
//!
//! Sub-structs are laid out back-to-back in schema order, each sized by
//! accumulating [`crate::schema_value_size`] over its `<value>` list:
//!
//! | Body range | Sub-struct | Size |
//! | ---------- | ---------- | ---- |
//! | `0x000`–`0x05B` | `sysGeneral` | 92 |
//! | `0x05C`–`0x25B` | `sysCategory` (32 × 16-char names) | 512 |
//! | `0x25C`–`0x2A7` | `sysSound` | 76 |
//! | `0x2A8`–`0x2CD` | `sysMidi` (named fields) | 38 |
//! | `0x2CE`–`0x2DF` | `sysMidi` reserve tail (zero) | 18 |

use crate::{Backup, RolandBlock, Section};

/// Bytes of one `SYS ` record — and of the whole `SYS ` payload, since there is
/// exactly one record. Declared by the record header itself (`payload+0x04`).
pub const SYS_RECORD_SIZE: usize = 0x2F0;
/// The 16-byte record header before the parameter body (same framing as
/// `PTN `/`KIT `): `u32 count`, `u32 record_size`, then an 8-byte section token.
pub const SYS_RECORD_HEADER_LEN: usize = 0x10;
/// Bytes of parameter body after the record header.
pub const SYS_BODY_LEN: usize = SYS_RECORD_SIZE - SYS_RECORD_HEADER_LEN;

/// `sysGeneral` base within the record body.
pub const SYS_GENERAL_OFFSET: usize = 0x000;
/// `sysGeneral` size (accumulated from the schema; 42 named fields + reserves).
pub const SYS_GENERAL_LEN: usize = 92;
/// `sysCategory` base within the record body — 32 user category names.
pub const SYS_CATEGORY_OFFSET: usize = 0x05C;
/// `sysSound` base within the record body.
pub const SYS_SOUND_OFFSET: usize = 0x25C;
/// `sysSound` size (accumulated from the schema).
pub const SYS_SOUND_LEN: usize = 76;
/// `sysMidi` base within the record body.
pub const SYS_MIDI_OFFSET: usize = 0x2A8;
/// Bytes of `sysMidi` that carry named (non-reserve) fields. The schema's
/// reserve tail is longer than the record provides — see [`Sys::reserve_tail`].
pub const SYS_MIDI_NAMED_LEN: usize = 38;

/// Number of user category-name slots (`sysCategory.CATEG_NAMEnnA`).
pub const SYS_CATEGORY_COUNT: usize = 32;
/// Bytes per category name.
pub const SYS_CATEGORY_NAME_LEN: usize = 16;

/// Number of MIDI note-assignment slots (`sysMidi.Inst NoteNN`, 0–22).
pub const SYS_INST_NOTES: usize = 23;
/// The value an `Inst Note` slot carries when no note is assigned. It is the
/// schema default and the top of the field's `0,128` range; on a TR-6S the five
/// TR-8S-only voices read exactly this.
pub const INST_NOTE_OFF: u8 = 128;

/// Instruments a TR-8S addresses (and a TR-6S leaves partly empty) — the length
/// of [`crate::INST_TRACKS`], and the stride of the `sysMidi` note map.
pub const SYS_INST_COUNT: usize = 11;
/// Number of assignable slider colours (`sysGeneral.Slider Color *`), one per
/// instrument — same order as [`crate::INST_TRACKS`].
pub const SYS_SLIDER_COLORS: usize = SYS_INST_COUNT;
/// Number of individual-output assignments (`sysSound.Assign 1`…`Assign 6`).
pub const SYS_ASSIGNS: usize = 6;

/// `sysGeneral` — display, sequencer and global-preference parameters.
///
/// Field order, ranges and defaults are TR Editor's; the offsets are the
/// accumulated schema offsets, confirmed by the observed bytes: all 41 named
/// fields (42 bytes) are in range, 40 of them sit on their schema default, and
/// `Tempo` reads its exact default `1250` as a little-endian `u16` across the
/// two bytes the `int4x4` rule predicts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SysGeneral {
    /// `LCD Contrast`, 0–9 (default 4).
    pub lcd_contrast: u8,
    /// `LED Bright`, 0–9 (default 7).
    pub led_bright: u8,
    /// `LED Off Bright`, 0–9 (default 2).
    pub led_off_bright: u8,
    /// `SliderLED`, 0–1.
    pub slider_led: u8,
    /// `SliderColorSource`, 0–1.
    pub slider_color_source: u8,
    /// `Auto Off`, 0–2.
    pub auto_off: u8,
    /// `Knob Mode`, 0–1.
    pub knob_mode: u8,
    /// `WeakBeat`, 0–1.
    pub weak_beat: u8,
    /// `LED Demo`, 0–10 (default 5).
    pub led_demo: u8,
    /// `Auto Save`, 0–1.
    pub auto_save: u8,
    /// `TempoSrc`, 0–1.
    pub tempo_src: u8,
    /// `TempoSync`, 0–3.
    pub tempo_sync: u8,
    /// `Tempo`, stored as **BPM × 10** (400–3000, default 1250 = 125.0 BPM) —
    /// the same encoding as the per-pattern tempo. See [`SysGeneral::tempo_bpm`].
    pub tempo: u16,
    /// `Sync Out`, 0–1 (default 1).
    pub sync_out: u8,
    /// `Shuffle`, 0–1.
    pub shuffle: u8,
    /// `SEQ Mode`, 0–1.
    pub seq_mode: u8,
    /// `ManualMode`, 0–2 (default 1).
    pub manual_mode: u8,
    /// `KitSelect`, 0–1 (default 1).
    pub kit_select: u8,
    /// `M.Trig`, 0–1 (default 1).
    pub m_trig: u8,
    /// `USB Mode`, 0–1.
    pub usb_mode: u8,
    /// `USB Audio`, 0–1.
    pub usb_audio: u8,
    /// `SCAT TRIG`, 0–2.
    pub scat_trig: u8,
    /// `HH Link`, 0–1.
    pub hh_link: u8,
    /// `Start Ptn`, 0–128 (default 1).
    pub start_ptn: u8,
    /// `Start Kit`, 0–128 (default 1).
    pub start_kit: u8,
    /// `Last Ptn`, 0–127 — the pattern slot the unit was left on.
    pub last_ptn: u8,
    /// `Last Kit`, 0–127.
    pub last_kit: u8,
    /// `Ptn Lock`, 0–1.
    pub ptn_lock: u8,
    /// `Slider Color BD`…`Slider Color RC`, 0–11 each, in [`crate::INST_TRACKS`]
    /// order. Factory value is `0,1,2,…,10` — the identity ramp, which is what
    /// pins this array's offset unambiguously.
    pub slider_color: [u8; SYS_SLIDER_COLORS],
    /// `Inst Pad`, 0–3 (default 1).
    pub inst_pad: u8,
    /// `Trig Adjust`, 0–12.
    pub trig_adjust: u8,
}

impl SysGeneral {
    /// Global tempo in BPM (the stored `u16` is BPM × 10).
    pub fn tempo_bpm(&self) -> f32 {
        f32::from(self.tempo) / 10.0
    }
}

impl RolandBlock for SysGeneral {
    /// Decoded span `+0x00..=0x29` (42 bytes).
    const LEN: usize = 0x2a;

    fn from_block(b: &[u8]) -> SysGeneral {
        SysGeneral {
            lcd_contrast: b[0x00],
            led_bright: b[0x01],
            led_off_bright: b[0x02],
            slider_led: b[0x03],
            slider_color_source: b[0x04],
            auto_off: b[0x05],
            knob_mode: b[0x06],
            weak_beat: b[0x07],
            led_demo: b[0x08],
            auto_save: b[0x09],
            tempo_src: b[0x0a],
            tempo_sync: b[0x0b],
            tempo: u16::from_le_bytes([b[0x0c], b[0x0d]]),
            sync_out: b[0x0e],
            shuffle: b[0x0f],
            seq_mode: b[0x10],
            manual_mode: b[0x11],
            kit_select: b[0x12],
            m_trig: b[0x13],
            usb_mode: b[0x14],
            usb_audio: b[0x15],
            scat_trig: b[0x16],
            hh_link: b[0x17],
            start_ptn: b[0x18],
            start_kit: b[0x19],
            last_ptn: b[0x1a],
            last_kit: b[0x1b],
            ptn_lock: b[0x1c],
            slider_color: std::array::from_fn(|i| b[0x1d + i]),
            inst_pad: b[0x28],
            trig_adjust: b[0x29],
        }
    }

    fn write_to(&self, b: &mut [u8]) {
        b[0x00] = self.lcd_contrast;
        b[0x01] = self.led_bright;
        b[0x02] = self.led_off_bright;
        b[0x03] = self.slider_led;
        b[0x04] = self.slider_color_source;
        b[0x05] = self.auto_off;
        b[0x06] = self.knob_mode;
        b[0x07] = self.weak_beat;
        b[0x08] = self.led_demo;
        b[0x09] = self.auto_save;
        b[0x0a] = self.tempo_src;
        b[0x0b] = self.tempo_sync;
        b[0x0c..0x0e].copy_from_slice(&self.tempo.to_le_bytes());
        b[0x0e] = self.sync_out;
        b[0x0f] = self.shuffle;
        b[0x10] = self.seq_mode;
        b[0x11] = self.manual_mode;
        b[0x12] = self.kit_select;
        b[0x13] = self.m_trig;
        b[0x14] = self.usb_mode;
        b[0x15] = self.usb_audio;
        b[0x16] = self.scat_trig;
        b[0x17] = self.hh_link;
        b[0x18] = self.start_ptn;
        b[0x19] = self.start_kit;
        b[0x1a] = self.last_ptn;
        b[0x1b] = self.last_kit;
        b[0x1c] = self.ptn_lock;
        b[0x1d..0x1d + SYS_SLIDER_COLORS].copy_from_slice(&self.slider_color);
        b[0x28] = self.inst_pad;
        b[0x29] = self.trig_adjust;
    }
}

/// `sysSound` — output routing and external-input mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SysSound {
    /// `Local Sw`, 0–2 (default 1).
    pub local_sw: u8,
    /// `Mix Out`, 0–1.
    pub mix_out: u8,
    /// `Assign 1`…`Assign 6`, 0–2 each — the individual-output assignments.
    pub assign: [u8; SYS_ASSIGNS],
    /// `ExtInMode`, 0–1.
    pub ext_in_mode: u8,
}

impl RolandBlock for SysSound {
    /// Decoded span `+0x00..=0x08` (9 bytes).
    const LEN: usize = 9;

    fn from_block(b: &[u8]) -> SysSound {
        SysSound {
            local_sw: b[0x00],
            mix_out: b[0x01],
            assign: std::array::from_fn(|i| b[0x02 + i]),
            ext_in_mode: b[0x08],
        }
    }

    fn write_to(&self, b: &mut [u8]) {
        b[0x00] = self.local_sw;
        b[0x01] = self.mix_out;
        b[0x02..0x02 + SYS_ASSIGNS].copy_from_slice(&self.assign);
        b[0x08] = self.ext_in_mode;
    }
}

/// `sysMidi` — MIDI channels, per-instrument note assignments, and the TX/RX
/// filter switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SysMidi {
    /// `Device ID`, 0–15.
    pub device_id: u8,
    /// `Omni Mode`, 0–1.
    pub omni_mode: u8,
    /// `Pattern Ch`, 0–15 (default 9 — MIDI channel 10, the GM drum channel).
    pub pattern_ch: u8,
    /// `Kit Ch`, 0–15.
    pub kit_ch: u8,
    /// `Inst Note00`…`Inst Note22`, 0–128, [`INST_NOTE_OFF`] when unassigned.
    /// See [`SysMidi::inst_note`] for the (inferred) main/alternate split.
    pub inst_note: [u8; SYS_INST_NOTES],
    /// `USB MIDI Thru`, 0–1 (default 1).
    pub usb_midi_thru: u8,
    /// `Soft Thru`, 0–1 (default 1).
    pub soft_thru: u8,
    /// `TX Prog Chg`, 0–1.
    pub tx_prog_chg: u8,
    /// `TX Bank Sel`, 0–1.
    pub tx_bank_sel: u8,
    /// `TX Edit Data`, 0–1.
    pub tx_edit_data: u8,
    /// `TX Nudge`, 0–1.
    pub tx_nudge: u8,
    /// `TX Shuffle`, 0–1.
    pub tx_shuffle: u8,
    /// `RX Prog Chg`, 0–1.
    pub rx_prog_chg: u8,
    /// `RX Bank Sel`, 0–1.
    pub rx_bank_sel: u8,
    /// `RX Edit Data`, 0–1.
    pub rx_edit_data: u8,
    /// `RX FA FC`, 0–1 (default 1) — accept MIDI Start/Stop.
    pub rx_fa_fc: u8,
}

impl RolandBlock for SysMidi {
    /// Decoded span `+0x00..=0x25` (38 bytes); the reserve tail after `+0x25` is
    /// preserved by `write_to`.
    const LEN: usize = 0x26;

    fn from_block(b: &[u8]) -> SysMidi {
        SysMidi {
            device_id: b[0x00],
            omni_mode: b[0x01],
            pattern_ch: b[0x02],
            kit_ch: b[0x03],
            inst_note: std::array::from_fn(|i| b[0x04 + i]),
            usb_midi_thru: b[0x1b],
            soft_thru: b[0x1c],
            tx_prog_chg: b[0x1d],
            tx_bank_sel: b[0x1e],
            tx_edit_data: b[0x1f],
            tx_nudge: b[0x20],
            tx_shuffle: b[0x21],
            rx_prog_chg: b[0x22],
            rx_bank_sel: b[0x23],
            rx_edit_data: b[0x24],
            rx_fa_fc: b[0x25],
        }
    }

    fn write_to(&self, b: &mut [u8]) {
        b[0x00] = self.device_id;
        b[0x01] = self.omni_mode;
        b[0x02] = self.pattern_ch;
        b[0x03] = self.kit_ch;
        b[0x04..0x04 + SYS_INST_NOTES].copy_from_slice(&self.inst_note);
        b[0x1b] = self.usb_midi_thru;
        b[0x1c] = self.soft_thru;
        b[0x1d] = self.tx_prog_chg;
        b[0x1e] = self.tx_bank_sel;
        b[0x1f] = self.tx_edit_data;
        b[0x20] = self.tx_nudge;
        b[0x21] = self.tx_shuffle;
        b[0x22] = self.rx_prog_chg;
        b[0x23] = self.rx_bank_sel;
        b[0x24] = self.rx_edit_data;
        b[0x25] = self.rx_fa_fc;
    }
}

impl SysMidi {
    /// The note assigned to instrument `inst` (0–10, [`crate::INST_TRACKS`]
    /// order), or `None` if the slot is [`INST_NOTE_OFF`] / out of range.
    ///
    /// **Inferred, not confirmed.** The schema names the 23 slots only
    /// `Inst Note00`…`Inst Note22`. Slots 0–10 lining up with the 11
    /// instruments is forced by the count and corroborated by the observed
    /// TR-6S values (`36 38 43 39 42 46` for BD/SD/LT/HC/CH/OH — the GM drum
    /// notes for those voices — with slots 6–10, the TR-8S-only voices,
    /// reading [`INST_NOTE_OFF`]).
    pub fn inst_note(&self, inst: usize) -> Option<u8> {
        let n = *self.inst_note.get(inst)?;
        (n != INST_NOTE_OFF).then_some(n)
    }

    /// The **alternate**-tone note for instrument `inst` (slots 11–21), or
    /// `None` if unassigned / out of range.
    ///
    /// **Inferred, not confirmed.** Same reasoning as [`SysMidi::inst_note`],
    /// plus: on the reference data slots 11–16 read `35 40 41 54 44 55`, which
    /// are the GM "second" drum sounds for exactly the six TR-6S voices, and
    /// slot 22 is unaccounted for. A controlled hardware diff (change one
    /// instrument's ALT note, save, diff) would settle it.
    pub fn inst_note_alt(&self, inst: usize) -> Option<u8> {
        if inst >= SYS_INST_COUNT {
            return None;
        }
        let n = *self.inst_note.get(SYS_INST_COUNT + inst)?;
        (n != INST_NOTE_OFF).then_some(n)
    }
}

/// The single `SYS ` record. A lightweight view: call the accessors with the
/// owning [`Backup`]'s `raw()`, in the style of [`crate::Kit`] / [`crate::Pattern`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sys {
    /// Byte offset of the record (= the `SYS ` payload offset) within the file.
    pub offset: usize,
}

impl Sys {
    /// Offset of the parameter body (`record + 0x10`) within the file.
    pub fn body_offset(&self) -> usize {
        self.offset + SYS_RECORD_HEADER_LEN
    }

    /// The parameter body ([`SYS_BODY_LEN`] bytes).
    pub fn body<'a>(&self, raw: &'a [u8]) -> &'a [u8] {
        let s = self.body_offset();
        &raw[s..s + SYS_BODY_LEN]
    }

    /// The 8-byte section token at record `+0x08`. Not decoded — see
    /// `docs/tr-format.md`; it is stable across the two corpora because their
    /// `SYS ` payloads are byte-identical.
    pub fn token(&self, raw: &[u8]) -> [u8; 8] {
        let s = self.offset + 8;
        raw[s..s + 8].try_into().unwrap()
    }

    /// The `sysGeneral` parameters.
    pub fn general(&self, raw: &[u8]) -> SysGeneral {
        SysGeneral::from_block(&self.body(raw)[SYS_GENERAL_OFFSET..])
    }

    /// The `sysSound` parameters.
    pub fn sound(&self, raw: &[u8]) -> SysSound {
        SysSound::from_block(&self.body(raw)[SYS_SOUND_OFFSET..])
    }

    /// The `sysMidi` parameters.
    pub fn midi(&self, raw: &[u8]) -> SysMidi {
        SysMidi::from_block(&self.body(raw)[SYS_MIDI_OFFSET..])
    }

    /// User category name `i` (0–31), trailing spaces/NULs trimmed, or `None`
    /// if `i` is out of range. Factory content is `USER01`…`USER32`.
    pub fn category_name(&self, raw: &[u8], i: usize) -> Option<String> {
        if i >= SYS_CATEGORY_COUNT {
            return None;
        }
        let s = self.body_offset() + SYS_CATEGORY_OFFSET + i * SYS_CATEGORY_NAME_LEN;
        Some(
            String::from_utf8_lossy(&raw[s..s + SYS_CATEGORY_NAME_LEN])
                .trim_end_matches([' ', '\0'])
                .to_string(),
        )
    }

    /// All 32 user category names.
    pub fn category_names(&self, raw: &[u8]) -> Vec<String> {
        (0..SYS_CATEGORY_COUNT)
            .filter_map(|i| self.category_name(raw, i))
            .collect()
    }

    /// Write the `sysGeneral` block back (inverse of [`Sys::general`]). Touches
    /// only its decoded bytes; the rest of the body is preserved.
    pub fn set_general(&self, raw: &mut [u8], g: &SysGeneral) {
        let s = self.body_offset() + SYS_GENERAL_OFFSET;
        g.write_to(&mut raw[s..]);
    }

    /// Write the `sysSound` block back (inverse of [`Sys::sound`]).
    pub fn set_sound(&self, raw: &mut [u8], s: &SysSound) {
        let o = self.body_offset() + SYS_SOUND_OFFSET;
        s.write_to(&mut raw[o..]);
    }

    /// Write the `sysMidi` block back (inverse of [`Sys::midi`]).
    pub fn set_midi(&self, raw: &mut [u8], m: &SysMidi) {
        let o = self.body_offset() + SYS_MIDI_OFFSET;
        m.write_to(&mut raw[o..]);
    }

    /// Set user category name `i` (0–31), space-padded (inverse of
    /// [`Sys::category_name`]). Returns false if `i` is out of range.
    pub fn set_category_name(&self, raw: &mut [u8], i: usize, name: &str) -> bool {
        if i >= SYS_CATEGORY_COUNT {
            return false;
        }
        let s = self.body_offset() + SYS_CATEGORY_OFFSET + i * SYS_CATEGORY_NAME_LEN;
        crate::write_name_field(raw, s, SYS_CATEGORY_NAME_LEN, name)
    }

    /// The undecoded tail of the body: the head of `sysMidi`'s `RESERVE*` run
    /// (18 bytes, all zero in both corpora). The schema declares a longer
    /// reserve tail than the 752-byte record provides, so the record stops
    /// here; whether the firmware simply truncates the struct is **unknown**.
    pub fn reserve_tail<'a>(&self, raw: &'a [u8]) -> &'a [u8] {
        &self.body(raw)[SYS_MIDI_OFFSET + SYS_MIDI_NAMED_LEN..]
    }
}

impl Backup {
    /// The `SYS ` record, or `None` if the backup has no `SYS ` section (or it
    /// is shorter than one record).
    pub fn sys(&self) -> Option<Sys> {
        let sec: &Section = self.find("SYS")?;
        if sec.payload_len < SYS_RECORD_SIZE
            || sec.payload_offset + SYS_RECORD_SIZE > self.raw().len()
        {
            return None;
        }
        Some(Sys {
            offset: sec.payload_offset,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HEADER_LEN, INST_TRACKS};

    /// Build a synthetic backup with one `SYS ` chunk whose body carries
    /// distinctive values at each schema-derived offset. No Roland bytes.
    fn synthetic_sys() -> Vec<u8> {
        let mut body = vec![0u8; SYS_BODY_LEN];

        // sysGeneral
        let g = SYS_GENERAL_OFFSET;
        body[g] = 9; // LCD Contrast
        body[g + 0x01] = 3; // LED Bright
        body[g + 0x08] = 5; // LED Demo
        body[g + 0x0c..g + 0x0e].copy_from_slice(&1234u16.to_le_bytes()); // Tempo
        body[g + 0x0e] = 1; // Sync Out
        body[g + 0x15] = 1; // USB Audio
        body[g + 0x18] = 7; // Start Ptn
        body[g + 0x19] = 8; // Start Kit
        body[g + 0x1a] = 42; // Last Ptn
        body[g + 0x1b] = 43; // Last Kit
        for i in 0..SYS_SLIDER_COLORS {
            body[g + 0x1d + i] = i as u8; // Slider Color ramp
        }
        body[g + 0x28] = 2; // Inst Pad
        body[g + 0x29] = 12; // Trig Adjust

        // sysCategory: 32 names
        for i in 0..SYS_CATEGORY_COUNT {
            let o = SYS_CATEGORY_OFFSET + i * SYS_CATEGORY_NAME_LEN;
            let name = format!("CAT{:02}", i + 1);
            let nb = name.as_bytes();
            body[o..o + nb.len()].copy_from_slice(nb);
            for b in &mut body[o + nb.len()..o + SYS_CATEGORY_NAME_LEN] {
                *b = b' ';
            }
        }

        // sysSound
        let s = SYS_SOUND_OFFSET;
        body[s] = 2; // Local Sw
        body[s + 1] = 1; // Mix Out
        for i in 0..SYS_ASSIGNS {
            body[s + 2 + i] = (i % 3) as u8;
        }
        body[s + 8] = 1; // ExtInMode

        // sysMidi
        let m = SYS_MIDI_OFFSET;
        body[m] = 15; // Device ID
        body[m + 1] = 1; // Omni Mode
        body[m + 2] = 9; // Pattern Ch
        body[m + 3] = 3; // Kit Ch
                         // main notes for insts 0..5, the rest off
        for (i, n) in [36u8, 38, 43, 39, 42, 46].iter().enumerate() {
            body[m + 4 + i] = *n;
        }
        for i in 6..SYS_INST_COUNT {
            body[m + 4 + i] = INST_NOTE_OFF;
        }
        // alternate notes for insts 0..5, the rest off
        for (i, n) in [35u8, 40, 41, 54, 44, 55].iter().enumerate() {
            body[m + 4 + SYS_INST_COUNT + i] = *n;
        }
        for i in 6..SYS_INST_NOTES - SYS_INST_COUNT {
            body[m + 4 + SYS_INST_COUNT + i] = INST_NOTE_OFF;
        }
        body[m + 0x1b] = 1; // USB MIDI Thru
        body[m + 0x1c] = 1; // Soft Thru
        body[m + 0x21] = 1; // TX Shuffle
        body[m + 0x25] = 1; // RX FA FC

        let mut v = Vec::new();
        v.extend_from_slice(b"TR6S");
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&5u32.to_le_bytes());
        v.resize(HEADER_LEN, 0);
        v.extend_from_slice(b"SYS ");
        v.extend_from_slice(&0u32.to_le_bytes()); // reserved
        v.extend_from_slice(&(SYS_RECORD_SIZE as u32).to_le_bytes()); // payload size
        v.extend_from_slice(&0u32.to_le_bytes()); // extra
                                                  // record header: count, record_size, 8-byte token
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&(SYS_RECORD_SIZE as u32).to_le_bytes());
        v.extend_from_slice(&[0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF]);
        v.extend_from_slice(&body);
        v
    }

    #[test]
    fn framing_is_one_record_filling_the_payload() {
        // The sub-struct map accounts for the whole body, with the schema's
        // over-long sysMidi reserve tail truncated at the record end.
        assert_eq!(SYS_BODY_LEN, 736);
        assert_eq!(SYS_GENERAL_OFFSET + SYS_GENERAL_LEN, SYS_CATEGORY_OFFSET);
        assert_eq!(
            SYS_CATEGORY_OFFSET + SYS_CATEGORY_COUNT * SYS_CATEGORY_NAME_LEN,
            SYS_SOUND_OFFSET
        );
        assert_eq!(SYS_SOUND_OFFSET + SYS_SOUND_LEN, SYS_MIDI_OFFSET);
        // The named region ends 18 bytes short of the record end; the rest is
        // the (truncated) sysMidi reserve tail.
        assert_eq!(SYS_BODY_LEN - (SYS_MIDI_OFFSET + SYS_MIDI_NAMED_LEN), 18);

        let bytes = synthetic_sys();
        let b = Backup::parse(bytes.clone()).unwrap();
        let sec = b.find("SYS").unwrap();
        assert_eq!(sec.payload_len, SYS_RECORD_SIZE);
        // The "array preamble" is record 0's own header: count then record_size.
        let p = sec.payload_offset;
        assert_eq!(u32::from_le_bytes(b.raw()[p..p + 4].try_into().unwrap()), 1);
        assert_eq!(
            u32::from_le_bytes(b.raw()[p + 4..p + 8].try_into().unwrap()) as usize,
            SYS_RECORD_SIZE
        );

        let sys = b.sys().unwrap();
        assert_eq!(sys.offset, p);
        assert_eq!(sys.body_offset(), p + SYS_RECORD_HEADER_LEN);
        assert_eq!(sys.body(b.raw()).len(), SYS_BODY_LEN);
        assert_eq!(sys.token(b.raw())[0], 0x01);
        assert_eq!(sys.reserve_tail(b.raw()).len(), 18);
        assert!(sys.reserve_tail(b.raw()).iter().all(|&x| x == 0));
        // still lossless
        assert_eq!(b.to_bytes(), bytes);
    }

    #[test]
    fn general_fields_land_at_schema_offsets() {
        let b = Backup::parse(synthetic_sys()).unwrap();
        let g = b.sys().unwrap().general(b.raw());
        assert_eq!(g.lcd_contrast, 9);
        assert_eq!(g.led_bright, 3);
        assert_eq!(g.led_demo, 5);
        assert_eq!(g.tempo, 1234);
        assert_eq!(g.tempo_bpm(), 123.4);
        assert_eq!(g.sync_out, 1);
        assert_eq!(g.usb_audio, 1);
        assert_eq!(g.start_ptn, 7);
        assert_eq!(g.start_kit, 8);
        assert_eq!(g.last_ptn, 42);
        assert_eq!(g.last_kit, 43);
        assert_eq!(g.inst_pad, 2);
        assert_eq!(g.trig_adjust, 12);
        // The slider-colour ramp: one entry per TR-8S instrument, in panel order.
        assert_eq!(g.slider_color.len(), INST_TRACKS.len());
        assert_eq!(g.slider_color, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        // Untouched fields read zero — no offset drift into a neighbour.
        assert_eq!(g.knob_mode, 0);
        assert_eq!(g.ptn_lock, 0);
    }

    #[test]
    fn category_names_are_32_slots_of_16_bytes() {
        let b = Backup::parse(synthetic_sys()).unwrap();
        let sys = b.sys().unwrap();
        let names = sys.category_names(b.raw());
        assert_eq!(names.len(), SYS_CATEGORY_COUNT);
        assert_eq!(names[0], "CAT01");
        assert_eq!(names[31], "CAT32");
        assert_eq!(sys.category_name(b.raw(), 15).as_deref(), Some("CAT16"));
        assert_eq!(sys.category_name(b.raw(), SYS_CATEGORY_COUNT), None);
    }

    #[test]
    fn sound_fields_land_at_schema_offsets() {
        let b = Backup::parse(synthetic_sys()).unwrap();
        let s = b.sys().unwrap().sound(b.raw());
        assert_eq!(s.local_sw, 2);
        assert_eq!(s.mix_out, 1);
        assert_eq!(s.assign, [0, 1, 2, 0, 1, 2]);
        assert_eq!(s.ext_in_mode, 1);
    }

    #[test]
    fn midi_fields_and_note_map() {
        let b = Backup::parse(synthetic_sys()).unwrap();
        let m = b.sys().unwrap().midi(b.raw());
        assert_eq!(m.device_id, 15);
        assert_eq!(m.omni_mode, 1);
        assert_eq!(m.pattern_ch, 9);
        assert_eq!(m.kit_ch, 3);
        assert_eq!(m.usb_midi_thru, 1);
        assert_eq!(m.soft_thru, 1);
        assert_eq!(m.tx_shuffle, 1);
        assert_eq!(m.rx_fa_fc, 1);
        // The TX/RX switches between the confirmed ones stay zero.
        assert_eq!(m.tx_prog_chg, 0);
        assert_eq!(m.rx_edit_data, 0);

        // 23 note slots; the (inferred) main/alt split.
        assert_eq!(m.inst_note.len(), SYS_INST_NOTES);
        assert_eq!(m.inst_note(0), Some(36)); // BD
        assert_eq!(m.inst_note(5), Some(46)); // OH on a TR-6S
        assert_eq!(m.inst_note(6), None); // TR-8S-only voice: 128 = off
        assert_eq!(m.inst_note(SYS_INST_NOTES), None);
        assert_eq!(m.inst_note_alt(0), Some(35));
        assert_eq!(m.inst_note_alt(3), Some(54));
        assert_eq!(m.inst_note_alt(6), None);
        assert_eq!(m.inst_note_alt(SYS_SLIDER_COLORS), None);
    }

    #[test]
    fn sys_is_none_without_a_full_record() {
        // A SYS chunk too short to hold the record must not be decoded.
        let mut v = Vec::new();
        v.extend_from_slice(b"TR6S");
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&5u32.to_le_bytes());
        v.resize(HEADER_LEN, 0);
        v.extend_from_slice(b"SYS ");
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&4u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&[0, 0, 0, 0]);
        let b = Backup::parse(v).unwrap();
        assert!(b.sys().is_none());
        // ...and a backup with no SYS section at all.
        let b2 = Backup::parse({
            let mut v = Vec::new();
            v.extend_from_slice(b"TR6S");
            v.extend_from_slice(&0u32.to_le_bytes());
            v.extend_from_slice(&5u32.to_le_bytes());
            v.resize(HEADER_LEN, 0);
            v
        })
        .unwrap();
        assert!(b2.sys().is_none());
    }

    #[test]
    fn sys_setters_round_trip_and_stay_lossless() {
        let mut b = Backup::parse(synthetic_sys()).unwrap();
        let sys = b.sys().unwrap();
        let orig = b.to_bytes();

        // general: edit a few fields, write back, re-read.
        let mut g = sys.general(b.raw());
        g.tempo = 1400;
        g.lcd_contrast = 7;
        g.slider_color[3] = 9;
        sys.set_general(b.raw_mut(), &g);
        assert_eq!(sys.general(b.raw()), g);

        // sound + midi write-back round-trips.
        let mut snd = sys.sound(b.raw());
        snd.assign[2] = 2;
        sys.set_sound(b.raw_mut(), &snd);
        assert_eq!(sys.sound(b.raw()), snd);

        let mut m = sys.midi(b.raw());
        m.pattern_ch = 5;
        m.inst_note[1] = 60;
        sys.set_midi(b.raw_mut(), &m);
        assert_eq!(sys.midi(b.raw()), m);

        // category name.
        assert!(sys.set_category_name(b.raw_mut(), 4, "Perc"));
        assert_eq!(sys.category_name(b.raw(), 4).unwrap(), "Perc");
        assert!(!sys.set_category_name(b.raw_mut(), 99, "x"));

        // Losslessness: every changed byte lies inside the SYS body; the record
        // header, the 8-byte token, and anything past the body are untouched.
        let after = b.to_bytes();
        let body = sys.body_offset()..sys.body_offset() + SYS_BODY_LEN;
        for i in 0..orig.len() {
            if orig[i] != after[i] {
                assert!(
                    body.contains(&i),
                    "byte 0x{i:x} changed outside the SYS body"
                );
            }
        }
        // the +0x08 token is preserved.
        assert_eq!(sys.token(&orig), sys.token(&after));
    }
}
