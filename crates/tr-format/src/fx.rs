//! Effects (reverb / delay / master FX / per-instrument insert FX).
//!
//! ## There is no `FX  ` chunk
//!
//! Earlier notes recorded an `FX  ` container chunk seen "×4". That was a false
//! positive: the four `FX  ` byte runs in the reference backup are all
//! *un-aligned* and sit inside `TONE` entry names — `Ring FX`, `Flute FX`,
//! `Tube FX`, `Voice FX`. Walking the chunk chain from `0x40` (each chunk is
//! `16 + payload`, then 16 bytes of zero padding to the next header) accounts
//! for the whole file with `SYS`, `PTN`, `KIT`, `TONE`, `PCMT`, `SMPL` — and no
//! FX chunk. **All effects state lives inside the `KIT ` record.**
//!
//! ## Where the FX live in a kit record
//!
//! `Script.xml`'s `kit` structType lists the sub-structs in record order.
//! Accumulating their field sizes with the crate's [`schema_value_size`] rule
//! reproduces every boundary below, given two facts learned here:
//!
//! - in the kit record `int4x4` with range `0..=65535` is **2 bytes** (as in
//!   `ptnVar00`, not the 3 that `ptnCmn` uses), and
//! - **each sub-struct starts at a 4-byte-aligned record offset** — the two
//!   sub-structs whose packed size is not a multiple of 4 (`kitCmn` 67 → 68,
//!   `kitMfxShare` 25 → 28) are padded, every other one already is.
//!
//! With those, the chain lands exactly on the independently-confirmed
//! `instCommon[0]` offset `+0x194`:
//!
//! | Record | Struct | Size | Contents |
//! | ------ | ------ | ---- | -------- |
//! | `+0x10` | `kitCmn` | 68 | name, level, mute groups, slider colours |
//! | `+0x54` | `kitRev` | 40 | **reverb** — [`ReverbParams`] |
//! | `+0x7C` | `kitDly` | 52 | **delay** — [`DelayParams`] |
//! | `+0xB0` | `kitMfxCommon` | 20 | **master FX** type + on/off |
//! | `+0xC4` | `kitMfxShare` | 28 | master FX `Ctrl` + `PRM00..23` |
//! | `+0xE0` | `kitExtIn` | 40 | external-input gain/pan/**sends** |
//! | `+0x108` | `kitLfo` | 36 | kit LFO |
//! | `+0x12C` | `kitCtrl` | 80 | CTRL assignments |
//! | `+0x17C` | `kitOut` | 12 | per-instrument output routing |
//! | `+0x188` | `kitRef` | 12 | kit references |
//! | `+0x194` | `instCommon[0]` | 11×52 | the voice blocks (`crate::VoiceParams`) |
//! | `+0x3D0` | `instFxCommon[0]` | 11×32 | **per-instrument insert FX** |
//!
//! ## The shared `PRM` pool
//!
//! Master FX and insert FX both store their parameters in a generic 24-byte
//! `PRM00..PRM23` pool preceded by a `Ctrl` byte. What those bytes *mean*
//! depends on the type field. `Script.xml`'s `alt` structType maps type index →
//! overlay struct (`kitMfxComp`, `kitMfxDrv`, … / `instFxComp`, `instFxDrv`, …)
//! in the same order as TR Editor's `mfxType` / `instFxType` name tables, and
//! each overlay's first value is its own `Ctrl`, so **overlay value `n+1` is
//! `PRMn`**. TR Editor's own EFX panels corroborate this directly: the LPF /
//! HPF / LPF-HPF panel binds `PRM00`→Depth, `PRM01`→Resonance, `PRM02`→filter
//! Type, `PRM03`→Gain (±40 dB), `PRM04`→Clipper — exactly `kitMfxFlt` minus its
//! `Ctrl`. [`MFX_TYPES`] and [`INST_FX_TYPES`] hold those tables.
//!
//! ## Evidence
//!
//! Everything above is checked against the 128 kit records of the v1.51 TR-6S
//! reference backup:
//!
//! - **Reverb / delay**: every schema default is the modal byte at the mapped
//!   offset (`REVERB TIME` 150 in 114/128, `PRE DELAY` 20 in 123, `LOW CUT` 2 in
//!   119, `HIGH CUT` 11 in 126, `DENSITY` 10 in 127; `DELAY FEEDBACK` 120 in 101,
//!   `HIGH CUT` 7 in 118, `HIGH DAMP F` 13 in 125, `TAP TIME` 50 in 124, `ECHO
//!   MODE` 1 in 126, `ECHO BASS`/`TREBLE` 15 and `TAPE DIST` 4 in all 128), and
//!   no byte leaves its schema range — several touch the maximum exactly
//!   (`DELAY HIGH CUT` 14 = max, `DELAY LOW DAMP` 81 = max, `ECHO MODE` 6 = max).
//! - **Master FX / insert FX**: decoding each kit's `PRM` pool through the
//!   overlay selected by its type field yields **0 out-of-range values** in 779
//!   master-FX values (12 distinct types exercised) and 6,157 insert-FX values
//!   (all 17 types exercised). Shifting the base offset by ±1/±2/±4 or using a
//!   wrong insert-FX stride (28/30/31/33/34/36) puts 2–27 % of values out of
//!   range, so the test discriminates.
//! - **Semantics**: the decode reads as music. `Lofi HipHop` → `ROOM` reverb at
//!   level 99 with a `TAPE ECHO` delay at level 189; `TR-808_Kit` → `HALL1` at
//!   level 0, plain `DLY`, `COMP+DRV` on the drums and `HPF` on the hats;
//!   `TR-626_Kit` → `L/H BOOST` on every voice.
//!
//! ## Known gaps
//!
//! - `kitDly.PITCH COARSE` / `PITCH FINE` read `0` in all 128 kits, below their
//!   `201..=237` / `1..=201` ranges. Explained rather than anomalous: they belong
//!   to `DELAY TYPE 3` (`PITCH SHFT`), and no factory kit uses that type (only
//!   `DLY`/`PAN`/`TAPE ECHO` occur). Left as raw bytes.
//! - Master-FX types 19 (`FATTENER`) and 20 (`VINYL SIM`) have **no overlay
//!   struct** in this `Script.xml`. Their `PRM` names below come from TR
//!   Editor's `mfxCtrl` CTRL-target table and are **inferred**, with unknown
//!   ranges — [`FxTypeInfo::params_confirmed`] is `false` for them.
//! - The 11th insert-FX block starts at `+0x510` but the kit record is only
//!   `0x520` long, so its last 16 bytes (`PRM11..PRM23` and the pad) **do not
//!   fit**. See [`InstFxParams::prm_available`]; the shortfall is real, not a
//!   mis-derivation (the 11 type columns are each followed by exactly 3 zero
//!   bytes and every one is inside `instFxCommon.Type`'s `0..=16` range). Why
//!   Roland's record is 16 bytes short of its own layout is **unresolved**.
//!
//! [`schema_value_size`]: crate::schema_value_size

use crate::{Kit, KIT_RECORD_SIZE};

// --- Record offsets (all confirmed; see the module docs) ----------------------

/// `kitRev` — the reverb block.
pub const KIT_REVERB_OFFSET: usize = 0x54;
/// `kitDly` — the delay block.
pub const KIT_DELAY_OFFSET: usize = 0x7c;
/// `kitMfxCommon` — master-FX `Type` then `Sw`.
pub const KIT_MFX_OFFSET: usize = 0xb0;
/// `kitMfxShare` — master-FX `Ctrl` then `PRM00..PRM23`.
pub const KIT_MFX_SHARE_OFFSET: usize = 0xc4;
/// `kitExtIn` — external-input gain/pan and FX sends.
pub const KIT_EXT_IN_OFFSET: usize = 0xe0;
/// `instFxCommon[0]` — the first per-instrument insert-FX block.
pub const INST_FX_OFFSET: usize = 0x3d0;
/// Bytes between consecutive insert-FX blocks (`instFxCommon` 4 + `instFxShare`
/// 25 padded to 28).
pub const INST_FX_STRIDE: usize = 0x20;
/// Insert-FX blocks in a kit record — the TR-8S instrument count. Note the last
/// one does not fit; see [`InstFxParams::prm_available`].
pub const INST_FX_SLOTS: usize = 11;
/// `PRM` bytes in a master-FX or insert-FX parameter pool.
pub const FX_PRM_COUNT: usize = 24;

