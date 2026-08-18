# BMC SoC family — convergent findings from MC-101/MC-707 analysis

Findings from static analysis of Roland MC-101 firmware v1.82 (`RPG69`) and
MC-707 firmware v1.82 (`RPG68`), cross-verified against the TR-6S work in this
repo. The MC-101, MC-707, TR-8S, TR-6S, Fantom, and Jupiter-X all use the same
BMC SoC, so these findings transfer directly.

**Source:** https://github.com/soobrosa/mc101-firmware-re — REPORT.md
**Date:** August 2026

---

## Compute chip

The BMC SoC is a **dual-core ARM** system:

| Core | Role | Code file |
|------|------|-----------|
| Core 0 | Main app: UI, MIDI/SysEx parser, sequencer, parameter engine | `App1_Main` (C0A) |
| Core 1 | DSP: voice engine, audio processing | `sdram1.bin` (C1A) |

- **ISA:** ARM Cortex-M4/M7, Thumb-2 only. Confirmed by LDR from `0xE000ED88`
  (CPACR — Cortex-M System Control Block coprocessor register).
- **BMC** is the main SoC across the product family. **E4E** is a separate
  SoC used in the AIRA Compacts (T-8, J-6, E-4) — different product family.
- **Panel controller:** STM32G0 (Cortex-M0+), communicates with the BMC via
  115200-baud UART speaking MIDI byte streams.

## Memory map

Deduced from `sdram1.bin` (Core-1 DSP code, unencrypted, shipped in updates):

| Address range | Region |
|---|---|
| `0x00000000`–`0x00389B1C` | sdram1.bin (Core-1 code) |
| `0x01100000`–`0x0113xxxx` | Shared kernel / RTOS blob (not shipped in updates) |
| `0x20000000`–`0x201xxxxx` | SDRAM (.data / .bss / heap / task queues) |
| `0x40000000`–`0x40xxxxxx` | Peripherals (GPIO, timers) |
| `0x41100000` | QSPI indirect-read controller |
| `0x60000000`–`0x611xxxxx` | QSPI-XIP (wave-ROM directory, parameter tables, boot helpers) |
| `0xE0000000+` | Cortex-M System Control / NVIC |

The NOR flash holds the bootloader and shared kernel — these are **not shipped**
in firmware update files. The update package only contains the application code
(encrypted) and data/content files (plaintext).

## UMDW inter-core protocol

Core 0 and Core 1 communicate via a custom 32-bit packed message format called
**UMDW** (Universal MIDI Datalink Word). Every SysEx byte, parameter edit, mute
command, and DSP error flows through this pipe.

### Word layout (32-bit little-endian)

| Bits | Field | Meaning |
|---|---|---|
| 0–3 | msg_type | 16-way dispatch |
| 4–7 | route | 0 = default, 1 = PRM-Edit, 7 = SysCmd |
| 8–11 | channel | MIDI channel (0–15) or sub-opcode for system messages |
| 12–15 | (extended) | usually unused |
| 16–23 | data1 | note#, CC#, patch#, SysEx byte, error id, ... |
| 24–31 | data2 | velocity, CC value, aftertouch, SysEx byte, ... |

### 16 message types

| Type | Meaning |
|---|---|
| 0, 3, 15 | Block Edit |
| 1 | System message (subop-dispatched) |
| 2 | MIST Ctrl (RPN/NRPN) |
| 4 | SysEx body chunk (3 bytes/word) |
| 5 | SysEx end (1 residual byte) |
| 6 | SysEx end (2 residual bytes) |
| 7 | SysEx end (3 residual bytes) |
| 8 | Note Off |
| 9 | Note On (vel=0 → NoteOff) |
| 10 | Poly Aftertouch |
| 11 | Control Change |
| 12 | Program Change |
| 13 | Channel Aftertouch |
| 14 | Pitch Bend (signed 14-bit) |

