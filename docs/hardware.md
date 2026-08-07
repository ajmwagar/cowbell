# Hardware — TR-6S chip ID findings

Running notes on the parts inside the Roland TR-6S. **Provenance:** the
maintainer has **not** opened their unit (owned ~1 day as of 2026-08-07); part
IDs here are from **third-party online teardowns/photos**, not first-party
inspection. The E4E/STM32G0 findings specifically come from Reddit photos of a
sibling **Beat 8**, not a TR-6S. Nothing here is confirmed by Roland or by
first-party inspection; treat every line as "best current understanding" until
corroborated. Getting a clear photo of the **TR-6S main SoC** is an open task —
it would confirm whether the TR-6S main chip is the same E4E seen on the Beat 8.

## Compute / main SoC — TR-6S almost certainly BMC

Two distinct, confirmed Roland custom SoCs now anchor this. The **big TR boxes
use BMC**; the **AIRA Compact line uses E4E**:

| Label | Status | Where seen |
| ----- | ------ | ---------- |
| **BMC** | **CONFIRMED real, discrete Roland silicon.** The TR-6S main SoC — its key-sharing sibling the TR-8S is BMC — so **TR-6S ≈ BMC** (strong inference, see below). | TR-8S main board (YouTube teardown) |
| **E4E** | **CONFIRMED real, discrete Roland silicon.** A *different* part; the AIRA Compact / Beat 8 SoC. Explains why the T-8 firmware uses a different key. | Beat 8 main board (Reddit) |
| **ESC2** | Referenced online for AIRA/boutique units; not observed in any photo we hold. Likely a different/earlier part or another product line. | — (unconfirmed) |

Both carry Roland's `5100xxxxxx` lot scheme (BMC `5100440716`, E4E `5100069694`)
— sibling custom parts, same foundry generation, distinct designs.

Correction to earlier notes: BMC is **not** merely ACB marketing ("Behavior
Modeling Core") — it is a real BGA part. And the TR-6S is **not** E4E (that is
the Compact-line chip); the E4E lead came from a different-key cousin and was
superseded by the direct TR-8S BMC photo.

Key unknowns tracked in `architecture-questions.md`: actual silicon vendor,
ISA, clock, and whether each is a single SoC or a main CPU + DSP coprocessor.

### BMC — confirmed discrete Roland SoC; the TR-family main chip (2026-08-07)

Confirmed from a **third-party YouTube teardown of a Roland TR-8S** (not the
maintainer's hardware):

- Marking: `Roland` / **`BMC`**, large BGA package.
- Lot/part: `5100440716`.
- Adjacent RAM: an **ISSI** SDRAM (note: the TR-6S is reported with ESMT SDRAM —
  vendor differs by product, unsurprising).

**Why this pins the TR-6S to BMC.** The TR-6S and TR-8S `App1_Main` images share
one encryption key (proven: a 15,936-byte identical ciphertext run at a fixed
offset — see `firmware-format.md`), the same load map (`0x60000000`), the same
container/CRC scheme, and 66k shared blocks. Roland ties that key + bootloader +
memory map to a hardware platform, so the two boxes almost certainly run the
**same main SoC** — and the TR-8S one is directly photographed as BMC. This is a
**stronger** chain than the E4E lead was: it runs through the *key-sharing*
sibling, not a different-key cousin. Still not first-party; a TR-6S board photo
would make it airtight.

### E4E — confirmed discrete Roland SoC (AIRA Compact line), 2026-08-07

Confirmed from **third-party teardown photos (Reddit)** of a **Roland AIRA
Compact "Beat 8"** (T-8-class) board — a sibling product, *not* the TR-6S, and
**not the maintainer's own hardware.** Treat as external corroboration, not a
first-party observation:

- Marking: `Roland` / **`E4E`**, BGA package, `Roland MADE IN CHINA`.
- Lot/part: `5100069694`, date code `2304` (2023, wk ~04).

E4E is a real, discrete Roland SoC — but a **different part from the BMC** in the
TR boxes. This is consistent with (and explains) the T-8 firmware using a
different encryption key than the TR-6S/TR-8S. The Beat 8 remains a candidate
E4E reference platform, but is **not** representative of the TR-6S main chip and
**not on hand.** See `architecture-questions.md` next-steps.

### E4E — confirmed discrete Roland SoC (2026-08-07)

Confirmed from **third-party teardown photos (Reddit)** of a **Roland AIRA
Compact "Beat 8"** (T-8-class) board — a sibling product, *not* the TR-6S, and
**not the maintainer's own hardware.** Treat as external corroboration, not a
first-party observation:

- Marking: `Roland` / **`E4E`**, BGA package, `Roland MADE IN CHINA`.
- Lot/part: `5100069694`, date code `2304` (2023, wk ~04) — same silicon
  generation as the TR-6S.

This resolves the "is E4E marketing or a real part" question: **it is a real,
discrete, Roland-branded SoC.** The Beat 8 is a candidate **reference platform**
for characterising the E4E (cheap, firmware-downloadable) — but note **we do not
currently have one on hand**; the hardware-probing sub-tasks require acquiring a
unit first. See `architecture-questions.md` next-steps.

Caveat: this confirms E4E on the *Beat 8*. Whether the **TR-6S** main chip is
also an E4E (vs. an ESC2) still needs a direct read of the TR-6S board markings
to confirm. The two may share the E4E, or the TR-6S may use a larger sibling.

### Panel / IO MCU — STM32G0 (ARM Cortex-M0+), confirmed

Also from the same third-party Beat 8 photos: the panel/keybed PCB (connector
`CN301`) is driven by an ST **STM32G0** in LQFP48 (`…C8T6`), i.e. an **ARM
Cortex-M0+ (ARMv6-M)** — a standard, fully-documented part. A `74HC138`
(`HA138`, IC304) 3-to-8 decoder next to it scans the button/LED matrix.

Design takeaway: Roland splits the system into a **stock ARM Cortex-M for
panel/IO** and the **custom E4E for audio/DSP**. The TR-6S very likely follows
the same split. Any panel-MCU firmware is therefore plain ARM Thumb and
directly analysable; the audio path is the hard, encrypted target.

### Candidate external flash — VERIFY

An **8-pin SOIC** marked `251` / `P2368` sits immediately beside the E4E in the
Beat 8 photos. Package and placement are consistent with a small **SPI NOR
flash / EEPROM**. If it is SPI NOR, it would be in-circuit clip-dumpable
(SOIC-8, no soldering) **on a unit we obtained** — but we have no Beat 8 on hand,
so this is a plan contingent on acquiring one, not something actionable now.
Marking not yet matched to a datasheet; confirm before assuming.

## Confirmed memory parts

These are legible from package markings and are considered confirmed:

- **SDRAM:** 2× **ESMT M12L128168A**
  - 128 Mbit each (8M × 16), so 2× 16 MB = **32 MB** total working RAM.
  - 16-bit data bus per device; likely run as a 32-bit bus across the pair, or
    two independent 16-bit banks — TBD from trace-out.
- **NOR flash:** **Infineon / Cypress S29GL512S10TFI020**
  - 512 Mbit = **64 MB** parallel NOR.
  - S29GL-S family, 110 ns, TFI (56-ball) package, 3 V.
  - Almost certainly holds the firmware image + sample/wavetable ROM data.
    This is the primary carving target for `fw-analyze`.

## Working memory-map hypotheses (unverified)

- 64 MB NOR mapped for XIP or copied to SDRAM at boot — unknown which.
- 32 MB SDRAM split between code/working set and ACB voice state — unknown.

See `architecture-questions.md` for the full open-questions log.