/// `kitRev.REVERB TYPE` names (TR Editor string table `reverbType`).
pub const REVERB_TYPES: [&str; 7] = ["AMBI", "ROOM", "HALL1", "HALL2", "PLATE", "MOD", "HA-DOU"];
/// `kitDly.DELAY TYPE` names (TR Editor string table `delayType`).
pub const DELAY_TYPES: [&str; 4] = ["DLY", "PAN", "TAPE ECHO", "PITCH SHFT"];
/// `kitDly.ECHO MODE` names (TR Editor string table `tapeEchoMode`), used when
/// `DELAY TYPE` is `TAPE ECHO`.
pub const ECHO_MODES: [&str; 7] = ["S", "M", "L", "S+M", "S+L", "M+L", "S+M+L"];

/// Name of a reverb type, or `None` if out of range.
pub fn reverb_type_name(t: u8) -> Option<&'static str> {
    REVERB_TYPES.get(t as usize).copied()
}

/// Name of a delay type, or `None` if out of range.
pub fn delay_type_name(t: u8) -> Option<&'static str> {
    DELAY_TYPES.get(t as usize).copied()
}

/// Name of a tape-echo mode, or `None` if out of range.
pub fn echo_mode_name(t: u8) -> Option<&'static str> {
    ECHO_MODES.get(t as usize).copied()
}

// --- Reverb -------------------------------------------------------------------

/// `kitRev` — the kit's reverb send effect. Seven bytes at
/// [`KIT_REVERB_OFFSET`], then 33 bytes of reserve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReverbParams {
    /// `REVERB TYPE` (0–6) — index into [`REVERB_TYPES`]. Default 2 (`HALL1`).
    pub reverb_type: u8,
    /// `REVERB TIME` (0–255). Default 150.
    pub time: u8,
    /// `REVERB LEVEL` (0–255). Default 0 — i.e. reverb off by default.
    pub level: u8,
    /// `REVERB PRE DELAY` (0–100). Default 20.
    pub pre_delay: u8,
    /// `REVERB LOW CUT` (0–17). Default 2.
    pub low_cut: u8,
    /// `REVERB HIGH CUT` (0–14). Default 11.
    pub high_cut: u8,
    /// `REVERB DENSITY` (0–10). Default 10.
    pub density: u8,
}

impl ReverbParams {
    /// Parse from a `kitRev` block (at least 7 bytes).
    pub fn from_block(b: &[u8]) -> ReverbParams {
        ReverbParams {
            reverb_type: b[0],
            time: b[1],
            level: b[2],
            pre_delay: b[3],
            low_cut: b[4],
            high_cut: b[5],
            density: b[6],
        }
    }

    /// The reverb type's display name.
    pub fn type_name(&self) -> Option<&'static str> {
        reverb_type_name(self.reverb_type)
    }

    /// Write back — inverse of [`ReverbParams::from_block`] (7 bytes).
    pub fn write_to(&self, b: &mut [u8]) {
        b[0] = self.reverb_type;
        b[1] = self.time;
        b[2] = self.level;
        b[3] = self.pre_delay;
        b[4] = self.low_cut;
        b[5] = self.high_cut;
        b[6] = self.density;
    }
}

// --- Delay --------------------------------------------------------------------

/// `kitDly` — the kit's delay send effect. 23 bytes at [`KIT_DELAY_OFFSET`],
/// then 29 bytes of reserve.
///
/// The `echo_*` fields apply to `DELAY TYPE 2` (`TAPE ECHO`) and the `pitch_*`
/// fields to `DELAY TYPE 3` (`PITCH SHFT`). No factory kit selects `PITCH SHFT`,
/// so its two bytes read `0` — below their documented ranges. They are exposed
/// raw rather than interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DelayParams {
    /// `DELAY TYPE` (0–3) — index into [`DELAY_TYPES`].
    pub delay_type: u8,
    /// `DELAY TEMPO SYNC` (0/1).
    pub tempo_sync: u8,
    /// `DELAY LEVEL` (0–255). Default 0 — i.e. delay off by default.
    pub level: u8,
    /// `DELAY TIME` (0–255). Default 104.
    pub time: u8,
    /// `DELAY FEEDBACK` (0–255). Default 120.
    pub feedback: u8,
    /// `DELAY HIGH CUT` (0–14). Default 7.
    pub high_cut: u8,
    /// `DELAY HIGH DAMP` (0–81).
    pub high_damp: u8,
    /// `DELAY HIGH DAMP F` (0–13). Default 13.
    pub high_damp_freq: u8,
    /// `DELAY LOW DAMP` (0–81).
    pub low_damp: u8,
    /// `DELAY LOW DAMP F` (0–10).
    pub low_damp_freq: u8,
    /// `DELAY TAP TIME` (0–100). Default 50.
    pub tap_time: u8,
    /// `ECHO MODE` (0–6) — index into [`ECHO_MODES`]. Default 1 (`M`).
    pub echo_mode: u8,
    /// `ECHO BASS` (0–30). Default 15.
    pub echo_bass: u8,
    /// `ECHO TREBLE` (0–30). Default 15.
    pub echo_treble: u8,
    /// `ECHO PAN S` (0–255, centre 128).
    pub echo_pan_s: u8,
    /// `ECHO PAN M` (0–255, centre 128).
    pub echo_pan_m: u8,
    /// `ECHO PAN L` (0–255, centre 128).
    pub echo_pan_l: u8,
    /// `ECHO TAPE DIST` (0–8). Default 4.
    pub echo_tape_dist: u8,
    /// `ECHO W/F RATE` (0–255). Default 128.
    pub echo_wf_rate: u8,
    /// `ECHO W/F DEPTH` (0–255). Default 128.
    pub echo_wf_depth: u8,
    /// `DELAY RVB SEND` (0–255) — how much of the delay feeds the reverb.
    pub reverb_send: u8,
    /// `PITCH COARSE` (documented 201–237; reads 0 on the reference backup).
    pub pitch_coarse: u8,
    /// `PITCH FINE` (documented 1–201; reads 0 on the reference backup).
    pub pitch_fine: u8,
}

impl DelayParams {
    /// Parse from a `kitDly` block (at least 23 bytes).
    pub fn from_block(b: &[u8]) -> DelayParams {
        DelayParams {
            delay_type: b[0],
            tempo_sync: b[1],
            level: b[2],
            time: b[3],
            feedback: b[4],
            high_cut: b[5],
            high_damp: b[6],
            high_damp_freq: b[7],
            low_damp: b[8],
            low_damp_freq: b[9],
            tap_time: b[10],
            echo_mode: b[11],
            echo_bass: b[12],
            echo_treble: b[13],
            echo_pan_s: b[14],
            echo_pan_m: b[15],
            echo_pan_l: b[16],
            echo_tape_dist: b[17],
            echo_wf_rate: b[18],
            echo_wf_depth: b[19],
            reverb_send: b[20],
            pitch_coarse: b[21],
            pitch_fine: b[22],
        }
    }

    /// The delay type's display name.
    pub fn type_name(&self) -> Option<&'static str> {
        delay_type_name(self.delay_type)
    }

    /// The tape-echo mode's display name (meaningful when the type is
    /// `TAPE ECHO`).
    pub fn echo_mode_name(&self) -> Option<&'static str> {
        echo_mode_name(self.echo_mode)
    }

    /// Write back — inverse of [`DelayParams::from_block`] (23 bytes).
    pub fn write_to(&self, b: &mut [u8]) {
        let v = [
            self.delay_type,
            self.tempo_sync,
            self.level,
            self.time,
            self.feedback,
            self.high_cut,
            self.high_damp,
            self.high_damp_freq,
            self.low_damp,
            self.low_damp_freq,
            self.tap_time,
            self.echo_mode,
            self.echo_bass,
            self.echo_treble,
            self.echo_pan_s,
            self.echo_pan_m,
            self.echo_pan_l,
            self.echo_tape_dist,
            self.echo_wf_rate,
            self.echo_wf_depth,
            self.reverb_send,
            self.pitch_coarse,
            self.pitch_fine,
        ];
        b[..v.len()].copy_from_slice(&v);
    }
}

// --- External input sends -----------------------------------------------------

