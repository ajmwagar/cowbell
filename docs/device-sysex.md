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
   reads/writes device memory. The repo has **no license (all rights reserved)**,
   so we take only the *facts* (addresses, message layout, ID ranges — not
   copyrightable, and re-observable from any MIDI capture) and **do not vendor its
   code or capture files.** Credit to that repo for surfacing them.
2. **TR Editor** (`cowbell-uqk`) implements the same primitives
   (`FKoaSendRq1`/`FKoaSendDt1`/`FKoaRequestRq1Dt1`) from the other direction.

Two independent sources agreeing is exactly the corroboration this project wants.

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
| `modelId` | n | model identifier (the TR-8S model bytes; from the captures) |
| `cmd` | 1 | **`0x11` = RQ1 (data request / read)**, **`0x12` = DT1 (data set / write)** |
| `addr` | 4 | 4-byte device address (big-endian, 7-bit-safe) |
| `data` | m | present on DT1 (and on the DT1 the device sends back to answer an RQ1) |
| `checksum` | 1 | Roland checksum: `(0x80 − (sum(addr+data) & 0x7F)) & 0x7F` |
| `F7` | 1 | end (247) |

**The read round-trip:** send **RQ1** with an address + length; the device
replies with a **DT1** carrying the bytes at that address. That is a full,
no-hardware read of device memory over USB — the thing `cowbell-uqk` was chasing.
Data payloads >7 bits are 7-bit-packed (the JS `encode/decode7bitBytes`).

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
| `kit.name` | `10 00 00 00` | 16 | block `0x10000`/kit |
| `kit.toneId` | `10 00 10 00` | (u16) | ×**11** @ step `0x100` |
| `ptn.name` | `20 00 00 00` | 16 | block `0x100000`/pattern |
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

Next: capture our own RQ1/DT1 exchange against a device to confirm the `modelId`
bytes and checksum end-to-end, and map the persistent-slot base addresses. A
clean-room Rust implementation over `tr-format`'s typed model would then give
direct device I/O.
