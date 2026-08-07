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
| `0x20` | 32 | header fields + a checksum-looking word (TBD) |

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
| `SYS ` | `0x40` | 752 B | System params. **Same body as the firmware `init_param`** (`d8 0c 34 03 …`); contains the 32 `USER01…32` slot names at payload+~0x7C. |
| `PTN ` | `0x350` | 3,136,512 B | Array: **128 records × 24,504 B** (`0x5FB8`). Payload begins `count(u32)=128, record_size(u32)=0x5FB8`. |
| `KIT ` | `0x2FDF70` | 167,936 B | Array: **128 records × 1,312 B** (`0x520`). Same `count,record_size` preamble. |
| `SMPL` | `0x33FFD0` | 0 B | User samples; empty here (none loaded). `extra` ≈ 0x3300000 = the reserved sample region — accounts for the ~51 MB of zero padding to EOF. |
| `FX  ` | — | — | Seen ×4; effects config. Not yet mapped. |

Array sections self-describe with a `count,record_size` preamble, so the parser
locates records generically.

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

FX (`FX  `) and SYS decode the same way from `Script.xml`.

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
  recomputation. This unblocks `tr-studio` writes to user slots.
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
3. **`SYS ` ↔ `init_param`** — reconcile the shared system-param body with the
   already-mapped `init_param` structure (`docs/firmware-format.md`).
4. **`FX  `** and the file-header checksum (`0x20` region).

Then the [`tr-librarian`](../crates/tr-format) layer: browse/organize/dedupe,
JSON/TOML export-import, SD-card layout — the FOSS alternative to Roland Cloud.