/// `kitExtIn` — the external audio input's gain/pan and its FX sends. Seven
/// bytes at [`KIT_EXT_IN_OFFSET`], then 33 bytes of reserve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtInFx {
    /// `SideChainSrc` (0–11).
    pub side_chain_src: u8,
    /// `SideChainType` (0–7).
    pub side_chain_type: u8,
    /// `SideChainDpt` (0–255).
    pub side_chain_depth: u8,
    /// `Gain` (0–161). Default 81.
    pub gain: u8,
    /// `Pan` (0–255, centre 128). Default 128.
    pub pan: u8,
    /// `ReverbSend` (0–255).
    pub reverb_send: u8,
    /// `DelaySend` (0–255).
    pub delay_send: u8,
}

impl ExtInFx {
    /// Parse from a `kitExtIn` block (at least 7 bytes).
    pub fn from_block(b: &[u8]) -> ExtInFx {
        ExtInFx {
            side_chain_src: b[0],
            side_chain_type: b[1],
            side_chain_depth: b[2],
            gain: b[3],
            pan: b[4],
            reverb_send: b[5],
            delay_send: b[6],
        }
    }

    /// Write back — inverse of [`ExtInFx::from_block`] (7 bytes).
    pub fn write_to(&self, b: &mut [u8]) {
        b[0] = self.side_chain_src;
        b[1] = self.side_chain_type;
        b[2] = self.side_chain_depth;
        b[3] = self.gain;
        b[4] = self.pan;
        b[5] = self.reverb_send;
        b[6] = self.delay_send;
    }
}

// --- The shared PRM pool ------------------------------------------------------

/// One parameter of an FX type, as `Script.xml` declares it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FxParam {
    /// The schema's field name (TR Editor's knob label).
    pub name: &'static str,
    /// Inclusive range minimum.
    pub min: u8,
    /// Inclusive range maximum.
    pub max: u8,
    /// The schema default.
    pub default: u8,
}

impl FxParam {
    /// Whether `value` is inside the schema range.
    pub fn in_range(&self, value: u8) -> bool {
        (self.min..=self.max).contains(&value)
    }
}

/// One FX type: its display name and the meaning of the `PRM` pool while it is
/// selected. `params[i]` describes `PRM{i}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FxTypeInfo {
    /// TR Editor's display name (`mfxType` / `instFxType` table).
    pub name: &'static str,
    /// `PRM00..` meanings, from the type's `Script.xml` overlay struct minus its
    /// leading `Ctrl`.
    pub params: &'static [FxParam],
    /// `false` when [`FxTypeInfo::params`] is **inferred** rather than taken
    /// from an overlay struct — the names then come from TR Editor's CTRL-target
    /// table and the ranges are unknown (placeholder `0..=255`). Only master-FX
    /// types `FATTENER` and `VINYL SIM` are in that situation.
    pub params_confirmed: bool,
}

/// Name of a master-FX type, or `None` if out of range.
pub fn mfx_type_name(t: u8) -> Option<&'static str> {
    MFX_TYPES.get(t as usize).map(|i| i.name)
}

/// Name of a per-instrument insert-FX type, or `None` if out of range.
pub fn inst_fx_type_name(t: u8) -> Option<&'static str> {
    INST_FX_TYPES.get(t as usize).map(|i| i.name)
}

/// A `Ctrl` + `PRM00..PRM23` pool paired with the type that gives it meaning.
fn named(
    table: &'static [FxTypeInfo],
    fx_type: u8,
    prm: &[u8],
    available: usize,
) -> Vec<(&'static str, u8)> {
    let Some(info) = table.get(fx_type as usize) else {
        return Vec::new();
    };
    info.params
        .iter()
        .enumerate()
        .take_while(|(i, _)| *i < available)
        .map(|(i, p)| (p.name, prm[i]))
        .collect()
}

fn out_of_range(
    table: &'static [FxTypeInfo],
    fx_type: u8,
    prm: &[u8],
    available: usize,
) -> Vec<(&'static str, u8)> {
    let Some(info) = table.get(fx_type as usize) else {
        return Vec::new();
    };
    if !info.params_confirmed {
        return Vec::new();
    }
    info.params
        .iter()
        .enumerate()
        .take_while(|(i, _)| *i < available)
        .filter(|(i, p)| !p.in_range(prm[*i]))
        .map(|(i, p)| (p.name, prm[i]))
        .collect()
}

// --- Master FX ----------------------------------------------------------------

/// `kitMfxCommon` + `kitMfxShare` — the kit-wide master effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MfxParams {
    /// `kitMfxCommon.Type` (0–20) — index into [`MFX_TYPES`]. Default 11 (`HPF`).
    pub fx_type: u8,
    /// `kitMfxCommon.Sw` — whether the master FX is engaged.
    pub switch: bool,
    /// `kitMfxShare.Ctrl` (0–15) — which parameter the CTRL knob drives. The
    /// legal values depend on the type.
    pub ctrl: u8,
    /// `kitMfxShare.PRM00..PRM23`. Interpret through [`MfxParams::params`].
    pub prm: [u8; FX_PRM_COUNT],
}

impl MfxParams {
    /// This type's parameter table (empty for an out-of-range type).
    pub fn params(&self) -> &'static [FxParam] {
        MFX_TYPES
            .get(self.fx_type as usize)
            .map(|i| i.params)
            .unwrap_or(&[])
    }

    /// The type's display name.
    pub fn type_name(&self) -> Option<&'static str> {
        mfx_type_name(self.fx_type)
    }

    /// `(name, value)` for each parameter this type actually uses.
    pub fn named_params(&self) -> Vec<(&'static str, u8)> {
        named(&MFX_TYPES, self.fx_type, &self.prm, FX_PRM_COUNT)
    }

    /// Parameters whose stored byte is outside the schema range — empty for
    /// every kit in the reference backup. Types whose table is only inferred are
    /// skipped (there is nothing to check them against).
    pub fn out_of_range(&self) -> Vec<(&'static str, u8)> {
        out_of_range(&MFX_TYPES, self.fx_type, &self.prm, FX_PRM_COUNT)
    }
}

// --- Per-instrument insert FX -------------------------------------------------

/// `instFxCommon[i]` + `instFxShare[i]` — one instrument's insert effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstFxParams {
    /// Instrument slot (0–10, TR-8S panel order).
    pub slot: usize,
    /// `instFxCommon.Type` (0–16) — index into [`INST_FX_TYPES`]. Default 12
    /// (`THRU`, i.e. bypass).
    pub fx_type: u8,
    /// `instFxShare.Ctrl` (0–255) — which parameter the CTRL knob drives.
    pub ctrl: u8,
    /// `instFxShare.PRM00..PRM23`. Bytes at or past
    /// [`InstFxParams::prm_available`] are **not in the record** and read 0.
    pub prm: [u8; FX_PRM_COUNT],
    /// How many `PRM` bytes are really stored. `24` for slots 0–9; the slot-10
    /// block starts at `+0x510` and the record ends at `+0x520`, leaving only
    /// **11**. Types needing more (`COMP+DRV`, `SATURATOR`) genuinely lose data
    /// there — see the module docs.
    pub prm_available: usize,
}

impl InstFxParams {
    /// This type's parameter table (empty for an out-of-range type).
    pub fn params(&self) -> &'static [FxParam] {
        INST_FX_TYPES
            .get(self.fx_type as usize)
            .map(|i| i.params)
            .unwrap_or(&[])
    }

    /// The type's display name.
    pub fn type_name(&self) -> Option<&'static str> {
        inst_fx_type_name(self.fx_type)
    }

    /// Whether the effect is bypassed (`THRU`).
    pub fn is_thru(&self) -> bool {
        self.type_name() == Some("THRU")
    }

    /// Whether this type needs more `PRM` bytes than the record stores.
    pub fn is_truncated(&self) -> bool {
        self.params().len() > self.prm_available
    }

    /// `(name, value)` for each parameter this type uses that is actually stored.
    pub fn named_params(&self) -> Vec<(&'static str, u8)> {
        named(&INST_FX_TYPES, self.fx_type, &self.prm, self.prm_available)
    }

    /// Parameters whose stored byte is outside the schema range — empty for
    /// every instrument of every kit in the reference backup.
    pub fn out_of_range(&self) -> Vec<(&'static str, u8)> {
        out_of_range(&INST_FX_TYPES, self.fx_type, &self.prm, self.prm_available)
    }
}

