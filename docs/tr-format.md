# TR-6S / TR-8S user-data format

Reverse-engineering notes for the TR device **backup** container and its
sections. **Plaintext user data only** — this has nothing to do with the
encrypted `App1_Main` firmware image or its key. Basis: a v1.51 TR-6S SD-card
backup (`TR-6S/BACKUP/tr6s_bak.bin` + `tr6s_bak.txt` manifest). No user or
factory content is committed; real backups are gitignored.

Implemented by the [`tr-format`](../crates/tr-format) crate.

## The manifest (`*_bak.txt`) — free ground truth

A human-readable table of contents that pairs with the binary. Header line
gives the firmware version (`Version 1.51 (0BDE)`), then:

- `[PATTERN]` — 128 entries, `bank-num = "16-char name" ; <ABCDEFGH track flags> <1/2/S variation> Kit NNN Tempo T.T [Motion ON]`.
- `[KIT]` — 128 entries, each `NNN = "16-char kit name"` followed by its **6
  voice slots** — `BD / SD / LT / HC / CH / OH` (the six TR-6S voices), each a
  16-char tone name.

Because the names are 16-char ASCII, they are searchable anchors into the
binary — the manifest tells us *what* is at each record index, which is how the
record internals get reversed.

## Container format (`*_bak.bin`)

A 64-byte file header, then a sequence of 16-byte-header chunks.

### File header (0x00–0x3F)

| Offset | Bytes | Meaning |
| ------ | ----- | ------- |
| `0x00` | 4 | magic `TR6S` (or `TR8S`) |
| `0x04` | 4 | 0 |
| `0x08` | 4 | format version (`5` observed) |
| `0x0C` | 4 | 0 |
| `0x10` | 16 | name field (spaces when blank) |
| `0x20` | 10 | content-correlated **token** (`0x20`–`0x29`) — **not** a plain checksum; unresolved (see below) |
| `0x2A` | 18 | zero |
| `0x38` | 4 | constant `18 E9 FF 13` (same in every backup) |
| `0x3C` | 4 | **header CRC-32** (`u32` LE) — **SOLVED** (see below) |

#### Header CRC-32 at `0x3C` — SOLVED

The word at `0x3C` is a standard **CRC-32** (zlib/IEEE: poly `0x04C11DB7`,
reflected, init & xorout `0xFFFFFFFF` — the same variant as the firmware) over
the **header bytes before it, `[0x00, 0x3C)`**, stored little-endian. Recovered
with two independent `(header, CRC)` pairs — the v1.51 backup
(`crc32(hdr[0:0x3C]) = 0xEC79C172`) and the factory image decompressed from
`init_param` — both reproduced exactly, and re-confirmed through `fw-analyze
checksum --offset 0 --len 60`. Implemented as `Backup::header_crc` /
`computed_header_crc` / `header_crc_valid` / `recompute_header_crc` (+ a
dependency-free `crc32`).

**Practical payoff:** a written backup whose header changed can be made
header-valid again with one `recompute_header_crc()`. Section-*content* edits
(past `0x40`) do not touch this CRC.

#### The `0x20` token — content-correlated, unresolved

The 10 bytes at `0x20`–`0x29` differ between the two corpora, so they track
content — but they are **not** a plain checksum: their **low nibbles are
constant** across both backups (`1,4,3,3,3,3,3,3`) while the high nibbles vary,
and no CRC/sum over the content reproduces them. That is the signature of the
same keyed/device-generated **token family** as the `+0x08` section token — see
the `+0x08` section below. Left raw; it matters only for a device-safe write of
edited *content*, and settling it needs the same hardware save-change-save test.

### Chunk (from 0x40 onward)

| Offset | Bytes | Meaning |
| ------ | ----- | ------- |
| `+0x00` | 4 | tag, e.g. `SYS `, `PTN `, `KIT `, `SMPL`, `FX  ` |
| `+0x04` | 4 | reserved (0) — a reliable discriminator vs. 16-char name fields |
| `+0x08` | 4 | payload size (LE) |
| `+0x0C` | 4 | extra (0 for most; `SMPL` carries the reserved sample-region size) |
| `+0x10` | … | payload |

Observed on the reference backup:

| Tag | Header @ | Payload | Notes |
| --- | -------- | ------- | ----- |
| `SYS ` | `0x40` | 752 B | System params — **decoded** (see "System record" below). Array of **1 record × 752 B**, byte-identical to the firmware `init_param` factory image. |
| `PTN ` | `0x350` | 3,136,512 B | Array: **128 records × 24,504 B** (`0x5FB8`). Payload begins `count(u32)=128, record_size(u32)=0x5FB8`. |
| `KIT ` | `0x2FDF70` | 167,936 B | Array: **128 records × 1,312 B** (`0x520`). Same `count,record_size` preamble. |
| `TONE` | `0x326F90` | 36,864 B | Tone table: **1024 entries × 36 B** (`0x24`). |
| `PCMT` | `0x32FFB0` | 65,536 B | PCM-tone table: **1024 records × 64 B** (`0x40`). |
| `SMPL` | `0x33FFD0` | 0 B | User samples; empty here (none loaded). `extra` ≈ 0x3300000 = the reserved sample region — accounts for the ~51 MB of zero padding to EOF. |

**There is no `FX  ` chunk** — earlier notes said one was "seen ×4"; that was a
false positive, corrected below.

Array sections self-describe with a `count,record_size` preamble, so the parser
locates records generically. The "preamble" is not a separate structure: it is
**record 0's own 16-byte header**, whose first two words happen to be `count`
and `record_size` and whose `+0x08` field is the section token (below). That is
why `128 × 0x5FB8` fills the `PTN ` payload exactly with nothing left over —
confirmed for `SYS `, `PTN `, `KIT ` and `TONE`.

## Kit record (`KIT ` section)

128 records of **`0x520` (1,312) bytes**, back-to-back (no preamble; `128 ×
0x520` fills the payload exactly). Verified by extracting all 128 names and
matching the manifest.

| Offset | Len | Field | Status |
| ------ | --- | ----- | ------ |
| `+0x00` | 16 | per-record header — flags + a checksum-like field (`+0x08`, 8 B) | **partial** — the `+0x08` field is not a plain CRC-32 of the body; treat as an opaque per-record hash. Matters only when *writing* a kit back. |
| `+0x10` | 16 | **kit name** (ASCII, space/NUL-padded) | **done** |
| `+0x20` | 0x500 | kit params + 6 voice configs (BD/SD/LT/HC/CH/OH) | **TODO** |

### Voices & the `TONE` table — tentative (single-backup RE)

Voices reference tones by **ID**, resolved through a `TONE` section that follows
the KIT section:

- `TONE` chunk `@0x326F90`. Payload begins with a 16-byte preamble, then
  **`0x24` (36-byte) entries**: `name[16]` + `params[20]`. Tone-ID 0 is the
  first entry after the preamble.
- Each kit record holds **6 voice tone-IDs** (`u16` LE) at
  `record + 0x194 + voice*0x34` (stride `0x34`), voices in order
  **BD, SD, LT, HC, CH, OH**.

**Confidence:** the tone-ID offset is cross-checked — kits 0–3 all resolve to
sensible tones (808→`808 Bass2`, 909→`909 Bass2`, 707→`707 Bass1/2`, 727→`727
HighBongo/LB` …). Marked `TODO(controlled-diff)` in the crate because it comes
from one v1.51 backup: the *rest* of each 0x34 voice block (level/pan/tune/decay/
…), the exact TONE entry count, and cross-firmware stability are **not** yet
confirmed. Don't trust it for *writing* kits until verified.

#### Voice block (0x34 bytes) — CONFIRMED

Named from **TR Editor's schema** (`Contents/Resources/Script/Script.xml`,
`structType instCommon[0]`) and verified: every schema default matches the
observed byte at the mapped offset (LEVEL 255→`+0x04`, GAIN 81→`+0x05`, PAN
128→`+0x06`, DELAY SEND 224→`+0x08`, LFO DEPTH 128→`+0x0B`). Params are
contiguous single bytes after the u16 tone.