SysEx is transported at 3 bytes per word (type 4). When F7 arrives with N
residual bytes, a type 5/6/7 word flushes them (N = 1/2/3).

### Type-1 system subops

| Subop | Meaning |
|---|---|
| 0x0E | RTE (real-time error) report |
| 0x10 | DSP level metering on/off |
| 0x11 | Unmute begin |
| 0x12 | Mute begin |
| 0x40 | DSP Ready (boot-complete signal) |
| 0x41 | Ack (silent) |
| 0x42 | Mute complete |
| 0x43 | Unmute complete |
| 0x60 | Test Mode Request (factory-service entry) |
| 0x61 | Test Mode Reply |
| 0xFE | Restart Sound Engine |
| ≥0x80 | Parameter-block edit |

## PMB parameter address model

The BMC uses a two-level parameter address model:

- **Table 1 (19 regions):** Primary parameter regions stepping through the `0x41`
  internal namespace in groups of three (0x10000-size, 0x3000-size, 0x8000/0x4000-size
  sub-regions). Each carries Roland internal debug names (Wr(1), Rd(1), SIO#1,
  mxmon1, r1, mac0/1, etc.).
- **Table 2 (11 sub-categories):** 0x1000-stride regions at 0x41110000–0x41138000
  (system block-set A range), named r7, Wr(0), r0, fnc1, mxmon2, DRAMIO3, IO4,
  mxmon0, etc.

The resolver (fn 0x25DA) strips the external address top byte, re-tags with
`0x41000000` (Roland's internal namespace), then walks the table to find the
matching region.

## Firmware update structure

The firmware update is a **GNU tar archive** containing four component files:

| Component | Content | Format |
|---|---|---|
| C0A | App1_Main (main CPU application) | Encrypted |
| C0C | PCM tone banks + init.lzs | Plaintext QSPI container |
| C1A | idm1.bin + sdram1.bin (DSP code) | Plaintext QSPI container |
| C1C | Preset metadata + metronome | Plaintext QSPI container |

The QSPI container format uses a simple header at offset 0x20 ("QSPI" magic),
followed by file entries (28 bytes each: 16-byte name, offset, size, CRC16,
reserved) and page-aligned payload data.

## RTOS

Roland's MC-707 owner's manual (page 2) discloses: "This product contains
eParts integrated software platform of eSOL Co., Ltd." and "This Product uses
the Source Code of μT-Kernel under T-License 2.0 granted by the T-Engine Forum."

The BMC SoC runs **eSOL μT-Kernel** (ITRON-derived RTOS). This is consistent
with the `wai_sem` / `sig_sem` / `cre_tsk` API naming convention seen in the
DSP code's C++ method names.

## Cross-product findings

MC-101 v1.82 and MC-707 v1.82 were built on the same day (~1 hour apart):
- All 26 non-code data blobs (PCM banks, presets, init.lzs, wromInfo) are
  **byte-identical** between the two products
- `sdram1.bin` (DSP code) is the same source recompiled with shifted link addresses
- Only `App1_Main` (the encrypted UI layer) meaningfully differs
- The UMDW protocol, parameter model, and wave-ROM framework are stable since
  at least November 2019 (MC-707 v1.20)

## What this answers from architecture-questions.md

| Question | Answer |
|---|---|
| Exact part number of the main compute chip | BMC SoC — dual-core ARM Cortex-M4/M7 |
| ESC2 vs BMC vs E4E | BMC = main SoC family; E4E = separate (AIRA Compacts) |
| ISA / architecture | ARM Thumb-2, dual-core (Core 0 = app, Core 1 = DSP) |
| On-chip vs external boot ROM | External NOR flash, not shipped in updates |
| How is NOR presented | QSPI-XIP at 0x60000000 |
| How is SDRAM partitioned | 0x20000000+ (.data/.bss/heap/task queues) |
| Physical base addresses | See memory map above |
| Do ACB voices run on main chip or separate DSP | Dual-core: Core 1 is the DSP/voice engine |
