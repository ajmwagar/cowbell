# TR-6S / TR-8S device SysEx — RQ1/DT1 read/write over USB-MIDI

The **device-side** counterpart to the backup file format: how the box's live
memory is addressed and moved over MIDI. This is the no-hardware read path — it
needs only the CTRL USB-MIDI port, no teardown, no NOR dump.

## Provenance & licensing

The message format and device address map below are **facts about Roland's
protocol**, surfaced two independent ways:

1. **`compuphonic/TR-8S-SysEx`** (GitHub) — MIDI-Monitor captures of the Roland
   **ARIA** web sound library (<https://aira.roland.com/soundlibrary-cat/tr-8s/>)
   transferring kits/patterns to a TR-8S, plus a JS reference implementation that
   reads/writes device memory.
2. **TR Editor** (`cowbell-uqk`) implements the same primitives
   (`FKoaSendRq1`/`FKoaSendDt1`/`FKoaRequestRq1Dt1`) from the other direction.

Two independent sources agreeing is exactly the corroboration this project wants.

**On copyright.** What we take here is **Roland's protocol** — a functional
interface (message layout, device addresses, ID ranges), reverse-engineered.
Nobody who *documents* an API owns it: functional interfaces and facts are not
copyrightable (17 U.S.C. §102(b), *Baker v. Selden*), and even the reuse of API
declarations is fair use (*Google v. Oracle*, 2021). So the `compuphonic` repo's
lack of a license does not restrict the **facts** it surfaces — they are Roland's
API, not that author's creative work, and are re-observable from any MIDI
capture. What copyright *would* protect is that author's specific **expression** —
their JS source, prose, and capture files — so we credit the repo and **do not
vendor any of its code or files**; a clean-room implementation from the facts is
unencumbered.

## The message format — Roland RQ1 / DT1

Standard Roland System Exclusive, confirmed by the captures and the JS parser:

```
F0  41  <deviceId>  <modelId…>  <cmd>  <addr…>  <data…>  <checksum>  F7
```

| Field | Bytes | Notes |
| ----- | ----- | ----- |
| `F0` | 1 | SysEx start (240) |
| `41` | 1 | Roland manufacturer ID |
| `deviceId` | 1 | the Utility "SysEx ID" (0-based unit number) |
| `modelId` | 4 | **`00 00 00 45`** — shared by TR-6S **and** TR-8S (confirmed) |
| `cmd` | 1 | **`0x11` = RQ1 (data request / read)**, **`0x12` = DT1 (data set / write)** |
| `addr` | 4 | 4-byte device address (big-endian, 7-bit-safe) |
| `data` | m | present on DT1 (and on the DT1 the device sends back to answer an RQ1) |
| `checksum` | 1 | Roland checksum: `(0x80 − (sum(addr+data) & 0x7F)) & 0x7F` |
| `F7` | 1 | end (247) |

**The read round-trip:** send **RQ1** with an address + length; the device
replies with a **DT1** carrying the bytes at that address. That is a full,
no-hardware read of device memory over USB — the thing `cowbell-uqk` was chasing.
Data payloads >7 bits are 7-bit-packed (the JS `encode/decode7bitBytes`).

**Checksum — external anchor.** The `(0x80 − (sum & 0x7F)) & 0x7F` form is pinned
to Roland's *own* published worked example (SC-88 MIDI implementation): the run
`40 00 7F 00` has checksum `41`. `tr-sysex` asserts this in a unit test, so the
routine matches Roland's spec, not just our internal invariant.

**7-bit ties to `tr-format`.** Because every wire byte is `0x00..=0x7F`, a
parameter spans the wire in `⌈bits(range_max) / 7⌉` bytes — the *same*
`int4x4`-sizing rule `tr-format` derives from `Script.xml` (a 0–1023 field is 2
bytes, a 16-bit mask 3). A field read via RQ1 unpacks with the exact width
`tr-format` uses to locate it in a backup.

## The device address map

Roland's parameter-address model for the edit buffer (`temp`), recovered from the
ARIA JS config. Addresses are 4-byte `[hi..lo]`; `step`/`count` describe arrays;
`block` is the per-slot stride when the same field repeats across kits/patterns.

| Field | Address | Size | Step × Count |
| ----- | ------- | ---- | ------------ |
| `sys.categoryName` | `00 01 00 00` | 16 | ×32 @ step `0x10` |
| `stp.currentKit` | `01 00 00 00` | 1 | |
| `stp.currentPattern` | `01 00 00 01` | 1 | |
| `stp.nextPattern` | `01 00 00 02` | 1 | |
| `stp.patternSelect` | `01 00 00 1B` | 4 | |
| `kit.name` | `10 00 00 00` | 16 | block `0x4000`/kit † |
| `kit.instrument` | `10 00 10 00` | **16** | ×**11** @ step `0x80` † |
| `kit.toneId` | `10 00 10 00` | (u16) | first 2 bytes of `kit.instrument` † |
| `ptn.name` | `20 00 00 00` | 16 | block `0x40000`/pattern † |
| `ptn.kitReference` | `20 00 00 14` | 2 | |
| `ptn.kitReferenceSw` | `20 00 01 06` | 1 | |
| `tone.name` | `30 00 00 00` | 16 | block `0x10000`/tone |
| `tone.category` | `30 00 00 10` | 1 | |
| `tone.type` | `30 00 00 11` | 1 | |
| `tone.address` / `addressRight` | `40 00 00 00` / `…08` | 8 / 8 | sample PCM start (L/R) |
| `tone.size` | `40 00 00 10` | 8 | sample length |
| `tone.channel` | `40 00 00 38` | 1 | mono/stereo |
| `utility` | `50 00 00 00` | — | |

Address regions: `0x01` step/system, `0x10` kit, `0x20` pattern, `0x30` tone
metadata, `0x40` tone PCM (sample refs), `0x50` utility. Persistent slots are the
same fields at slot-offset base addresses (the JS walks them via an `offsets`
table + `offsetAddress`).

**† Confirmed and corrected by a real transfer capture.** The compuphonic
"send pattern/kit" MIDI-Monitor capture is attested wire traffic (DT1 writes to a
device). It resolves the earlier ambiguity in the `block`/`step` literals — those
JS numbers are *byte-patterns*, and the real on-wire delta is the **base-128
value of one address digit**, which is 4× smaller:

- **Kit slot** → the 0-indexed kit number lands directly in address byte 1: kit
  126 ("kit 127" 1-indexed) is written at `10 7e 00 00`. Per-kit delta `0x4000`,
  not `0x10000`.
- **Kit instrument record** → 11 blocks of **16 bytes** (not a bare `u16` tone
  id), stepping address byte 2 by 1: `10 7e 10 00 … 10 7e 1a 00`. The tone id is
  the first two bytes; the remaining 14 are the device's per-voice parameter
  encoding (not yet field-decoded — the backup `VoiceParams` is the decoded
  reference for the same knobs).
- **Pattern slot** → address byte 1 steps by `0x10` per pattern (`20 00 00 00` →
  `20 10 00 00`), 8 patterns per region byte, rolling `0x20 → 0x21 → …`. Per-
  pattern delta `0x40000`, not `0x100000`. This capture transferred only pattern
  **headers** (name + `kitReference` + `kitReferenceSw`); pattern **step words**
  and **FX** blocks were not in it and have no confirmed device address yet.

`tone.*` strides are still on the doc/JS literal alone — no tone region appeared
in this capture. `tr-sysex`'s `address` module carries the corrected values and a
regression test (`strides_match_the_send_pattern_capture`) pins these exact
attested addresses.

### These are NOT the editor/backup addresses (a discrepancy to respect)

`Script.xml` and the backup-file format address the same sections **differently**:
`kit = 03 00 00 00`, `ptn = 04 00 00 00` (see `docs/tr-format.md`). The device
SysEx capture above puts kit at `0x10`, pattern at `0x20`, tone at `0x30` —
*different region bytes*. So the editor/backup model and the device's RQ1/DT1
address space are **two coordinate systems, not one.**

This matters because a parallel effort derived the device addresses from
`Script.xml` and assumed they were the same (`kit = 03 00 00 00` sent in an RQ1).
The **ARIA capture is real wire traffic to a device**, so it is the authority:
send the `0x10`/`0x20`/`0x30` addresses, not the Script.xml ones. `tr-sysex`
exposes the Script.xml bases under `address::editor_model` **only** for
cross-reference, explicitly marked as *not* the device addresses. Whether the
device also answers on the editor coordinates is an open question for a capture —
current evidence says it does not.

### Device constants (from the same config)

| Constant | Value | Meaning |
| -------- | ----- | ------- |
| pattern IDs | 0–127 | 128 patterns |
| kit IDs | 0–127 | 128 kits |
| **tone IDs** | **624–1023 user** | preset tones are **0–623**; user tones start at 624 |
| sample sectors | 104–511, `0x20000` each | 128 KB sectors |
| sample address | `0xD00000`–`0x3FFFFFF` | mapped sample RAM window |
| `storageSize` | `0x3300000` (≈51 MB) | user-sample storage |
| firmware | `rpg42_m0c0a_up.bin`, `rpg42_init_param.bin` | **confirms `rpg42` = TR-8S**, and TR-8S ships an `init_param` too |

## Cross-validation against our backup RE

The device map and our independently-reversed **backup file** format agree, which
raises confidence in both:

- **32 category names × 16 B** (`sys.categoryName`, step `0x10`, count 32) = our
  `SYS ` `sysCategory` block (512 B = 32×16, the `USER01…32` names). Exact.
- **`storageSize = 0x3300000`** = the `SMPL` chunk's reserved `extra` region and
  the ~51 MB of zero padding to EOF in the backup. Exact.
- **Full backup = 56.9 MB** (per the repo) = our `tr6s_bak.bin` (56,885,232 B).
  Same container across TR-6S and TR-8S.
- **Tone = name[16] + category + type** (`0x30` region) = our `TONE` entry layout.
- **11 kit tone-IDs** (`kit.toneId`, count 11) = the TR-8S instrument count
  (`INST_TRACKS`); a TR-6S populates 6 of them.
- **Pattern kit-reference is a u16** on the device (`ptn.kitReference`, size 2) —
  our backup stores the referenced kit in a single byte at record `+0x22`; the
  wider device field is consistent (the value fits in one byte).

New facts we did **not** have from the backup alone: the **preset/user tone
boundary at 624**, the sample sector geometry, and the device edit-buffer address
model itself.

## What this unlocks

- **A no-hardware device read path is confirmed to exist**, with a working
  reference: RQ1 at an address → DT1 reply. This reads kits/patterns/tones/system
  and the sample map straight off the device over USB, with no teardown — the goal
  of `cowbell-uqk`, largely answered by observation rather than decompilation.
- **A FOSS librarian could talk to the hardware directly** (read/organise/write
  user slots) in addition to editing backup files — the two halves of a Roland
  Cloud alternative.
- It does **not** touch firmware decryption: this is the plaintext user-data
  plane, same as the backup. The `App1_Main` key is still a NOR-dump problem
  (`cowbell-8o5`).

## The `tr-sysex` crate + CLI

The [`tr-sysex`](../crates/tr-sysex) crate implements this protocol clean-room
(build/parse RQ1/DT1, the checksum, 7-bit packing, the address map, a `MidiPort`
trait for the read/write round-trip). It also ships a byte-only CLI — no MIDI
I/O, it just prints messages to inspect or diff:

```sh
tr-sysex checksum "40 00 7F 00"        # -> 0x41 (Roland's SC-88 worked example)
tr-sysex rq1 kit --size 0x520          # RQ1: read one kit record (device kit region 0x10)
tr-sysex parse "F0 41 10 00 11 10 00 00 00 00 00 0A 20 46 F7"   # describe + verify checksum
tr-sysex dt1 pattern --data "01 02 03 7F"   # DT1 write (prints bytes only; sends nothing)
```

The region names (`kit`/`pattern`/`tone`/`sys`) resolve to the **device** SysEx
bases, not the editor-model ones.

Next: capture our own RQ1/DT1 exchange against a device to confirm the `modelId`
bytes and checksum end-to-end, and map the persistent-slot base addresses. A
clean-room Rust implementation over `tr-format`'s typed model would then give
direct device I/O.

## Confirmed from real TR-8S captures (2026-08-10)

The `compuphonic/TR-8S-SysEx` repo's `.mmon` files are **MIDI-Monitor captures of
actual TR-8S wire traffic** (binary plists; the messages are `SMSystemExclusiveMessage`
objects whose `NS.data` holds each message body `41 <dev> <model×4> <cmd> <addr×4>
… <ck>`). Decoding two of them — "initial connection" (1,000 messages) and "send
pattern 8-16 kit 127" (956) — settles everything that was inferred, with the facts
extracted (not the files vendored):

| Field | Captured value | Was |
| ----- | -------------- | --- |
| device ID | `0x10` (all 1,956) | default assumption |
| **model ID (TR-8S)** | **`00 00 00 45`** (all 1,956) | *unknown / parameter* |
| command | RQ1 `0x11` / DT1 `0x12` | ✓ |
| **checksum** | **0 mismatches / 1,956** vs our `roland_checksum` | corroborated → **confirmed** |
| address | 4 bytes, 7-bit | ✓ |
| **RQ1 length** | **4-byte field**, base-128 | inferred width |
| **base-128** | a 1,171-byte request is `00 00 09 13` — impossible in base-256 on a 7-bit wire | inferred |
| DT1 data | every byte `≤ 0x7F` | ✓ |

`tr-sysex` now ships `MODEL_ID_TR8S` + `DeviceConfig::tr8s()`, promotes the
address/length/checksum docs from *inferred* to *confirmed*, and has a test built
from a real captured RQ1 (`… 47 2c 00 10 00 00 00 08 75`).

Two nuances the captures surface:

- **No 256-byte cap for the TR-8S.** Single DT1s run up to **1,171 data bytes** —
  the JV-era 256-byte limit does not apply here, so bulk kit/pattern transfers do
  *not* need chunking at that size (`cowbell-1ne.3` is lower priority than thought).
- **7-in-8 packing is likely unused.** Multi-byte parameters are carried
  **base-128 per field** (`⌈bits/7⌉` bytes — the same rule `tr-format` derives from
  `Script.xml`), not MIDI 7-in-8 packed. The `encode_7bit` helper is kept but
  flagged as probably-not-the-device-scheme.
- The **TR-6S shares the model ID** `00 00 00 45` — confirmed from TR Editor's
  Script.xml (the single `TR CTRL` midiIn/midiOut declares it for both boxes; the
  two are told apart by a device-model field, 1=8S / 2=6S, not the model ID).

## Corroboration — the universal Roland RQ1/DT1 spec

Roland's RQ1/DT1 is a **universal** scheme, unchanged since the GS era, so a
generic reference independently confirms the parts of this protocol that are not
TR-specific. Glenn Meader's
[Roland SysEx primer](http://www.chromakinetics.com/handsonic/rolSysEx.htm)
(written for the JV-1010) confirms, from a source unrelated to our capture or to
TR Editor:

- the framing `F0 41 <dev> <model> <cmd> <addr> <data|size> <checksum> F7`;
- **RQ1 = `0x11`** ("Data Request 1"), **DT1 = `0x12`** ("Data Set 1"); default
  device ID **`0x10`**;
- the **checksum** as "sum the address + data/size bytes, take the remainder mod
  128, subtract from 128 (128 → 0)" — identical to
  `(0x80 − (sum & 0x7F)) & 0x7F`, with worked examples;
- **RQ1 carries a 4-byte size/count** (`F0 41 10 6A 11 00 00 00 01 …` requests
  one byte) — corroborating our 4-byte length field;
- the **model ID is per-device** (JV-1010 = `6A`, generic Roland = `42`), which
  is why the TR-6S/TR-8S model bytes remain legitimately unknown until captured.

So the framing, command bytes, checksum, device ID, and the RQ1 size-field width
are confirmed *twice over* (this spec + our capture); only the **TR model ID**
and the **7-bit *packing layout*** of multi-byte data still need a real device
capture.