| Off | Field | Type | Range | Default |
| --- | ----- | ---- | ----- | ------- |
| `+0x00` | INST TONE | u16 | 0–1023 | 72 |
| `+0x02` | INST TUNE | u8 | 0–255 (ctr 128) | 128 |
| `+0x03` | INST DECAY | u8 | 0–255 | 128 |
| `+0x04` | INST LEVEL | u8 | 0–255 | 255 |
| `+0x05` | INST GAIN | u8 | 0–161 | 81 |
| `+0x06` | INST PAN | u8 | 0–255 (ctr 128) | 128 |
| `+0x07` | INST REVERB SEND | u8 | 0–255 | 128 |
| `+0x08` | INST DELAY SEND | u8 | 0–255 | 224 |
| `+0x09` | INST LFO SWITCH | u8 | 0–1 | 1 |
| `+0x0A` | INST LFO DEST | u8 | 0–37 | 1 |
| `+0x0B` | INST LFO DEPTH | u8 | 0–255 | 128 |
| `+0x0C` | CATEGORY LOCK | u8 | 0–1 | 0 |
| `+0x0D..0x0F` | RESERVE000–002 | u8 | 0–255 | 0 |
| `+0x10..0x33` | RESERVE100–102 (+tail) | u32… | — | 0 |

Implemented as `VoiceParams` in the crate; `tr-format kit <n>` shows the named
values. (This corrects the earlier statistical guesses — pan is `+0x06`, not
`+0x02`; the "fixed template" `51 80 80 e0…` was just GAIN/PAN/SENDS/LFO at their
defaults.)

**The schema oracle — `Script.xml`.** TR Editor's parameter layout is
data-driven: the resolver (`CKoaValueRef::Associate` → `CKoaExpression`) walks a
schema shipped as `Contents/Resources/Script/Script.xml` (794 KB, ~27.8k lines):
`<struct>`/`<structType>` with `<address>`/`<size>`, and per-param `<name>`
`<range>` `<default>` `<title>`. Section addresses match the backup
(`kit`=`03 00 00 00`, `ptn`=`04 00 00 00`). This XML is the definitive map for
the remaining records (patterns, FX, SYS) — parse it, don't guess.

## User samples — `PCMT` records + the `SMPL` region — SOLVED (layout)

User samples live in two chunks. **`SMPL`** is a *zero-length header* whose
`extra` field declares the reserved PCM region size (`0x330_0000` ≈ 51 MB on the
TR-6S); the raw audio blob follows it, which is why a sample-loaded backup is
tens of MB. **`PCMT`** is a flat table of **1024 × 64-byte** records
(`tonePcm`) — the backup analogue of the device's SysEx `tone.*` PCM region
(`0x40`; see `docs/device-sysex.md`).

**Record layout (64 B), from `Script.xml`'s `tonePcm` struct.** Field widths
follow the type names (`int8x4`→32-bit/4 B, `int4x4`→16-bit/2 B, `int1x7`→7-bit
/1 B); the widths sum to exactly 64, matching the observed stride.

| off | field | type | meaning |
| --- | ----- | ---- | ------- |
| `0x00` | `Address` | int8x4 | PCM offset of the sample (L) in the `SMPL` region |
| `0x04` | `AddressRight` | int8x4 | PCM offset (R), for stereo |
| `0x08` | `Size` | int8x4 | the sample's stored length |
| `0x0C` | `Start` | int8x4 | **playback window start — the slice in-point** |
| `0x10` | `End` | int8x4 | **playback window end — the slice out-point** |
| `0x14` | `EndMax` | int8x4 | full playable length (window upper bound) |
| `0x18` | `SamplingFrequency` | int8x4 | sample rate |
| `0x1C` | `Channel` | int1x7 | mono/stereo (`0..2`) |
| `0x1D` | `Gain` | int1x7 | level (`0..36`) |
| `0x1E` | `Reserve0` | int4x4 | reserved |
| `0x20` | `ToneId0..3` | int8x4×4 | the tone slot(s) that use this record |
| `0x30` | `Reserve1_0..3` | int8x4×4 | reserved |

**Cross-validated independently:** in the device's *wire* form each `int8x4` is
8 bytes (7-bit-safe digits), which places `Channel` at `0x38` — exactly where
`docs/device-sysex.md` documents `tone.channel`. So the field *order* is attested
from two directions (schema + device map), not just one.

**Why this is the slicer's foundation (cowbell-7po).** `Start`/`End` are
independent of `Address`/`Size`: a record plays a *window* of a sample, not the
whole thing. So **several records can share one `Address` with different
`Start`/`End`** — one uploaded break, windowed into N slices, with *zero audio
duplication*. `tr-format`'s `pcm` module exposes this: `Backup::pcm_tone(i)`,
`pcm_tones()`, and the length-preserving `set_pcm_tone_window(i, start, end)` /
`set_pcm_tone_address(i, …)` slice edits. Adding `PCMT` to the recognised tags is
round-trip-verified on the real 56 MB backup (all six chunks enumerated, bytes
unchanged).

**Confidence.** The record *layout* (offsets, widths, order) is schema-attested
and device-cross-validated — high. Not yet confirmed on populated data: the
numeric encoding of the 4-byte `int8x4` fields is taken as little-endian `u32`
(the container's convention for every other multi-byte field), and whether tone
slot maps 1:1 onto record index or sits behind a small preamble — the only
backup on hand has **no user samples** (record 0 reads as non-empty from stale/
preamble bytes). Both want a sample-loaded backup to nail end-to-end; neither
affects the layout above.

## Effects — there is no `FX  ` section

**Status: decoded, inside the kit record.** The container has no FX chunk at
all. The four `FX  ` byte runs the earlier notes counted are every one of them
**unaligned and inside `TONE` entry names** — `Ring FX`, `Flute FX`, `Tube FX`,
`Voice FX` at `0x32A3E1`, `0x32A9CA`, `0x32A9ED`, `0x32ABC2`. Walking the chunk
chain from `0x40` (each chunk is `16 + payload`, then 16 zero bytes of padding to
the next header) accounts for the entire file with `SYS`, `PTN`, `KIT`, `TONE`,
`PCMT`, `SMPL` and leaves no room for another. **All effects state lives in the
`KIT ` record**, which is also what TR Editor's model says (`fm.usrKit[k]` owns
`kitRev`, `kitDly`, `kitMfx*`, `instFx*`).

### The kit-record sub-struct chain — CONFIRMED