// --- Kit accessors ------------------------------------------------------------

impl Kit {
    /// Byte offset of a field within this kit record, or `None` if the record
    /// (or the field) runs past `raw`.
    fn field<'a>(&self, raw: &'a [u8], offset: usize, len: usize) -> Option<&'a [u8]> {
        let end = self.offset.checked_add(KIT_RECORD_SIZE)?;
        if end > raw.len() || offset + len > KIT_RECORD_SIZE {
            return None;
        }
        Some(&raw[self.offset + offset..self.offset + offset + len])
    }

    /// The kit's reverb settings (`kitRev`).
    pub fn reverb(&self, raw: &[u8]) -> Option<ReverbParams> {
        self.field(raw, KIT_REVERB_OFFSET, 7)
            .map(ReverbParams::from_block)
    }

    /// The kit's delay settings (`kitDly`).
    pub fn delay(&self, raw: &[u8]) -> Option<DelayParams> {
        self.field(raw, KIT_DELAY_OFFSET, 23)
            .map(DelayParams::from_block)
    }

    /// The kit's external-input gain/pan and FX sends (`kitExtIn`).
    pub fn ext_in_fx(&self, raw: &[u8]) -> Option<ExtInFx> {
        self.field(raw, KIT_EXT_IN_OFFSET, 7)
            .map(ExtInFx::from_block)
    }

    /// The kit's master effect (`kitMfxCommon` + `kitMfxShare`).
    pub fn mfx(&self, raw: &[u8]) -> Option<MfxParams> {
        let common = self.field(raw, KIT_MFX_OFFSET, 2)?;
        let share = self.field(raw, KIT_MFX_SHARE_OFFSET, 1 + FX_PRM_COUNT)?;
        let mut prm = [0u8; FX_PRM_COUNT];
        prm.copy_from_slice(&share[1..1 + FX_PRM_COUNT]);
        Some(MfxParams {
            fx_type: common[0],
            switch: common[1] != 0,
            ctrl: share[0],
            prm,
        })
    }

    /// Record offset of instrument `slot`'s insert-FX block.
    pub fn inst_fx_offset(&self, slot: usize) -> usize {
        debug_assert!(slot < INST_FX_SLOTS);
        self.offset + INST_FX_OFFSET + slot * INST_FX_STRIDE
    }

    /// Instrument `slot`'s insert effect (`instFxCommon[slot]` +
    /// `instFxShare[slot]`), 0-based in TR-8S panel order. The slot-10 block is
    /// truncated by the record end — [`InstFxParams::prm_available`] says how
    /// much of it is really there.
    pub fn inst_fx(&self, raw: &[u8], slot: usize) -> Option<InstFxParams> {
        if slot >= INST_FX_SLOTS {
            return None;
        }
        let base = INST_FX_OFFSET + slot * INST_FX_STRIDE;
        // instFxCommon.Type + 3 reserve, then instFxShare.Ctrl.
        let head = self.field(raw, base, 5)?;
        let prm_start = base + 5;
        let available = KIT_RECORD_SIZE.saturating_sub(prm_start).min(FX_PRM_COUNT);
        let stored = self.field(raw, prm_start, available)?;
        let mut prm = [0u8; FX_PRM_COUNT];
        prm[..available].copy_from_slice(stored);
        Some(InstFxParams {
            slot,
            fx_type: head[0],
            ctrl: head[4],
            prm,
            prm_available: available,
        })
    }

    /// Every instrument's insert effect, slots 0–10.
    pub fn inst_fx_all(&self, raw: &[u8]) -> Vec<InstFxParams> {
        (0..INST_FX_SLOTS)
            .filter_map(|s| self.inst_fx(raw, s))
            .collect()
    }

    /// Mutable in-range slice of a kit field, or `None` if out of range —
    /// the write counterpart to `field`.
    fn field_mut<'a>(&self, raw: &'a mut [u8], offset: usize, len: usize) -> Option<&'a mut [u8]> {
        let s = self.offset + offset;
        (offset + len <= KIT_RECORD_SIZE && s + len <= raw.len()).then_some(&mut raw[s..s + len])
    }

    /// Write the kit's reverb settings back (inverse of [`Kit::reverb`]).
    pub fn set_reverb(&self, raw: &mut [u8], p: &ReverbParams) -> bool {
        self.field_mut(raw, KIT_REVERB_OFFSET, 7)
            .map(|b| p.write_to(b))
            .is_some()
    }

    /// Write the kit's delay settings back (inverse of [`Kit::delay`]).
    pub fn set_delay(&self, raw: &mut [u8], p: &DelayParams) -> bool {
        self.field_mut(raw, KIT_DELAY_OFFSET, 23)
            .map(|b| p.write_to(b))
            .is_some()
    }

    /// Write the external-input FX settings back (inverse of [`Kit::ext_in_fx`]).
    pub fn set_ext_in_fx(&self, raw: &mut [u8], p: &ExtInFx) -> bool {
        self.field_mut(raw, KIT_EXT_IN_OFFSET, 7)
            .map(|b| p.write_to(b))
            .is_some()
    }

    /// Write the master effect back (inverse of [`Kit::mfx`]): `Type`+`Sw` into
    /// `kitMfxCommon`, `Ctrl`+`PRM00..23` into `kitMfxShare`.
    pub fn set_mfx(&self, raw: &mut [u8], m: &MfxParams) -> bool {
        if self.field_mut(raw, KIT_MFX_OFFSET, 2).is_none()
            || self
                .field_mut(raw, KIT_MFX_SHARE_OFFSET, 1 + FX_PRM_COUNT)
                .is_none()
        {
            return false;
        }
        let common = self.field_mut(raw, KIT_MFX_OFFSET, 2).unwrap();
        common[0] = m.fx_type;
        common[1] = m.switch as u8;
        let share = self
            .field_mut(raw, KIT_MFX_SHARE_OFFSET, 1 + FX_PRM_COUNT)
            .unwrap();
        share[0] = m.ctrl;
        share[1..1 + FX_PRM_COUNT].copy_from_slice(&m.prm);
        true
    }

    /// Write an instrument's insert effect back (inverse of [`Kit::inst_fx`]).
    /// Only the `prm_available` PRM bytes are written, so the truncated slot-10
    /// block (`+0x510`, 11 bytes) is respected — no write past the record end.
    pub fn set_inst_fx(&self, raw: &mut [u8], f: &InstFxParams) -> bool {
        if f.slot >= INST_FX_SLOTS {
            return false;
        }
        let base = INST_FX_OFFSET + f.slot * INST_FX_STRIDE;
        // instFxCommon.Type at +0, instFxShare.Ctrl at +4.
        let Some(head) = self.field_mut(raw, base, 5) else {
            return false;
        };
        head[0] = f.fx_type;
        head[4] = f.ctrl;
        let avail = f.prm_available.min(FX_PRM_COUNT);
        if let Some(prm) = self.field_mut(raw, base + 5, avail) {
            prm.copy_from_slice(&f.prm[..avail]);
        }
        true
    }
}

// --- Generated FX type tables -------------------------------------------------
// Derived from Script.xml: `alt` maps a type index to its overlay structType,
// whose values after the leading `Ctrl` are PRM00, PRM01, ... in order. Names,
// ranges and defaults are transcribed from those structs; the display names come
// from TR Editor's `mfxType` / `instFxType` dataTables.

