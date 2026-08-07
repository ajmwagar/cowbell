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

The voices reference tones by **ID**, not by name: the tone/instrument names
live in a separate table just past the KIT section (~`0x326FD4`) with **144-byte
(`0x90`) entries**, name at the entry start. So decoding a voice = (a) find the
6 tone-ID fields in the kit record, (b) map them into that 144-byte table.

Reversing the per-voice params: save two kits differing by exactly one voice
parameter and diff them (`fw-analyze diff --block`) to pin each field. Record 0
vs record 1 differ in ~121 scattered bytes (name + all params), so a controlled
one-change diff is the way to isolate individual fields.

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