`Script.xml`'s `kit` structType lists the sub-structs in record order.
Accumulating their sizes with the [schema offset model](#schema-offset-model--solved)
reproduces every boundary below, once two kit-specific facts are added:

- **`int4x4` with range `0..=65535` is 2 bytes here**, not the 3 that `ptnCmn`
  uses — the same exception `ptnVar00` shows. Forced by the data: `kitCmn`'s 11
  `INST GROUP` masks + `KIT GROUP(S)` must span `+0x22..+0x39`, because the
  eleven `SLIDER COLOR` bytes are at `+0x3A..+0x44` (the only 11-byte window
  whose every value stays in `0..=11` across all 128 kits, with `+0x45..+0x53`
  and `+0x22..+0x39` identically zero, and whose first three read `0,1,3` = the
  schema defaults for BD/SD/LT).
- **Every sub-struct starts at a 4-byte-aligned record offset.** Exactly the two
  whose packed size is not a multiple of 4 get padded — `kitCmn` 67 → 68 and
  `kitMfxShare` 25 → 28 — and every other sub-struct is already a multiple of 4.
  This is what closes the arithmetic; without it the chain misses by 4.

With those the chain lands exactly on the independently-confirmed
`instCommon[0]` offset `+0x194`, which is the check that validates the whole
thing:

| Record | Struct | Size | Contents |
| ------ | ------ | ---- | -------- |
| `+0x10` | `kitCmn` | 68 (67+1 pad) | name, level, mute groups, slider colours |
| `+0x54` | `kitRev` | 40 | **reverb** |
| `+0x7C` | `kitDly` | 52 | **delay** |
| `+0xB0` | `kitMfxCommon` | 20 | **master FX** type + on/off |
| `+0xC4` | `kitMfxShare` | 28 (25+3 pad) | master FX `Ctrl` + `PRM00..23` |
| `+0xE0` | `kitExtIn` | 40 | ext-input gain/pan + **reverb/delay sends** |
| `+0x108` | `kitLfo` | 36 | kit LFO |
| `+0x12C` | `kitCtrl` | 80 | CTRL assignments |
| `+0x17C` | `kitOut` | 12 | per-instrument output routing |
| `+0x188` | `kitRef` | 12 | kit references |
| `+0x194` | `instCommon[0]` | 11 × 52 | the voice blocks (`instCommon`+`instShare`) |
| `+0x3D0` | `instFxCommon[0]` | 11 × 32 | **per-instrument insert FX** |

Each non-FX boundary corroborates independently on the backup: `kitLfo` reads
`WAVEFORM 0 / RATE 128 / TEMPO SYNC 1` — its three schema defaults — at
`+0x108..+0x10A`; `kitCtrl` reads `Select` then 44 bytes that never exceed `35`
(its schema max) in four visibly regular 11-byte blocks; `kitOut`'s 12 bytes are
all `0` (= MIX). There are **11** instrument blocks, not 6: a TR-6S backup still
carries the TR-8S layout.

### Reverb (`kitRev`, `+0x54`) — CONFIRMED

| Off | Field | Range | Default | Modal byte on the backup |
| --- | ----- | ----- | ------- | ------------------------ |
| `+0x54` | REVERB TYPE | 0–6 | 2 (`HALL1`) | 2 ×114 |
| `+0x55` | REVERB TIME | 0–255 | 150 | 150 ×114 |
| `+0x56` | REVERB LEVEL | 0–255 | 0 | 0 ×96 |
| `+0x57` | REVERB PRE DELAY | 0–100 | 20 | 20 ×123 |
| `+0x58` | REVERB LOW CUT | 0–17 | 2 | 2 ×119 |
| `+0x59` | REVERB HIGH CUT | 0–14 | 11 | 11 ×126 |
| `+0x5A` | REVERB DENSITY | 0–10 | 10 | 10 ×127 |

Types: `AMBI, ROOM, HALL1, HALL2, PLATE, MOD, HA-DOU`. `+0x5B..+0x7B` is reserve
and reads zero in all 128 kits.

### Delay (`kitDly`, `+0x7C`) — CONFIRMED (21 of 23)

23 single bytes at `+0x7C..+0x92`, then 29 bytes of reserve. Every one is inside
its schema range across all 128 kits, and the schema default is the modal byte
for each: `TIME` 104 ×66, `FEEDBACK` 120 ×101, `HIGH CUT` 7 ×118, `HIGH DAMP F`
13 ×125, `TAP TIME` 50 ×124, `ECHO MODE` 1 ×126, `ECHO BASS`/`ECHO TREBLE` 15 and
`ECHO TAPE DIST` 4 in **all 128**, the three `ECHO PAN`s and the two `W/F`s 128
×126–127. Several observations touch a range maximum exactly — `DELAY HIGH CUT`
14 = max, `DELAY LOW DAMP` 81 = max, `ECHO MODE` 6 = max — which a drifted
offset would not produce.

Order: `DELAY TYPE, TEMPO SYNC, LEVEL, TIME, FEEDBACK, HIGH CUT, HIGH DAMP,
HIGH DAMP F, LOW DAMP, LOW DAMP F, TAP TIME, ECHO MODE, ECHO BASS, ECHO TREBLE,
ECHO PAN S/M/L, ECHO TAPE DIST, ECHO W/F RATE, ECHO W/F DEPTH, DELAY RVB SEND,
PITCH COARSE, PITCH FINE`. Types: `DLY, PAN, TAPE ECHO, PITCH SHFT`; echo modes
`S, M, L, S+M, S+L, M+L, S+M+L`.

**The two exceptions are explained, not anomalies.** `PITCH COARSE` (201–237)
and `PITCH FINE` (1–201) read `0` in every kit — below their ranges. They belong
to `DELAY TYPE 3` (`PITCH SHFT`), and no factory kit selects it: the observed
type histogram is `DLY ×114, PAN ×9, TAPE ECHO ×5`, max value 2. Uninitialised,
so they are exposed raw and not interpreted.

### The shared `PRM` pool — CONFIRMED

Master FX and per-instrument insert FX both store their parameters in a generic
`Ctrl` byte + `PRM00..PRM23`. What those bytes mean depends on the type field.
The map is `Script.xml`'s **`alt` structType**, which lists one overlay struct
per type index — `kitMfxComp, kitMfxDrv, kitMfxOd, …` and `instFxComp,
instFxDrv, instFxCr, …` — in the same order as TR Editor's `mfxType` /
`instFxType` name tables. Each overlay's first value is its own `Ctrl`, so
**overlay value `n+1` is `PRMn`**.

TR Editor's own EFX panel corroborates that directly rather than by inference:
the panel opened for `Type 10..12` binds `PRM00`→Depth, `PRM01`→Resonance,
`PRM02`→filter Type (`mfxFltType` combo), `PRM03`→Gain (max 80, offset −40, dB),
`PRM04`→Clipper — exactly `kitMfxFlt` minus its `Ctrl`.

**The decisive test.** Decode each kit's pool through the overlay its own type
field selects, then check every byte against that parameter's schema range:

| Table | Values checked | Out of range | Types exercised |
| ----- | -------------- | ------------ | --------------- |
| master FX (`kitMfxShare`) | 779 | **0** | 12 of 21 |
| insert FX (`instFxShare`) | 6,157 | **0** | **17 of 17** |

The test discriminates. Shifting the base by ±1/±2/±4 puts 2.1–26.5 % of values
out of range; using a wrong insert-FX stride (28/30/31/33/34/36) puts 5.8–7.1 %
out. (One degenerate alternative also scores 0: reading the insert-FX Type
column 8 bytes early lands in the previous block's zero tail, so every type reads
`0` with all-zero params. It is excluded by the type histogram — the real column
shows 17 distinct types with the schema default 12 = `THRU` modal.)

### Master FX (`kitMfxCommon` `+0xB0`, `kitMfxShare` `+0xC4`)

| Off | Field | Range | Default | Observed |
| --- | ----- | ----- | ------- | -------- |
| `+0xB0` | `Type` | 0–20 | 11 (`HPF`) | 12 distinct values, all ≤ 17 |
| `+0xB1` | `Sw` | 0–1 | 0 | strictly `{0,1}`; on in 13 of 128 kits |
| `+0xB2..+0xC3` | reserve | — | 0 | zero in all 128 |
| `+0xC4` | `Ctrl` | 0–15 | 0 | `{0,1,2}` |
| `+0xC5..+0xDC` | `PRM00..PRM23` | — | — | see above |
| `+0xDD..+0xDF` | alignment pad | — | 0 | zero in all 128 |

The 21 types, in index order: `COMPRESSOR, DRIVE, OVERDRIVE, DISTORTION, FUZZ,
CRUSHER, PHASER, FLANGER, TRANSIENT, TRANSIENT2, LPF, HPF, LPF/HPF, L BOOST,
H BOOST, L/H BOOST, ISOLATOR, SBF, NOISE, FATTENER, VINYL SIM`.

**Types 19 (`FATTENER`) and 20 (`VINYL SIM`) have no overlay struct** in this
`Script.xml`, so their `PRM` names are **inferred** — taken from TR Editor's
`mfxCtrl` CTRL-target table (`Depth, Level` / `Compressor, Noise, Wow Flut`) with
**unknown ranges**. No factory kit uses either type, so there is nothing to check
them against. The crate flags them with `FxTypeInfo::params_confirmed == false`
and skips them in range checks rather than reporting a guess as a reading.

### Per-instrument insert FX (`instFxCommon`/`instFxShare`, `+0x3D0`)

Eleven blocks at `+0x3D0 + slot × 0x20` (`instFxCommon` 4 B + `instFxShare` 25 B
padded to 28). Block layout: `Type`, 3 reserve bytes, `Ctrl`, `PRM00..PRM23`,
3 pad. Located empirically, not just derived: there are exactly **11** offsets in
`+0x3B0..+0x520` whose next three bytes are zero in all 128 kits and whose own
value stays within `instFxCommon.Type`'s `0..=16`, and they are `+0x3D0` plus
multiples of 32.

The 17 types, in index order: `COMPRESSOR, DRIVE, CRUSHER, COMP+DRV, TRANSIENT,
LPF, HPF, LPF/HPF, L BOOST, H BOOST, L/H BOOST, ISOLATOR, THRU, SATURATOR,
FREQ SHIFT, RING MOD, SPREAD`. `THRU` (12) is the schema default and a true
bypass — its overlay holds nothing but the shared `Ctrl`. Observed across
128 × 11 slots: `THRU ×660, L/H BOOST ×346, COMP+DRV ×129, HPF ×90, LPF/HPF ×75`,
tailing off through all 17.

**The 11th block does not fit — UNRESOLVED.** Slot 10's block starts at `+0x510`
and the kit record is `0x520` long, so only `Type`, the reserves, `Ctrl` and
`PRM00..PRM10` are stored; `PRM11..PRM23` and the pad are 16 bytes past the end.
The layout wants `0x530` and Roland's record is `0x520`. This is not a
mis-derivation — the 11 type columns are individually verified above, the inst
region ends exactly at `+0x3D0` (`0x194 + 11 × 52`), and slot 10 does carry live
types (`COMP+DRV ×5`, which needs 18 `PRM`s). Why the record is 16 bytes short of
its own schema is an open question; settling it wants a **TR-8S** backup (where
slot 10 = `RC` is a real voice) or hardware. The crate reports it rather than
hiding it: `InstFxParams::prm_available` is 24 for slots 0–9 and 11 for slot 10,
`is_truncated()` says whether the selected type needs the missing bytes, and
`named_params()` returns only what is actually stored.

### External-input sends (`kitExtIn`, `+0xE0`) — CONFIRMED

`SideChainSrc, SideChainType, SideChainDpt, Gain, Pan, ReverbSend, DelaySend` at
`+0xE0..+0xE6`. Anchored by `Gain` at `+0xE3` — 25 distinct values spanning
81–109, its schema default 81 in 66 kits, inside range 0–161 — and `Pan` at
`+0xE4`, which is its default 128 in **all 128** kits.

### Reading it back

The decode reads as music, which is the last check:

- `Lofi HipHop` → `ROOM` reverb at level 99, `TAPE ECHO` delay at level 189,
  `L BOOST`/`TRANSIENT`/`COMP+DRV` inserts across the voices.
- `TR-808_Kit` → `HALL1` at level 0 (reverb effectively off), plain `DLY`,
  `COMP+DRV` on BD/SD/LT and `HPF` on the hats.
- `TR-626_Kit` → `L/H BOOST` on every one of its six voices.

Implemented as `tr_format::fx`: `ReverbParams`, `DelayParams`, `ExtInFx`,
`MfxParams`, `InstFxParams`, the `MFX_TYPES` / `INST_FX_TYPES` parameter tables,
and `Kit::reverb()` / `delay()` / `mfx()` / `ext_in_fx()` / `inst_fx()`.

## TR Editor data model (oracle)

Roland's official **TR Editor** (Mac/Win) is a **JUCE C++ app** whose binary
keeps its mangled C++ symbols and a full parameter-path table — a definitive map
of the device data model. (Analyzed locally for interoperability; the app is not
committed.) The parameter tree maps 1:1 onto the backup sections:

| Model path | Backup section |
| ---------- | -------------- |
| `fm.usrKit[k].kitCmn.NAMEA` | KIT record name (`+0x10`) — confirmed |
| `fm.usrKit[k].instCommon[i]` | **the per-voice block** (6 insts/kit) |
| `fm.usrKit[k].instShare[i]`, `instFxCommon[i]`, `instFxShare[i]`, `kitMfxShare` | other kit sub-structures |
| `fm.usrPtn[p].ptnCmn` (`NAMEA`, `KIT REFFERENCE`), `.ptnVar[v]` | PTN record + variations (the 1/2/S) |
| `fm.tone[t].toneCmn` (`NAMEA`, `Category`, `Type`, `LOOP`) | TONE table |
| `fm.PCM_TONE[t].tonePcm` (`Address`, `Channel`, `Size`) | SMPL / sample refs |
| `fm.SYS.*` | SYS chunk |

**Instrument (voice) param vocabulary** (`inst*` — the leaves of `instCommon`):
`instToneValueRef` (**= our `+0x00` tone-ID, confirmed by name**), `level`,
`instOutput`, `instGroupCombo` (mute group), `instFilterValueRef`,
`instCtrlKnob` / `instCtrlSelect` / `instCtrlCombo` / `instCtrlPrm` (the
assignable **CTRL** knobs — tone-dependent), `instLastStepSwValueRef`,
`instSelectValueRef`.

Note: there is **no fixed pan/tune/decay** — beyond Tone/Level, instrument
tweaking is via the assignable CTRL knobs. This supersedes the earlier
"pan/tune" guesses in the voice-block map: `+0x02`/`+0x03` (bipolar, center
`0x80`) are more likely CTRL-knob values, `+0x04` a level/output field. The
definitive **name→byte-offset** mapping needs decompiling TR Editor's backup
serializer / param table (the imported `TREditor_x86_64` in the Ghidra project).

**SysEx read/write:** TR Editor implements `FKoaSendRq1` / `FKoaSendDt1` /
`FKoaRequestRq1Dt1` — i.e. Roland **RQ1 (data request) / DT1 (data set)**. RQ1
is a **read primitive over USB**; decompiling it yields the device address map
and could enable a no-hardware read of device memory (see the software-surface
avenue in `docs/prior-art.md`).

## Pattern record (`PTN ` section)

128 records of **`0x5FB8` (24,504) bytes**. Same framing as kits: 16-byte record
header, then the schema data (`ptnCmn` header, then `ptnVar*` step/motion).

Header fields **confirmed** against `Script.xml` `ptnCmn` + the manifest (name,
tempo, kit all match for every pattern):

| Off | Field | Type | Notes |
| --- | ----- | ---- | ----- |
| `+0x10` | NAMEA | string×16 | pattern name |
| `+0x20` | TEMPO | u16 LE | **BPM × 10** (1440 → 144.0) |
| `+0x22` | KIT REFFERENCE | u8 | kit slot 1–128 |

Implemented as `Pattern` (`name`/`tempo_bpm`/`kit_ref`); `tr-format patterns`
lists all 128 with tempo + kit — the basis for browsing / performance
management.

### Schema offset model — SOLVED

Backup record offset = **`0x0F` + schema offset** (i.e. field data follows the
16-byte record header). Per-field byte size from the `Script.xml` `<value>`
`<type>`:

| Type | Bytes |
| ---- | ----- |
| `int1x7`, `int2x4` | 1 |
| `int2x7` | 2 |
| `int8x4` | 4 |
| `int4x4` | **⌈bits(range_max) / 7⌉** (7-bit-safe): 0–1023→2, 0–3000→2, 0–65535→3 |
| `stringNx7` | N (16 for names) |

The `int4x4` rule was the missing piece: TONE/TEMPO (≤12 bits) are 2 bytes, but
16-bit masks like `SHUFFLE SWITCH` are 3. Validated: **102/103 `ptnCmn` fields
land in range** (the one miss is `MASTER PROBABILITY` reading 0, an off value
below its 1–201 UI range — not a drift). Encoded as `schema_value_size()` in the
crate. Independently re-checked while decoding the step word: derived this way,
`FLAM SPACING` (schema `+0x48` → record `+0x58`) reads its default `1` for 120 of
128 patterns and its non-default values correlate with flam usage, which would
not happen if the accumulated offset had drifted.

**Known limit of the rule.** In `ptnVar00` the two `int4x4` accent masks
(`0,65535`) occupy **2 bytes each**, not the 3 the rule gives — the accent header
is 4 bytes, which the variation stride confirms exactly. So `int4x4` sizing is
not uniform across structs; `ptnCmn` is empirically validated, `ptnVar00` is the
counter-example. Treat the rule as validated per-struct, not universal.

### Pattern body structure

`ptn` = `ptnCmn` (header, ~145 B) + **10 variations** (A–H + 2 fills; the schema
lists them at addresses `01`–`0A`). Each variation holds per-instrument step
arrays: `ptnVar01` = `INST01 PTN00…PTN15` — **16 steps as `int8x4` step words**.

**Step word (4 bytes) — DECODED.** See [the step word](#the-step-word-4-bytes--decoded)
below. Verified: variation A INST01 of "Speak C0DE" reads `. X . X X X . X …` at
record `+0xA0` — a real drum pattern. Exposed as `StepWord` in the crate.

**Stride — NAILED (empirically verified).** Variation 0 (A) begins at record
`+0xA0`; consecutive variations are a steady **`0x984` (2436 bytes)** apart
(A–H boundaries confirmed by locating each variation's step cluster). Per
variation: `ptnVar00` accent (4 B) + 25 step-arrays (`ptnVar01…25`, 64 B each =
16 steps × 4-B word) + motion (`ptnVar26`, 832 B) = 2436. So:

```
step_word_offset = 0xA0 + variation*0x984 + 4 + track*64 + step*4
```

Verified against the backup — reading this formula yields real beats (variation A
track 0 of "Speak C0DE": `X.XXX.X.`). Exposed low-level in the crate as
`Pattern::step_word()` + the `PATTERN_*` stride constants.

### The 25 array slots — SOLVED

`Script.xml` names every field of `ptnVar01`…`ptnVar26`, and the reference
backup corroborates the split exactly (the motion slots of unused voices are
zero; the step slots are the ones that read as music):

| Slot (0-based) | Schema | Holds |
| -------------- | ------ | ----- |
| — | `ptnVar00` | accent masks (4 B: `ACCENT PTN`, `ACCENT PTN_WEAK`) |
| `0`–`10` | `ptnVar01`–`ptnVar11` | **step words**, `INST01`–`INST11 PTNnn` |
| `11` | `ptnVar12` | **step words**, `TRIG PTNnn` (trigger out) |
| `12`–`22` | `ptnVar13`–`ptnVar23` | per-step **motion**, `INST01`–`INST11 PRMnn` |
| `23`–`24` | `ptnVar24`–`ptnVar25` | motion planes `OTH0`/`OTH1 PRMnn` |
| — | `ptnVar26` | 832 B of `RESERVE00`… (208 × `int8x4`) |

So `4 + 25×64 + 832 = 2436 = 0x984` is fully accounted for. Note the tail is
**reserve, not motion** — motion lives in slots 12–24 (this corrects the earlier
"motion(832)" reading of `ptnVar26`).

The 11 instrument tracks are the **TR-8S** panel layout, from TR Editor's
`editor_pattern_inst` panels: `BD SD LT MT HT RS HC CH OH CC RC`. A **TR-6S**
stores its six voices in slots 0–5 and leaves 6–10 empty, so on a TR-6S backup
slots 3/4/5 are its **HC/CH/OH**, not MT/HT/RS. Confirmed on the reference
backup: slots 6–10 are all zero, slot 11 (TRIG) is on at velocity 80 for all
20,480 steps, and slot 4 — the TR-6S closed hat — is by far the busiest track.
Exposed as `track_role()` / `TrackRole` / `INST_TRACKS` in the crate.

### The step word (4 bytes) — DECODED

| Byte | Bits | Field |
| ---- | ---- | ----- |
| `0` | 0–7 | **velocity**, 1–127 (`0` = step off; `80` = the default) |
| `1` | 0–2 | **sub step** — `0` none, `1` FLAM, `2` `1/2`, `3` `1/3`, `4` `1/4` |
| `1` | 3–6 | unknown (always 0 here) |
| `1` | 7 | **ALTERNATE** flag |
| `2`–`3` | — | unknown (always 0 here) |

Across all 512,000 step words in the backup, the six TR-6S voice tracks use
exactly **eight** byte-1 values — `00 01 02 03 04 80 81 82` — and bytes 2–3 are
zero in every one of the 16,987 on-steps. So the low field is `0..4` and bit 7 is
an independent flag.

**Why the low field is a hit count, not the combo index.** TR Editor's
`subStep` string table is `1/2,1/3,1/4,FLAM` (index 0–3), which would put FLAM at
`4`. It's the other way round — the device stores the **number of hits**, with
FLAM as the `1` special case. Evidence:

- `ptnCmn` has a per-pattern **`FLAM SPACING`** (range 0–8, default 1). Only 8 of
  the 128 patterns set it away from the default — and 3 of those 8 use low value
  `1`, versus 4 of the other 120 (a ~11× enrichment, hypergeometric *p* ≈ 7e-4).
  Values `2`/`3`/`4` show no enrichment at all (1/8, 0/8, 0/8).
- Low value `1` lands on the **low tom** 31 times out of 34 — tom flams.
- Value `2` is the most common by 6× (652 uses), is hat-heavy (331 on CH), and is
  the only value that runs for a whole bar (10 runs of 16) — a `1/2` double is
  the one subdivision you can use pervasively. Values `3`/`4` never run past 3
  steps and skew to the last quarter of the bar (38–40%), i.e. fills.

Reading it back confirms the musical sense: `707_Variation` variation D shows
`LT |.... .F.. F...|` (707 tom flams, and this is one of the non-default
`FLAM SPACING` patterns), and `Footwerk` shows `LT |X.X. X.X. X.X. X.22|` — a
juke tom line ending in doubles.

**Bit 7 = ALTERNATE.** TR Editor's step editor exposes exactly four per-step
attributes — velocity, probability, sub step, alternate — so one boolean is
unaccounted for, and bit 7 is the only boolean left. It concentrates in the
percussion-pair presets: `727_&_909` and `727_Variation_1/2` account for 572 of
the 754 alternate steps, and in `727_&_909` the whole 16-step HC row is flagged —
exactly the high/low conga-bongo alternation a 727 kit is for.

**Probability is not in this data.** Per-step probability (`0`–`10`, displayed
`---,90,…,0`) reads 0 for every step of every factory pattern, as does
`ptnCmn.MASTER PROBABILITY`, so its bit position is **unconfirmed** — it is
presumably one of the zero bits (byte 1 bits 3–6, or bytes 2–3) and looks like a
later-firmware feature. `StepWord`'s setters preserve every undecoded bit, and
`StepWord::unknown_bits()` reports them, so an edit can never silently drop
per-step data this crate doesn't understand yet.

### Motion (`PRM`) arrays — slots 12–24

The same 4-byte word, different meaning. `Script.xml`'s **`motionPrm` dataTable**
is the map: each entry has an `<order>` field, which is the **byte index of that
parameter within the PRM word**.

| Byte | INST slots 12–22 | `OTH0` (slot 23) | `OTH1` (slot 24) |
| ---- | ---------------- | ---------------- | ---------------- |
| `0` | TUNE (centre 128) | DELAY FEEDBACK | REVERB LEVEL |
| `1` | DECAY | DELAY LEVEL | MFX SW (0/1) |
| `2` | CTRL (centre 128) | DELAY TIME | MFX DEPTH |
| `3` | flags — see below | flags | flags |

`VELOCITY` and `PROBABILITY` are in that same table with **`order -1`**: they are
*not* in the PRM arrays, they live in the step word. That independently confirms
the step-word decode above, and says per-step probability is a step-word field.

**Which OTH array is which is forced, not guessed.** The delay's three
parameters use orders 0/1/2, so delay must occupy an array by itself; reverb
(order 0) and MFX (orders 1, 2) fill the other without collision. The empirical
tiebreak: `MFX SW` has `<max>1</max>`, and slot 24 byte 1 is the only lane in the
whole backup that is strictly `{0, 1}`. So slot 24 = reverb + MFX, slot 23 =
delay. Reading it back: `[TR]ntablist` variation A shows `MFX SW |1 - - …|` with
`MFX DEPTH` values beside it.

**Ground truth check.** The manifest marks 54 patterns `Motion ON`. Every one of
them has data in slots 12–24, and 73 of the 74 unmarked patterns have none — a
**127/128** match. The single exception (`DnB-FM`) has exactly one stale word, in
a fill variation, on a pattern whose motion switch is off.

**The flags byte (byte 3)** is never zero on a live motion word, because a lane
value of `0` is a legal parameter value and needs a bit to distinguish it from
"nothing recorded here". Confirmed across all 8,933 live motion words, with
**zero** violations:

- **bit 7 ⟺ lane 0 records a value** (100%: every non-zero lane 0 has it set)
- **bit 6 ⟺ lane 1 records a value** (100%)

**Lane 2's flag is unresolved.** No single bit implies it across slots — bit 1
fits the INST slots at 90–100% but not exactly, bit 0 fits slot 23 at 100%, and
slot 24 matches nothing cleanly. The low six bits are only 63–90% constant across
a track, so they are neither a clean per-step lane flag nor a pure per-track
selector. `MotionWord::lane_recorded(2)` returns `None` (undetermined) rather
than guessing, and `MotionWord::lane(2)` falls back to "non-zero means recorded"
— which under-reports a genuine recorded `0`. Settling this needs a controlled
diff (record motion on one parameter, save, diff), i.e. hardware.

Exposed as `MotionWord` / `motion_lane_name()` / `Pattern::motion_word()` in
`tr-format` and `MotionLanes` + `tr-studio motion <backup> <n> <var>` above it.

The effects those `OTH0`/`OTH1` lanes automate live in the **kit** record, not in
a section of their own — see "Effects" below.

## System record (`SYS ` section)

**Status: decoded.** Named from TR Editor's `Script.xml` (model path `fm.SYS.*`
→ `structType sys`, whose four sub-structs are `sysGeneral`, `sysCategory`,
`sysSound`, `sysMidi`) and checked against the reference v1.51 backup **and**
the v2.00 firmware's factory image (see the `init_param` reconciliation below).

### Framing — CONFIRMED

`SYS ` is an ordinary array section holding **one** record; it does **not** skip
the 16-byte record header.

```text
payload+0x00  u32 count       = 1
payload+0x04  u32 record_size = 0x2F0 (752)      <- the whole payload
payload+0x08  8 B  section token  d8 0c 34 03 1c 2a a9 63
payload+0x10  736 B parameter body
```

Evidence: the declared `record_size` equals the declared payload size, and the
body decodes correctly *only* from `payload+0x10` — the `int1x7` field
`LCD Contrast` (schema offset 0) reads its default `4` there and every following
field falls into place (below). At `payload+0x08` or `+0x0C` nothing lines up.

### Body map — CONFIRMED

Sub-structs are concatenated in schema order, each sized by accumulating
`schema_value_size()` over its `<value>` list. The four accumulated sizes
predict three boundaries, and **all three land on an independently visible
landmark in the data**:

| Body range | Sub-struct | Size | Boundary evidence |
| ---------- | ---------- | ---- | ----------------- |
| `0x000`–`0x05B` | `sysGeneral` | 92 | end = first byte of `USER01` |
| `0x05C`–`0x25B` | `sysCategory` | 512 | 32 × 16; end = last byte of `USER32` |
| `0x25C`–`0x2A7` | `sysSound` | 76 | end = start of `sysMidi` (next row) |
| `0x2A8`–`0x2CD` | `sysMidi` (named) | 38 | `Pattern Ch` = 9 and `RX FA FC` = 1, its two non-zero defaults, both land |
| `0x2CE`–`0x2DF` | `sysMidi` reserve tail | 18 | all zero |

`92 + 512 + 76 + 38 + 18 = 736` — the body is fully accounted for.

(At `payload+0x08` `LCD Contrast` would read `0xD8` and at `+0x0C` `0x1C`, both
far outside its `0,9` range — the body base is not merely plausible at `+0x10`,
the alternatives are excluded.)

**Aggregate check:** of the **120** named fields, the **88** that carry a
numeric `<range>` are **all in range** (zero violations), and **68** sit exactly
on their `Script.xml` `<default>`. Nothing here is a statistical guess: the
map is derived, then every derived offset is checked.

### `sysGeneral` (body `0x000`) — CONFIRMED

41 named fields in 42 bytes; 40 read their schema default.

| Off | Field | Type | Range | Def | Observed |
| --- | ----- | ---- | ----- | --- | -------- |
| `+0x00` | LCD Contrast | u8 | 0–9 | 4 | 4 |
| `+0x01` | LED Bright | u8 | 0–9 | 7 | 7 |
| `+0x02` | LED Off Bright | u8 | 0–9 | 2 | 2 |
| `+0x03` | SliderLED | u8 | 0–1 | 0 | 0 |
| `+0x04` | SliderColorSource | u8 | 0–1 | 0 | 0 |
| `+0x05` | Auto Off | u8 | 0–2 | 0 | 0 |
| `+0x06` | Knob Mode | u8 | 0–1 | 0 | 0 |
| `+0x07` | WeakBeat | u8 | 0–1 | 0 | 0 |
| `+0x08` | LED Demo | u8 | 0–10 | 5 | 5 |
| `+0x09` | Auto Save | u8 | 0–1 | 0 | 0 |
| `+0x0A` | TempoSrc | u8 | 0–1 | 0 | 0 |
| `+0x0B` | TempoSync | u8 | 0–3 | 0 | 0 |
| `+0x0C` | **Tempo** | u16 LE | 400–3000 | 1250 | **1250** |
| `+0x0E` | Sync Out | u8 | 0–1 | 1 | 1 |
| `+0x0F` | Shuffle | u8 | 0–1 | 0 | 0 |
| `+0x10` | SEQ Mode | u8 | 0–1 | 0 | 0 |
| `+0x11` | ManualMode | u8 | 0–2 | 1 | 1 |
| `+0x12` | KitSelect | u8 | 0–1 | 1 | 1 |
| `+0x13` | M.Trig | u8 | 0–1 | 1 | 1 |
| `+0x14` | USB Mode | u8 | 0–1 | 0 | 0 |
| `+0x15` | USB Audio | u8 | 0–1 | 0 | **1** |
| `+0x16` | SCAT TRIG | u8 | 0–2 | 0 | 0 |
| `+0x17` | HH Link | u8 | 0–1 | 0 | 0 |
| `+0x18` | Start Ptn | u8 | 0–128 | 1 | 1 |
| `+0x19` | Start Kit | u8 | 0–128 | 1 | 1 |
| `+0x1A` | Last Ptn | u8 | 0–127 | 0 | 0 |
| `+0x1B` | Last Kit | u8 | 0–127 | 0 | 0 |
| `+0x1C` | Ptn Lock | u8 | 0–1 | 0 | 0 |
| `+0x1D`–`+0x27` | **Slider Color** BD SD LT MT HT RS HC CH OH CC RC | u8×11 | 0–11 | 0…10 | 0…10 |
| `+0x28` | Inst Pad | u8 | 0–3 | 1 | 1 |
| `+0x29` | Trig Adjust | u8 | 0–12 | 0 | 0 |
| `+0x2A`–`+0x5B` | RESERVE100/101 + RESERVE200–211 | — | — | 0 | `07 01`, then `128, 0×11` |

Two independent offset anchors, either of which alone would pin the block:

- **`Tempo` = 1250** — a 12-bit value reading its *exact* non-trivial default as
  an LE `u16` at the two bytes the `int4x4` sizing rule predicts. This also
  re-validates that rule (`ceil(bits(3000)/7)` = 2) on a struct other than
  `ptnCmn`. Tempo is stored as **BPM × 10**, same as `ptnCmn.TEMPO`.
- **The `Slider Color` ramp `0,1,2,…,10`** — 11 consecutive bytes counting up,
  exactly the schema's per-instrument defaults, in `INST_TRACKS` panel order.
  A one-byte drift would break it.

`RESERVE100`/`RESERVE101`/`RESERVE200` are non-zero (`7`, `1`, `128`).
**Unknown** what they hold — presumably TR-6S/firmware fields that postdate
TR Editor's schema. They are *not* a sign of drift: they sit after every named
field, and the named fields all validate.

### `sysCategory` (body `0x05C`) — CONFIRMED

32 × 16-byte user category names, space-padded; factory content `USER01`…
`USER32`. The schema entry is the `stringNx7` **`CATEG_NAMEnnA`**, whose length
comes from its `<size>10</size>` — read as **hexadecimal**, i.e. 16 bytes. Two
independent confirmations: consecutive `<address>` values step by `0x10`, and
`kitCmn.NAMEA`, `ptnCmn.NAMEA` and `toneCmn.NAMEA` all carry the same
`<size>10</size>` while being *known* 16-byte fields. So the `N` in `stringNx7`
is `<size>` parsed as hex — a refinement of `schema_value_size()`'s current
"assume 16".

Note the same alias trap as `ptnCmn.NAME`/`NAMEA`: each name also appears as an
`int1x7` **`CATEG_NAMEnn`** carrying *sixteen* `<address>` children (one per
character). A `<value>` with multiple `<address>` children is a per-element
alias — **skip it** when accumulating offsets, or every subsequent field shifts.

### `sysSound` (body `0x25C`) — CONFIRMED

| Off | Field | Range | Def | Observed |
| --- | ----- | ----- | --- | -------- |
| `+0x00` | Local Sw | 0–2 | 1 | 1 |
| `+0x01` | Mix Out | 0–1 | 1 | 0 |
| `+0x02`–`+0x07` | Assign 1…Assign 6 | 0–2 | 1 | 0 |
| `+0x08` | ExtInMode | 0–1 | 0 | 0 |
| `+0x09`–`+0x4B` | RESERVE000–002, RESERVE100–115 | — | 0 | 0 |

Seven of the nine differ from TR Editor's default. That is expected rather than
alarming: `Assign 1`–`6` are the TR-8S's **individual output** routings, which a
TR-6S has no jacks for, and these are the *device's* factory values (the same
bytes ship in the TR-6S firmware image — see below), not the editor's. Offset
confirmation for this block comes from its two boundaries, not from its values.

### `sysMidi` (body `0x2A8`) — CONFIRMED

| Off | Field | Range | Def | Observed |
| --- | ----- | ----- | --- | -------- |
| `+0x00` | Device ID | 0–15 | 0 | 0 |
| `+0x01` | Omni Mode | 0–1 | 0 | 0 |
| `+0x02` | **Pattern Ch** | 0–15 | 9 | **9** |
| `+0x03` | Kit Ch | 0–15 | 0 | 0 |
| `+0x04`–`+0x1A` | **Inst Note00…22** | 0–128 | 128 | see below |
| `+0x1B` | USB MIDI Thru | 0–1 | 1 | 1 |
| `+0x1C` | Soft Thru | 0–1 | 1 | 1 |
| `+0x1D`–`+0x23` | TX Prog Chg / Bank Sel / Edit Data / Nudge / Shuffle, RX Prog Chg / Bank Sel | 0–1 | 0 | 0 |
| `+0x24` | RX Edit Data | 0–1 | 0 | 0 |
| `+0x25` | **RX FA FC** | 0–1 | 1 | **1** |
| `+0x26`… | RESERVE0, RESERVE100… | — | 0 | 0 (truncated) |

The block is bracketed by its two non-zero defaults: `Pattern Ch` = 9 (MIDI
channel 10, the GM drum channel) at `+0x02`, and `RX FA FC` = 1 at `+0x25` after
exactly eight zero switch bytes. Both land, so the 23-byte note array between
them is correctly sized.

**The note map.** The 23 `Inst NoteNN` slots read:

```
slot  00 01 02 03 04 05 06 07 08 09 10 | 11 12 13 14 15 16 17 18 19 20 21 22
note  36 38 43 39 42 46 -- -- -- -- -- | 35 40 41 54 44 55 -- -- -- -- -- --
```

Slots 0–10 = the 11 instruments in `INST_TRACKS` order is **confirmed**: on a
TR-6S the first six carry the GM drum notes of exactly its six voices
(36 BD, 38 SD, 43 low tom, 39 hand clap, 42 closed HH, 46 open HH) and the five
TR-8S-only voices read `128` = unassigned.

Slots 11–21 being the same instruments' **alternate**-tone notes is
**inferred**, not confirmed: the schema names them only `Inst Note11`… . The
inference is that slots 11–16 hold `35 40 41 54 44 55` — the GM "second" sound
of each of the same six voices (35 acoustic BD, 40 electric snare, 41 low floor
tom, 54 tambourine, 44 pedal HH, 55 splash) — with 17–21 unassigned, and it
dovetails with the step word's ALTERNATE flag. **Slot 22 is unaccounted for.**
Settling both needs a controlled hardware diff (change one ALT note, save,
diff). `SysMidi::inst_note_alt()` is marked inferred in the crate.

### The truncated tail — partly UNKNOWN

Body `0x2CE`–`0x2DF` (18 bytes) is zero. The schema declares `sysMidi`'s
reserve tail as `RESERVE0` (`int4x4`) + `RESERVE100`–`RESERVE107` (8 ×
`int8x4`) = 34–35 bytes, but the 752-byte record only provides 18. **Observed:**
the record stops mid-reserve. **Unknown:** whether the firmware truncates the
struct, or the schema over-declares for a model/version other than this one.
It affects nothing decodable — the bytes are zero in both corpora — but a writer
must not assume the schema's declared length. Exposed as `Sys::reserve_tail()`.

Implemented as `sys::Sys` / `SysGeneral` / `SysSound` / `SysMidi` +
`Backup::sys()` in `crates/tr-format/src/sys.rs`.

## Reconciling `SYS ` with the firmware `init_param`

The prior note that the `SYS ` payload has "the same body as the firmware
`init_param`" turns out to understate it, and the mechanism was not what it
looked like.

### `dd001_init_param.bin` is LZSS-compressed — CONFIRMED

Searching the firmware blob for the `SYS ` payload bytes finds **nothing**: the
data is there, but literal runs are broken up by short control sequences. It is
**classic LZSS** (Okumura's `lzss.c`, the ubiquitous embedded variant):

| Parameter | Value |
| --------- | ----- |
| Ring buffer | 4096 bytes, pre-filled `0x00` |
| Initial write position | `4096 − 18` = `0xFEE` |
| Control byte | 1 flag bit per token, **LSB first**; `1` = literal, `0` = match |
| Match token | 2 bytes: `offset` = `b0` OR'd with `(b1 & 0xF0) << 4` (12 bits); `length` = `(b1 & 0x0F) + 3` |

Decoding a `0x6B858`-byte section from file offset `0x30` consumes it exactly
and yields **exactly `0x33FFD0` bytes** — the value the `init_param` header
declares. (The three sections are byte-identical *compressed*, so decoding all
three is one confirmation, not three.) Three checks, none of which a wrong codec
survives:

1. The decoder consumes the compressed section to the last byte and lands on the
   declared output length to the byte.
2. The output starts with the `TR6S` magic and a container whose chunk directory
   sits at the same offsets, with the same record sizes, as a real SD backup.
3. Its `SYS ` chunk is **byte-identical** to the reference backup's — 768 bytes
   of independently-sourced known plaintext reproduced exactly.

**This corrects `docs/firmware-format.md`** (not edited here, to avoid a merge
conflict) on two points:

1. The header field at `0x14` documented as `load_addr[3] = 0x0033FFD0` is the
   **decompressed size**, not a load address — it equals the decode output
   length exactly. `section_size[3] = 0x0006B858` is the *compressed* size.
2. The "ASCII tags embedded in binary tables / `uint16` runs at stride `0x12`"
   description of the section content was an artifact of reading compressed
   data. The real content is plain.

### What the firmware actually carries — CONFIRMED

Each decompressed section is a **complete factory-default TR-6S backup image**,
in the same container this document describes:

| Tag | Header @ | Payload |
| --- | -------- | ------- |
| `SYS ` | `0x40` | 752 B |
| `PTN ` | `0x350` | 3,136,512 B |
| `KIT ` | `0x2FDF70` | 167,936 B |
| `TONE` | `0x326F90` | 36,864 B |

Same magic, same chunk offsets, same record sizes as the SD-card backup. The
image simply ends where the backup's `SMPL` chunk begins (`0x33FFD0`) — the
factory ships no user samples. So `tr-format` parses the decompressed firmware
blob with no changes at all, and the project gains a **second, factory-clean
corpus** for every section, obtained without touching the encrypted `App1_Main`.

### The reconciliation itself — CONFIRMED

| Section | v2.00 factory vs. v1.51 backup |
| ------- | ------------------------------ |
| `SYS ` (header + 752 B payload) | **byte-identical, 0 differing bytes** |
| `TONE` (36,864 B) | **byte-identical** |
| `KIT ` | 97.8 % identical (user-edited kits differ) |
| `PTN ` | 97.5 % identical (user-edited patterns differ) |
| File header `0x00`–`0x1F`, `0x2A`–`0x3B` | identical |
| File header `0x20`–`0x29`, `0x3C`–`0x3F` | all 11 differing bytes fall in these two windows |

That last row is a free bound on the **file-header checksum**: whatever the
`0x20` region computes over, only those 14 bytes react to a change of contents.

So the answer to "where does the shared body start, how long is it, does v2.00
agree with v1.51" is: **the whole `SYS ` chunk, all 768 bytes including its
header, identical across both firmware generations.** Every field in the map
above is therefore corroborated on both corpora.

**The honest caveat.** Byte-identity means the reference backup's system
settings were *never changed from factory* — so this is one value sample seen
twice, not two independent samples. It proves the layout is stable v1.51→v2.00
and that the observed values are Roland's factory init (which is why `USB Audio`
and the `sysSound` block differ from TR Editor's defaults: those are the
*editor's* defaults, which are not the device's). It does **not** substitute for
a save-change-save diff, which is still what would let us watch a field move.

### Bonus: the `+0x08` token *co-varies* with section content (weak, confounded)

Falling out of the same comparison, and relevant to the `+0x08` field section
below:

| Section | Content identical v1.51↔v2.00? | Token identical? |
| ------- | ------------------ | ---------------- |
| `SYS ` | yes | **yes** |
| `TONE` | yes | **yes** |
| `KIT ` | no | **no** |
| `PTN ` | no | **no** |

Tempting to read this as "the token is a content digest" — but the evidence is
**weaker than it looks, and it is confounded**, so this is a lead to test, not a
finding:

- **The `yes → yes` rows are near-tautological.** The token lives *inside* the
  section payload (record 0's `+0x08`). If two sections are byte-identical, every
  byte including the token is identical by definition — that says nothing about
  what computes it.
- **The `no → no` rows don't isolate a digest.** Between two firmware builds a
  section that changed content changed in *many* ways at once; the token
  differing is equally consistent with it being an independent build/version
  stamp that got regenerated, not a hash *of* the content.

So n is effectively closer to 2 than 4, and even those 2 don't distinguish
"content digest" from "field that happens to change alongside content." The
discriminating test is the one this corpus can't do: **change a single content
byte, hold everything else, and see whether the token moves** — a
save-change-save diff on hardware. Until then it stays flagged, not adopted.
Prior sweeps already ruled out plain CRC-32/64 and MD5/SHA1/SHA256.

## The `+0x08` field — NOT a per-record checksum (2026-08-07)

Investigated for device-safe writes. Result: **there is no per-record
checksum.** The 8-byte field at `+0x08` is **zero for every record except
record 0** of each section (verified: kits 1–127 and patterns 1–127 are all
`00…00`). Only record 0 carries a value (kit `5CDA15CA6F3A2E12`, pattern
`316A2D82BE026C64`), i.e. a single **section-level token** parked in record 0's
header slot.

Consequences:

- **Editing records ≥ 1 is checksum-free** — the field is already zero and stays
  zero, so length-preserving edits to any user kit/pattern slot need no
  recomputation. This unblocks `tr-studio` writes to user slots. **Caveat (flag,
  not a reversal):** the cross-version comparison above is *consistent with* the
  record-0 token being a digest over the whole section payload (records ≥ 1
  included), which would make it need recomputation — but that evidence is
  confounded (see the "Bonus" note) and does not distinguish a digest from a
  build stamp. "Checksum-free for records ≥ 1" stands as the working position;
  the open risk is only that editing slot 0's neighbours might require updating
  slot 0's token, and that needs a hardware save-change-save diff to settle.
- The record-0 token's algorithm is unresolved: it is **not** a plain
  CRC-32/64, MD5/SHA1/SHA256 (first/last 8) of the section, record, or backup
  (all swept and ruled out). Likely keyed/Roland-specific or device-generated.
  It matters only if you edit slot 0 *and* the device verifies section
  integrity on restore — an empirical question to settle by testing a modified
  backup on the device (needs the SD reader).
- Ruled out as the algorithm: `ML::CRolandMessage::CheckSum` in TR Editor is the
  **SysEx** checksum (sum two regions, negate, `& 0x7F`), not this field.

Practical guidance for the librarian: edit user slots (≥1), preserve record 0
untouched, and a written backup should be structurally valid; confirm the device
accepts it once hardware is available.

## Losslessness contract

The [`tr-format`] crate retains the original bytes and returns them unchanged
from `to_bytes()`, so `parse(x).to_bytes() == x` **always** holds — verified on
the real 54 MB backup (`tr-format verify`). The section list is an index over
retained bytes, never a re-serialization; edits are length-preserving in place.
This is the safety property a librarian needs: touching one section can never
corrupt unknown/reserved bytes.

### Typed write API

Every decoded field has a setter that mirrors its reader and writes **in place**,
so the losslessness contract holds at field granularity — an edit changes only
the bytes of the field it targets and never a byte outside it (asserted by
`assert_changed_within` in the tests):

- **Kit:** `set_name`, `set_voice_tone`, `set_voice_params`, and
  `set_reverb`/`set_delay`/`set_ext_in_fx`/`set_mfx`/`set_inst_fx`.
- **Pattern:** `set_name`, `set_tempo_bpm` (clamped 40–300), `set_kit_ref`, plus
  the low-level `set_step_word`/`set_motion_word`.
- **Sys:** `set_general`/`set_sound`/`set_midi`, `set_category_name`.

The block-shaped structs (`VoiceParams`, `SysGeneral`/`Sound`/`Midi`,
`ReverbParams`/`DelayParams`/`ExtInFx`) implement a **`RolandBlock`** trait —
`from_block` / `write_to` / `LEN` — that formalises the byte-exact round-trip and
lets one generic test prove `write_to ∘ from_block = id` for all of them. The
types stay byte-faithful (raw `u8` fields, not enums) precisely to keep
losslessness: a semantic view with enums/ranges belongs one layer up
(`tr-studio`), where dropping an unknown value is harmless. `MfxParams` /
`InstFxParams` span two sub-blocks (and carry the slot-10 truncation), so they
keep bespoke `Kit::set_mfx`/`set_inst_fx` rather than the trait.

### JSON/serde (`serde` feature)

The value structs derive `Serialize`/`Deserialize` behind an off-by-default
`serde` feature, so any decoded record serialises to JSON/TOML for human editing
and diffing. **JSON is a known-fields view, not a byte-exact one:** it carries
the decoded fields only, so it is *lossy* for a record's unknown/reserved bytes.
That is deliberate — the losslessness guarantee lives in the binary layer (edit
via the write API on the retained bytes), and JSON exists for readability and
diffs. Round-tripping *through JSON* (`typed → JSON → typed`) is exact on the
known fields; round-tripping a whole record *through JSON back to a device
backup* should go binary→edit→binary, not JSON→binary, to preserve the unknown
bytes. The aggregate export/import documents and CLI live in `tr-studio`.

## Status & next steps

Done (v0 crate): container magic/version, section directory, array shapes,
byte-exact round-trip, length-preserving section edits, **kit record framing +
name** (`tr-format kits` lists all 128, verified against the manifest).

Next (record internals — the RE that turns bytes into editable fields):

1. **Kit voices (6 × in the 0x500 param area)** — find the 6 tone-ID fields and
   map them into the 144-byte tone table (~`0x326FD4`); then per-voice params
   (level/pan/tune/decay/…) via controlled one-change diffs. Also identify the
   `+0x08` per-record checksum so kits can be *written* back safely.
2. **Pattern record** — step words, the slot map, and the motion lane map are
   done. Still open: the **lane-2 flag bit** and the rest of the motion flags
   byte, and which step-word bits hold **per-step probability** (no factory
   pattern sets it). Both want the "save two patterns differing by one
   step/param, then diff" technique — `fw-analyze diff --block`/byte-diff
   pinpoints the changed field — which needs the device.
3. **`SYS ` ↔ `init_param`** — **done.** The section is decoded and the two
   corpora reconcile byte-for-byte. Follow-ups it opened:
   - Add the **LZSS decoder** to `fw-extract` (parameters documented above, ~20
     lines) so the factory image is a first-class corpus instead of a one-off
     script. It belongs there, not in `tr-format`, which stays firmware-free.
   - Fold the `init_param` corrections (the `0x14` field is the **decompressed
     size**, not a load address; the "indexed parameter tables" reading was an
     artifact of compressed data) into `docs/firmware-format.md`.
   - Re-examine the **`+0x08` section token** with the second corpus — but note
     the cross-version signal is confounded (whole-section identity includes the
     token byte), so the discriminating test is a single-byte change on hardware,
     not the corpus. Flagged, not adopted.
   - Still needing hardware: `sysMidi` slots 11–22 (the inferred ALT note map)
     and whether the 18-byte truncated `sysMidi` reserve tail is deliberate.
4. **`FX ` — done.** All effects state lives in the kit record (there is no
   `FX  ` chunk); reverb/delay/master-FX/insert-FX decoded — see "Effects" above.
   Remaining: the **file-header checksum** (`0x20` region — every byte that
   reacts to a content change lies in `0x20`–`0x29` or `0x3C`–`0x3F`, which
   bounds the checksum fields to those 14 bytes).

Then the [`tr-librarian`](../crates/tr-format) layer: browse/organize/dedupe,
JSON/TOML export-import, SD-card layout — the FOSS alternative to Roland Cloud.
