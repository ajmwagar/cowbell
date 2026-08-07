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

#### Voice block (0x34 bytes) — tentative structural map

From value-distribution analysis over all 768 voice blocks + a blank-vs-active
diff. **Types are inferred from default values; the names are guesses.** Only
the tone ID is confirmed.

| Off | Type | Default | Guess | Confidence |
| --- | ---- | ------- | ----- | ---------- |
| `+0x00` | u16 | — | **tone ID** | confirmed |
| `+0x02` | u8 | `0x80` | pan (bipolar, center 0x80) | type likely, name guess |
| `+0x03` | u8 | `0x80` | tune (bipolar) | type likely, name guess |
| `+0x04` | u8 | `0xFF` | level or decay (unipolar max) | type likely, name guess |
| `+0x05..0x0B` | — | `51 80 80 e0 01 01 80` | fixed template / reserved | — |
| `+0x0C..0x1B` | — | `0x00` | padding | — |
| `+0x1C..0x29` | mixed | `0x00` | sparse params (envelope/sends) | structural only |
| `+0x2A..0x33` | — | `0x00` | padding | — |

**Statistics have hit their ceiling for *naming*** — they can say `+0x02` is a
centered bipolar param but not whether it's pan vs tune. Two ways to finish:

1. **Controlled diff** — save two kits differing by exactly one voice parameter,
   diff (`fw-analyze diff --block`). Needs an SD reader.
2. **Decompile TR-EDITOR** (Roland's official editor) — its code carries the
   full data model (param names, ranges) and the SysEx address map, which would
   *name* every field and confirm these offsets. Needs no SD reader. See
   `docs/prior-art.md` / the software-surface avenue.

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
2. **Pattern record (24,504 B)** — steps/tracks/motion. Use the "save two
   patterns differing by one step/param, then diff" technique — `fw-analyze
   diff --block`/byte-diff pinpoints the changed field.
3. **`SYS ` ↔ `init_param`** — reconcile the shared system-param body with the
   already-mapped `init_param` structure (`docs/firmware-format.md`).
4. **`FX  `** and the file-header checksum (`0x20` region).

Then the [`tr-librarian`](../crates/tr-format) layer: browse/organize/dedupe,
JSON/TOML export-import, SD-card layout — the FOSS alternative to Roland Cloud.