const MFX_P_0: &[FxParam] = &[
    FxParam {
        name: "Balance",
        min: 0,
        max: 255,
        default: 0,
    },
    FxParam {
        name: "Attack",
        min: 0,
        max: 255,
        default: 10,
    },
    FxParam {
        name: "Release",
        min: 0,
        max: 255,
        default: 20,
    },
    FxParam {
        name: "Thre",
        min: 0,
        max: 40,
        default: 28,
    },
    FxParam {
        name: "Gain",
        min: 0,
        max: 80,
        default: 46,
    },
    FxParam {
        name: "Ratio",
        min: 0,
        max: 13,
        default: 11,
    },
    FxParam {
        name: "Knee",
        min: 0,
        max: 9,
        default: 4,
    },
];
const MFX_P_1: &[FxParam] = &[
    FxParam {
        name: "Balance",
        min: 0,
        max: 255,
        default: 0,
    },
    FxParam {
        name: "Drive",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Level",
        min: 0,
        max: 255,
        default: 45,
    },
    FxParam {
        name: "HpFreq",
        min: 0,
        max: 255,
        default: 64,
    },
    FxParam {
        name: "PreEqFreq",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "PreEqL",
        min: 0,
        max: 255,
        default: 214,
    },
    FxParam {
        name: "PreEqH",
        min: 0,
        max: 255,
        default: 174,
    },
    FxParam {
        name: "PostEqFreq",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "PostEqL",
        min: 0,
        max: 255,
        default: 214,
    },
    FxParam {
        name: "PostEqH",
        min: 0,
        max: 255,
        default: 214,
    },
];
const MFX_P_2: &[FxParam] = &[
    FxParam {
        name: "Balance",
        min: 0,
        max: 255,
        default: 0,
    },
    FxParam {
        name: "Drive",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Tone",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Level",
        min: 0,
        max: 255,
        default: 45,
    },
];
const MFX_P_3: &[FxParam] = MFX_P_2;
const MFX_P_4: &[FxParam] = MFX_P_2;
const MFX_P_5: &[FxParam] = &[
    FxParam {
        name: "Balance",
        min: 0,
        max: 255,
        default: 0,
    },
    FxParam {
        name: "SampleRate",
        min: 0,
        max: 255,
        default: 255,
    },
    FxParam {
        name: "Filter",
        min: 0,
        max: 255,
        default: 128,
    },
];
const MFX_P_6: &[FxParam] = &[
    FxParam {
        name: "Balance",
        min: 0,
        max: 255,
        default: 0,
    },
    FxParam {
        name: "Rate",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Depth",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Resonance",
        min: 0,
        max: 255,
        default: 32,
    },
    FxParam {
        name: "Manual",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Type",
        min: 0,
        max: 3,
        default: 2,
    },
    FxParam {
        name: "TempoSync",
        min: 0,
        max: 1,
        default: 1,
    },
];
const MFX_P_7: &[FxParam] = &[
    FxParam {
        name: "Balance",
        min: 0,
        max: 255,
        default: 0,
    },
    FxParam {
        name: "Rate",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Depth",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Resonance",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Manual",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "LoCutF",
        min: 0,
        max: 17,
        default: 0,
    },
    FxParam {
        name: "Mode",
        min: 0,
        max: 1,
        default: 0,
    },
    FxParam {
        name: "TempoSync",
        min: 0,
        max: 1,
        default: 1,
    },
];
const MFX_P_8: &[FxParam] = &[
    FxParam {
        name: "EnvDepth",
        min: 0,
        max: 255,
        default: 0,
    },
    FxParam {
        name: "Attack",
        min: 0,
        max: 255,
        default: 255,
    },
    FxParam {
        name: "Release",
        min: 0,
        max: 255,
        default: 0,
    },
];
const MFX_P_9: &[FxParam] = &[
    FxParam {
        name: "EnvDepth",
        min: 0,
        max: 255,
        default: 0,
    },
    FxParam {
        name: "Attack",
        min: 0,
        max: 255,
        default: 255,
    },
    FxParam {
        name: "Release",
        min: 0,
        max: 255,
        default: 0,
    },
    FxParam {
        name: "Q",
        min: 0,
        max: 7,
        default: 3,
    },
    FxParam {
        name: "HP Level",
        min: 0,
        max: 255,
        default: 0,
    },
    FxParam {
        name: "BP Level",
        min: 0,
        max: 255,
        default: 0,
    },
    FxParam {
        name: "LP Level",
        min: 0,
        max: 255,
        default: 214,
    },
    FxParam {
        name: "Bypass",
        min: 0,
        max: 255,
        default: 0,
    },
];
const MFX_P_10: &[FxParam] = &[
    FxParam {
        name: "Depth",
        min: 0,
        max: 255,
        default: 0,
    },
    FxParam {
        name: "Resonance",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Type",
        min: 0,
        max: 2,
        default: 2,
    },
    FxParam {
        name: "Gain",
        min: 0,
        max: 80,
        default: 40,
    },
    FxParam {
        name: "Clipper",
        min: 0,
        max: 1,
        default: 1,
    },
];
const MFX_P_11: &[FxParam] = MFX_P_10;
const MFX_P_12: &[FxParam] = &[
    FxParam {
        name: "Depth",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Resonance",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Type",
        min: 0,
        max: 2,
        default: 2,
    },
    FxParam {
        name: "Gain",
        min: 0,
        max: 80,
        default: 40,
    },
    FxParam {
        name: "Clipper",
        min: 0,
        max: 1,
        default: 1,
    },
];
const MFX_P_13: &[FxParam] = &[
    FxParam {
        name: "Boost",
        min: 0,
        max: 255,
        default: 0,
    },
    FxParam {
        name: "Frequency",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Gain",
        min: 0,
        max: 80,
        default: 40,
    },
];
const MFX_P_14: &[FxParam] = MFX_P_13;
const MFX_P_15: &[FxParam] = &[
    FxParam {
        name: "Boost",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Frequency",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Gain",
        min: 0,
        max: 80,
        default: 40,
    },
];
const MFX_P_16: &[FxParam] = &[
    FxParam {
        name: "Balance",
        min: 0,
        max: 255,
        default: 255,
    },
    FxParam {
        name: "Low",
        min: 0,
        max: 255,
        default: 0,
    },
    FxParam {
        name: "Mid",
        min: 0,
        max: 255,
        default: 0,
    },
    FxParam {
        name: "High",
        min: 0,
        max: 255,
        default: 0,
    },
];
const MFX_P_17: &[FxParam] = &[
    FxParam {
        name: "Balance",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Band Interval",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Band Width",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Type",
        min: 0,
        max: 5,
        default: 1,
    },
    FxParam {
        name: "Gain",
        min: 0,
        max: 255,
        default: 255,
    },
];
const MFX_P_18: &[FxParam] = &[
    FxParam {
        name: "Color",
        min: 0,
        max: 255,
        default: 0,
    },
    FxParam {
        name: "Level",
        min: 0,
        max: 255,
        default: 214,
    },
    FxParam {
        name: "Direction",
        min: 0,
        max: 1,
        default: 0,
    },
];
// INFERRED — no overlay struct exists for types 19/20. Names from TR Editor's
// `mfxCtrl` CTRL-target table; ranges/defaults unknown.
const MFX_P_19: &[FxParam] = &[
    FxParam {
        name: "Depth",
        min: 0,
        max: 255,
        default: 0,
    },
    FxParam {
        name: "Level",
        min: 0,
        max: 255,
        default: 0,
    },
];
const MFX_P_20: &[FxParam] = &[
    FxParam {
        name: "Compressor",
        min: 0,
        max: 255,
        default: 0,
    },
    FxParam {
        name: "Noise",
        min: 0,
        max: 255,
        default: 0,
    },
    FxParam {
        name: "Wow Flut",
        min: 0,
        max: 255,
        default: 0,
    },
];

/// The 21 master-FX types, indexed by `kitMfxCommon.Type`.
pub const MFX_TYPES: [FxTypeInfo; 21] = [
    FxTypeInfo {
        name: "COMPRESSOR",
        params: MFX_P_0,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "DRIVE",
        params: MFX_P_1,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "OVERDRIVE",
        params: MFX_P_2,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "DISTORTION",
        params: MFX_P_3,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "FUZZ",
        params: MFX_P_4,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "CRUSHER",
        params: MFX_P_5,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "PHASER",
        params: MFX_P_6,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "FLANGER",
        params: MFX_P_7,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "TRANSIENT",
        params: MFX_P_8,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "TRANSIENT2",
        params: MFX_P_9,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "LPF",
        params: MFX_P_10,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "HPF",
        params: MFX_P_11,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "LPF/HPF",
        params: MFX_P_12,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "L BOOST",
        params: MFX_P_13,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "H BOOST",
        params: MFX_P_14,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "L/H BOOST",
        params: MFX_P_15,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "ISOLATOR",
        params: MFX_P_16,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "SBF",
        params: MFX_P_17,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "NOISE",
        params: MFX_P_18,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "FATTENER",
        params: MFX_P_19,
        params_confirmed: false,
    },
    FxTypeInfo {
        name: "VINYL SIM",
        params: MFX_P_20,
        params_confirmed: false,
    },
];

const IFX_P_0: &[FxParam] = &[
    FxParam {
        name: "Balance",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Attack",
        min: 0,
        max: 255,
        default: 158,
    },
    FxParam {
        name: "Release",
        min: 0,
        max: 255,
        default: 49,
    },
    FxParam {
        name: "Thre",
        min: 0,
        max: 40,
        default: 8,
    },
    FxParam {
        name: "Gain",
        min: 0,
        max: 80,
        default: 47,
    },
    FxParam {
        name: "Ratio",
        min: 0,
        max: 13,
        default: 3,
    },
    FxParam {
        name: "Knee",
        min: 0,
        max: 9,
        default: 0,
    },
];
const IFX_P_1: &[FxParam] = &[
    FxParam {
        name: "Balance",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Drive",
        min: 0,
        max: 255,
        default: 192,
    },
    FxParam {
        name: "Level",
        min: 0,
        max: 255,
        default: 45,
    },
    FxParam {
        name: "HpFreq",
        min: 0,
        max: 255,
        default: 64,
    },
    FxParam {
        name: "PreEqFreq",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "PreEqL",
        min: 0,
        max: 255,
        default: 214,
    },
    FxParam {
        name: "PreEqH",
        min: 0,
        max: 255,
        default: 174,
    },
    FxParam {
        name: "PostEqFreq",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "PostEqL",
        min: 0,
        max: 255,
        default: 214,
    },
    FxParam {
        name: "PostEqH",
        min: 0,
        max: 255,
        default: 214,
    },
];
const IFX_P_2: &[FxParam] = &[
    FxParam {
        name: "Balance",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "SampleRate",
        min: 0,
        max: 255,
        default: 255,
    },
    FxParam {
        name: "Filter",
        min: 0,
        max: 255,
        default: 128,
    },
];
const IFX_P_3: &[FxParam] = &[
    FxParam {
        name: "Balance",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "CmpBalance",
        min: 0,
        max: 255,
        default: 255,
    },
    FxParam {
        name: "DrvBalance",
        min: 0,
        max: 255,
        default: 255,
    },
    FxParam {
        name: "CmpAttack",
        min: 0,
        max: 255,
        default: 158,
    },
    FxParam {
        name: "CmpRelease",
        min: 0,
        max: 255,
        default: 49,
    },
    FxParam {
        name: "CmpThre",
        min: 0,
        max: 40,
        default: 8,
    },
    FxParam {
        name: "CmpGain",
        min: 0,
        max: 80,
        default: 47,
    },
    FxParam {
        name: "CmpRatio",
        min: 0,
        max: 13,
        default: 3,
    },
    FxParam {
        name: "CmpKnee",
        min: 0,
        max: 9,
        default: 0,
    },
    FxParam {
        name: "DrvDrive",
        min: 0,
        max: 255,
        default: 192,
    },
    FxParam {
        name: "DrvLevel",
        min: 0,
        max: 255,
        default: 45,
    },
    FxParam {
        name: "DrvHpF",
        min: 0,
        max: 255,
        default: 64,
    },
    FxParam {
        name: "DrvPreF",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "DrvPreL",
        min: 0,
        max: 255,
        default: 214,
    },
    FxParam {
        name: "DrvPreH",
        min: 0,
        max: 255,
        default: 174,
    },
    FxParam {
        name: "DrvPstF",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "DrvPstL",
        min: 0,
        max: 255,
        default: 214,
    },
    FxParam {
        name: "DrvPstH",
        min: 0,
        max: 255,
        default: 214,
    },
];
const IFX_P_4: &[FxParam] = &[
    FxParam {
        name: "EnvDepth",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Attack",
        min: 0,
        max: 255,
        default: 255,
    },
    FxParam {
        name: "Release",
        min: 0,
        max: 255,
        default: 0,
    },
];
const IFX_P_5: &[FxParam] = &[
    FxParam {
        name: "Depth",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Resonance",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Type",
        min: 0,
        max: 2,
        default: 2,
    },
    FxParam {
        name: "Gain",
        min: 0,
        max: 80,
        default: 40,
    },
    FxParam {
        name: "Clipper",
        min: 0,
        max: 1,
        default: 1,
    },
];
const IFX_P_6: &[FxParam] = IFX_P_5;
const IFX_P_7: &[FxParam] = IFX_P_5;
const IFX_P_8: &[FxParam] = &[
    FxParam {
        name: "Boost",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Frequency",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Gain",
        min: 0,
        max: 80,
        default: 40,
    },
];
const IFX_P_9: &[FxParam] = IFX_P_8;
const IFX_P_10: &[FxParam] = IFX_P_8;
const IFX_P_11: &[FxParam] = &[
    FxParam {
        name: "Balance",
        min: 0,
        max: 255,
        default: 255,
    },
    FxParam {
        name: "Low",
        min: 0,
        max: 255,
        default: 0,
    },
    FxParam {
        name: "Mid",
        min: 0,
        max: 255,
        default: 255,
    },
    FxParam {
        name: "High",
        min: 0,
        max: 255,
        default: 0,
    },
];
/// `THRU` is a true bypass: its overlay struct holds only the shared `Ctrl`.
const IFX_P_12: &[FxParam] = &[];
const IFX_P_13: &[FxParam] = &[
    FxParam {
        name: "Drive",
        min: 0,
        max: 48,
        default: 11,
    },
    FxParam {
        name: "PreType",
        min: 0,
        max: 4,
        default: 0,
    },
    FxParam {
        name: "PreFreq",
        min: 0,
        max: 255,
        default: 65,
    },
    FxParam {
        name: "PreGain",
        min: 0,
        max: 48,
        default: 24,
    },
    FxParam {
        name: "Post1Type",
        min: 0,
        max: 4,
        default: 3,
    },
    FxParam {
        name: "Post1Freq",
        min: 0,
        max: 255,
        default: 88,
    },
    FxParam {
        name: "Post1Gain",
        min: 0,
        max: 48,
        default: 24,
    },
    FxParam {
        name: "Post2Type",
        min: 0,
        max: 4,
        default: 4,
    },
    FxParam {
        name: "Post2Freq",
        min: 0,
        max: 255,
        default: 203,
    },
    FxParam {
        name: "Post2Gain",
        min: 0,
        max: 48,
        default: 24,
    },
    FxParam {
        name: "Post3Type",
        min: 0,
        max: 4,
        default: 4,
    },
    FxParam {
        name: "Post3Freq",
        min: 0,
        max: 255,
        default: 149,
    },
    FxParam {
        name: "Post3Gain",
        min: 0,
        max: 48,
        default: 24,
    },
    FxParam {
        name: "Post3Q",
        min: 5,
        max: 160,
        default: 5,
    },
    FxParam {
        name: "Sense",
        min: 0,
        max: 60,
        default: 12,
    },
    FxParam {
        name: "PostGain",
        min: 0,
        max: 60,
        default: 48,
    },
    FxParam {
        name: "Balance",
        min: 0,
        max: 255,
        default: 255,
    },
    FxParam {
        name: "Level",
        min: 0,
        max: 255,
        default: 255,
    },
];
const IFX_P_14: &[FxParam] = &[
    FxParam {
        name: "Freq",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Fine",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Balance",
        min: 0,
        max: 255,
        default: 255,
    },
];
const IFX_P_15: &[FxParam] = &[
    FxParam {
        name: "Freq",
        min: 0,
        max: 255,
        default: 0,
    },
    FxParam {
        name: "Fine",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Balance",
        min: 0,
        max: 255,
        default: 255,
    },
];
const IFX_P_16: &[FxParam] = &[
    FxParam {
        name: "Rate",
        min: 0,
        max: 255,
        default: 128,
    },
    FxParam {
        name: "Mode",
        min: 0,
        max: 1,
        default: 0,
    },
    FxParam {
        name: "Balance",
        min: 0,
        max: 255,
        default: 255,
    },
];

/// The 17 per-instrument insert-FX types, indexed by `instFxCommon.Type`.
pub const INST_FX_TYPES: [FxTypeInfo; 17] = [
    FxTypeInfo {
        name: "COMPRESSOR",
        params: IFX_P_0,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "DRIVE",
        params: IFX_P_1,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "CRUSHER",
        params: IFX_P_2,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "COMP+DRV",
        params: IFX_P_3,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "TRANSIENT",
        params: IFX_P_4,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "LPF",
        params: IFX_P_5,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "HPF",
        params: IFX_P_6,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "LPF/HPF",
        params: IFX_P_7,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "L BOOST",
        params: IFX_P_8,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "H BOOST",
        params: IFX_P_9,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "L/H BOOST",
        params: IFX_P_10,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "ISOLATOR",
        params: IFX_P_11,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "THRU",
        params: IFX_P_12,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "SATURATOR",
        params: IFX_P_13,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "FREQ SHIFT",
        params: IFX_P_14,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "RING MOD",
        params: IFX_P_15,
        params_confirmed: true,
    },
    FxTypeInfo {
        name: "SPREAD",
        params: IFX_P_16,
        params_confirmed: true,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Backup, HEADER_LEN};

    /// A synthetic backup with `n` kit records. No Roland bytes.
    fn synthetic_kits(n: usize) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"TR6S");
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&5u32.to_le_bytes());
        v.resize(HEADER_LEN, 0);
        v.extend_from_slice(b"KIT ");
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&((n * KIT_RECORD_SIZE) as u32).to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.resize(v.len() + n * KIT_RECORD_SIZE, 0);
        v
    }

    fn record_mut(v: &mut [u8], index: usize) -> &mut [u8] {
        let base = HEADER_LEN + 16 + index * KIT_RECORD_SIZE;
        &mut v[base..base + KIT_RECORD_SIZE]
    }

    #[test]
    fn reverb_lands_on_the_schema_defaults() {
        // kitRev at its documented defaults: HALL1 / 150 / 0 / 20 / 2 / 11 / 10.
        let mut v = synthetic_kits(1);
        record_mut(&mut v, 0)[KIT_REVERB_OFFSET..KIT_REVERB_OFFSET + 7]
            .copy_from_slice(&[2, 150, 0, 20, 2, 11, 10]);
        let b = Backup::parse(v).unwrap();
        let rev = b.kits()[0].reverb(b.raw()).unwrap();
        assert_eq!(rev.reverb_type, 2);
        assert_eq!(rev.type_name(), Some("HALL1"));
        assert_eq!(rev.time, 150);
        assert_eq!(rev.level, 0);
        assert_eq!(rev.pre_delay, 20);
        assert_eq!(rev.low_cut, 2);
        assert_eq!(rev.high_cut, 11);
        assert_eq!(rev.density, 10);
        assert_eq!(reverb_type_name(6), Some("HA-DOU"));
        assert_eq!(reverb_type_name(7), None);
    }

    #[test]
    fn delay_lands_on_the_schema_defaults() {
        // kitDly at its documented defaults, ending with the two PITCH SHFT
        // bytes that read 0 on the reference backup.
        let defaults: [u8; 23] = [
            0, 0, 0, 104, 120, 7, 0, 13, 0, 0, 50, 1, 15, 15, 128, 128, 128, 4, 128, 128, 0, 0, 0,
        ];
        let mut v = synthetic_kits(1);
        record_mut(&mut v, 0)[KIT_DELAY_OFFSET..KIT_DELAY_OFFSET + 23].copy_from_slice(&defaults);
        let b = Backup::parse(v).unwrap();
        let d = b.kits()[0].delay(b.raw()).unwrap();
        assert_eq!(d.type_name(), Some("DLY"));
        assert_eq!(d.time, 104);
        assert_eq!(d.feedback, 120);
        assert_eq!(d.high_cut, 7);
        assert_eq!(d.high_damp_freq, 13);
        assert_eq!(d.tap_time, 50);
        assert_eq!(d.echo_mode_name(), Some("M"));
        assert_eq!(d.echo_bass, 15);
        assert_eq!(d.echo_treble, 15);
        assert_eq!(d.echo_tape_dist, 4);
        assert_eq!(d.echo_wf_depth, 128);
        assert_eq!(d.reverb_send, 0);
        // PITCH SHFT is never selected by a factory kit, so these read 0 —
        // below their documented ranges, and left uninterpreted.
        assert_eq!((d.pitch_coarse, d.pitch_fine), (0, 0));
        assert_eq!(delay_type_name(3), Some("PITCH SHFT"));
        assert_eq!(delay_type_name(4), None);
    }

    #[test]
    fn ext_in_sends() {
        let mut v = synthetic_kits(1);
        record_mut(&mut v, 0)[KIT_EXT_IN_OFFSET..KIT_EXT_IN_OFFSET + 7]
            .copy_from_slice(&[0, 0, 0, 81, 128, 40, 60]);
        let b = Backup::parse(v).unwrap();
        let e = b.kits()[0].ext_in_fx(b.raw()).unwrap();
        assert_eq!(e.gain, 81);
        assert_eq!(e.pan, 128);
        assert_eq!(e.reverb_send, 40);
        assert_eq!(e.delay_send, 60);
    }

    #[test]
    fn mfx_prm_pool_is_named_by_its_type() {
        // Type 12 = LPF/HPF: PRM00 Depth, PRM01 Resonance, PRM02 Type,
        // PRM03 Gain, PRM04 Clipper — the mapping TR Editor's EFX panel binds.
        let mut v = synthetic_kits(1);
        {
            let r = record_mut(&mut v, 0);
            r[KIT_MFX_OFFSET] = 12;
            r[KIT_MFX_OFFSET + 1] = 1; // Sw on
            r[KIT_MFX_SHARE_OFFSET] = 3; // Ctrl
            r[KIT_MFX_SHARE_OFFSET + 1..KIT_MFX_SHARE_OFFSET + 6]
                .copy_from_slice(&[131, 128, 2, 40, 1]);
        }
        let b = Backup::parse(v).unwrap();
        let mfx = b.kits()[0].mfx(b.raw()).unwrap();
        assert_eq!(mfx.fx_type, 12);
        assert!(mfx.switch);
        assert_eq!(mfx.ctrl, 3);
        assert_eq!(mfx.type_name(), Some("LPF/HPF"));
        assert_eq!(
            mfx.named_params(),
            vec![
                ("Depth", 131),
                ("Resonance", 128),
                ("Type", 2),
                ("Gain", 40),
                ("Clipper", 1),
            ]
        );
        assert!(mfx.out_of_range().is_empty());
    }

    #[test]
    fn mfx_out_of_range_is_reported() {
        // Gain's schema range is 0..=80; 200 must be flagged rather than shown
        // as if it were a valid reading.
        let mut v = synthetic_kits(1);
        {
            let r = record_mut(&mut v, 0);
            r[KIT_MFX_OFFSET] = 12;
            r[KIT_MFX_SHARE_OFFSET + 1..KIT_MFX_SHARE_OFFSET + 6]
                .copy_from_slice(&[131, 128, 2, 200, 1]);
        }
        let b = Backup::parse(v).unwrap();
        let mfx = b.kits()[0].mfx(b.raw()).unwrap();
        assert_eq!(mfx.out_of_range(), vec![("Gain", 200)]);

        // An unknown type yields no names and no false range reports.
        let mut v = synthetic_kits(1);
        record_mut(&mut v, 0)[KIT_MFX_OFFSET] = 99;
        let b = Backup::parse(v).unwrap();
        let mfx = b.kits()[0].mfx(b.raw()).unwrap();
        assert_eq!(mfx.type_name(), None);
        assert!(mfx.params().is_empty());
        assert!(mfx.named_params().is_empty());
        assert!(mfx.out_of_range().is_empty());
    }

    #[test]
    fn inst_fx_blocks_are_strided_by_32() {
        let mut v = synthetic_kits(1);
        {
            let r = record_mut(&mut v, 0);
            for slot in 0..INST_FX_SLOTS {
                let o = INST_FX_OFFSET + slot * INST_FX_STRIDE;
                r[o] = 5; // LPF
                r[o + 4] = slot as u8; // Ctrl
                r[o + 5..o + 10.min(KIT_RECORD_SIZE - o)].copy_from_slice(&[128, 128, 2, 40, 1]);
            }
        }
        let b = Backup::parse(v).unwrap();
        let kit = b.kits()[0];
        let all = kit.inst_fx_all(b.raw());
        assert_eq!(all.len(), INST_FX_SLOTS);
        for (slot, fx) in all.iter().enumerate() {
            assert_eq!(fx.slot, slot);
            assert_eq!(fx.type_name(), Some("LPF"));
            assert_eq!(fx.ctrl, slot as u8);
            assert_eq!(
                fx.named_params(),
                vec![
                    ("Depth", 128),
                    ("Resonance", 128),
                    ("Type", 2),
                    ("Gain", 40),
                    ("Clipper", 1),
                ]
            );
            assert!(fx.out_of_range().is_empty());
        }
        assert_eq!(kit.inst_fx_offset(1), kit.offset + INST_FX_OFFSET + 32);
        assert_eq!(kit.inst_fx(b.raw(), INST_FX_SLOTS), None);
    }

    #[test]
    fn the_last_inst_fx_block_is_truncated_by_the_record() {
        // Slot 10's block starts at +0x510 and the record ends at +0x520, so
        // only 11 of its 24 PRM bytes exist. That is a fact about the format,
        // not something to paper over.
        let v = synthetic_kits(1);
        let b = Backup::parse(v).unwrap();
        let kit = b.kits()[0];
        for slot in 0..INST_FX_SLOTS - 1 {
            assert_eq!(
                kit.inst_fx(b.raw(), slot).unwrap().prm_available,
                FX_PRM_COUNT
            );
        }
        let last = kit.inst_fx(b.raw(), INST_FX_SLOTS - 1).unwrap();
        assert_eq!(kit.inst_fx_offset(10), kit.offset + 0x510);
        assert_eq!(last.prm_available, 11);
        assert_eq!(
            INST_FX_OFFSET + (INST_FX_SLOTS - 1) * INST_FX_STRIDE + 5 + 11,
            KIT_RECORD_SIZE
        );
        // THRU (the default) needs no PRM bytes, so it survives the truncation.
        assert!(!last.is_truncated());
    }

    #[test]
    fn truncation_is_flagged_for_types_that_need_the_lost_bytes() {
        let mut v = synthetic_kits(1);
        {
            let r = record_mut(&mut v, 0);
            let o = INST_FX_OFFSET + 10 * INST_FX_STRIDE;
            r[o] = 3; // COMP+DRV: 18 params, only 11 stored
        }
        let b = Backup::parse(v).unwrap();
        let fx = b.kits()[0].inst_fx(b.raw(), 10).unwrap();
        assert_eq!(fx.type_name(), Some("COMP+DRV"));
        assert_eq!(fx.params().len(), 18);
        assert!(fx.is_truncated());
        // Only the stored PRMs are reported — no invented values.
        assert_eq!(fx.named_params().len(), 11);
    }

    #[test]
    fn thru_is_the_default_inst_fx_and_is_bypass() {
        assert_eq!(inst_fx_type_name(12), Some("THRU"));
        assert_eq!(inst_fx_type_name(17), None);
        assert!(INST_FX_TYPES[12].params.is_empty());
        let mut v = synthetic_kits(1);
        record_mut(&mut v, 0)[INST_FX_OFFSET] = 12;
        let b = Backup::parse(v).unwrap();
        let fx = b.kits()[0].inst_fx(b.raw(), 0).unwrap();
        assert!(fx.is_thru());
        assert!(fx.named_params().is_empty());
    }

    #[test]
    fn type_tables_match_the_editor_name_tables() {
        assert_eq!(MFX_TYPES.len(), 21);
        assert_eq!(INST_FX_TYPES.len(), 17);
        assert_eq!(mfx_type_name(0), Some("COMPRESSOR"));
        assert_eq!(mfx_type_name(11), Some("HPF")); // the schema default
        assert_eq!(mfx_type_name(20), Some("VINYL SIM"));
        assert_eq!(mfx_type_name(21), None);
        // The two types with no overlay struct are the only inferred ones.
        let inferred: Vec<_> = MFX_TYPES
            .iter()
            .filter(|i| !i.params_confirmed)
            .map(|i| i.name)
            .collect();
        assert_eq!(inferred, vec!["FATTENER", "VINYL SIM"]);
        assert!(INST_FX_TYPES.iter().all(|i| i.params_confirmed));
    }

    #[test]
    fn fx_layout_constants_are_self_consistent() {
        // The sub-struct chain: every boundary is 4-byte aligned and the sizes
        // accumulate onto the confirmed instCommon[0] offset (0x194).
        for o in [
            KIT_REVERB_OFFSET,
            KIT_DELAY_OFFSET,
            KIT_MFX_OFFSET,
            KIT_MFX_SHARE_OFFSET,
            KIT_EXT_IN_OFFSET,
            INST_FX_OFFSET,
        ] {
            assert_eq!(o % 4, 0, "sub-struct offsets are 4-byte aligned");
        }
        assert_eq!(KIT_REVERB_OFFSET + 40, KIT_DELAY_OFFSET);
        assert_eq!(KIT_DELAY_OFFSET + 52, KIT_MFX_OFFSET);
        assert_eq!(KIT_MFX_OFFSET + 20, KIT_MFX_SHARE_OFFSET);
        assert_eq!(KIT_MFX_SHARE_OFFSET + 28, KIT_EXT_IN_OFFSET);
        // instCommon[0] at 0x194 + 11 voice blocks of 0x34 lands on the
        // insert-FX region.
        assert_eq!(
            crate::VOICE_TONE_ID_OFFSET + 11 * crate::VOICE_STRIDE,
            INST_FX_OFFSET
        );
    }

    #[test]
    fn out_of_bounds_reads_are_none_not_panics() {
        // A record that runs past the end of the buffer must not be decoded.
        let mut v = synthetic_kits(1);
        v.truncate(v.len() - 1);
        let b = Backup::parse(v).unwrap();
        let kit = b.kits().first().copied();
        if let Some(kit) = kit {
            assert_eq!(kit.reverb(b.raw()), None);
            assert_eq!(kit.delay(b.raw()), None);
            assert_eq!(kit.mfx(b.raw()), None);
            assert_eq!(kit.ext_in_fx(b.raw()), None);
            assert_eq!(kit.inst_fx(b.raw(), 0), None);
            assert!(kit.inst_fx_all(b.raw()).is_empty());
        }
    }

    #[test]
    fn fx_setters_round_trip_and_stay_lossless() {
        let mut b = Backup::parse(synthetic_kits(1)).unwrap();
        let kit = b.kits()[0];
        let orig = b.to_bytes();

        // reverb / delay / ext-in write-back round-trips.
        let mut rv = kit.reverb(b.raw()).unwrap();
        rv.reverb_type = 2;
        rv.time = 150;
        assert!(kit.set_reverb(b.raw_mut(), &rv));
        assert_eq!(kit.reverb(b.raw()), Some(rv));

        let mut dl = kit.delay(b.raw()).unwrap();
        dl.feedback = 120;
        dl.echo_mode = 1;
        assert!(kit.set_delay(b.raw_mut(), &dl));
        assert_eq!(kit.delay(b.raw()), Some(dl));

        let mut ei = kit.ext_in_fx(b.raw()).unwrap();
        ei.gain = 81;
        assert!(kit.set_ext_in_fx(b.raw_mut(), &ei));
        assert_eq!(kit.ext_in_fx(b.raw()), Some(ei));

        // master FX: type + switch + ctrl + PRM pool.
        let mut m = kit.mfx(b.raw()).unwrap();
        m.fx_type = 3;
        m.switch = true;
        m.ctrl = 5;
        m.prm[0] = 100;
        m.prm[23] = 7;
        assert!(kit.set_mfx(b.raw_mut(), &m));
        assert_eq!(kit.mfx(b.raw()), Some(m));

        // insert FX for a non-truncated slot round-trips fully.
        let mut f = kit.inst_fx(b.raw(), 0).unwrap();
        f.fx_type = 5;
        f.ctrl = 9;
        f.prm[0] = 42;
        assert!(kit.set_inst_fx(b.raw_mut(), &f));
        assert_eq!(kit.inst_fx(b.raw(), 0), Some(f));

        // Losslessness: every changed byte is inside this kit record; and the
        // truncated slot-10 write never runs past the record end.
        let after = b.to_bytes();
        let rec = kit.offset..kit.offset + KIT_RECORD_SIZE;
        for i in 0..orig.len() {
            if orig[i] != after[i] {
                assert!(
                    rec.contains(&i),
                    "byte 0x{i:x} changed outside the kit record"
                );
            }
        }
        let f10 = kit.inst_fx(b.raw(), 10).unwrap();
        assert!(f10.prm_available < FX_PRM_COUNT);
        assert!(kit.set_inst_fx(b.raw_mut(), &f10)); // must not panic / overrun
    }
}
