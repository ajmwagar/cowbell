# Hardware — TR-6S chip ID findings

Running notes on the parts inside the Roland TR-6S. **Provenance:** the
maintainer **opened their own TR-6S on 2026-09-01** and photographed the boards
— so the TR-6S main-board parts below are now **first-party confirmed** (see the
teardown section immediately following). The **E4E / STM32G0** findings are a
separate matter: those still come from third-party Reddit photos of a sibling
**Beat 8** and are *not* about the TR-6S. Lines outside the first-party teardown
section remain "best current understanding" until corroborated.

## First-party teardown (2026-09-01) — the big confirmations

From the maintainer's own unit. Board silkscreen ties the physical hardware to
the `dd001` firmware model code directly:

- **Main board:** `DD001 MAIN BOARD NAK ASSY 5100076052`, "A Side", Roland,
  "MADE IN JAPAN / ASSEMBLED IN MALAYSIA".
- **Jack board:** `DD001 JACK BOARD ASSY 5100070378`, "MADE IN MALAYSIA".

So **`dd001` = TR-6S is now silkscreen-confirmed**, not inferred from the update
archive.

| Part | Marking (as read from the photo) | Confirms |
| ---- | -------------------------------- | -------- |
| **Main SoC** | `Roland BMC`, date `2152` (wk52 2021), sticker `76052A221000220`, "JAPAN", large BGA | **TR-6S main chip is BMC — first-party.** Removes the "not first-party" caveat; the earlier chain went through the TR-8S teardown, this is the actual TR-6S. |
| **NOR flash** | `Spansion S29GL512S10TFI02` / `128BB085 D` / `©10 SPANSION`, **TSOP-56** | The 64 MB NOR — and it sits on the **top/component side, right beside the BMC** (see the recon correction below). |
| **SDRAM** | `ESMT M12L128168A-6T` `AZS2P0FLE`, date `2104`, TSOP | ESMT SDRAM confirmed on the real TR-6S (vs. ISSI on the TR-8S — the expected per-product vendor difference). One device visible on the top side; a second likely on the reverse (the 2×=32 MB claim is not yet fully verified from these top-side shots). |

Two new leads from the same photos:

- **Internal microSD socket on the main board.** The DD001 main board carries a
  microSD push-socket (center of the board). The TR-6S has **no user-facing SD
  slot**, so this is either internal storage (samples/OS?) or a populated
  footprint from the shared TR-8S board lineage. **Worth investigating** — if it
  holds readable data it could be a far cheaper route than a NOR dump. Contents
  unknown; handle per the no-distribution policy (it may carry Roland factory
  content).
- **`SW1` DIP switch with a `TMS` silkscreen**, near the BMC (bottom-right of the
  main-board photo). `TMS` is the JTAG Test-Mode-Select signal — a **candidate
  boot-mode / debug-config lead** for the debug-pad hunt in `prior-art.md`.
  Tentative: the label is legible but the switch's actual function is unverified.

Board photos are the maintainer's own and are **not committed** to the repo
(keeping docs text-only); findings are transcribed here.

## Compute / main SoC — TR-6S is BMC (first-party confirmed 2026-09-01)

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
sibling, not a different-key cousin.

**Update (2026-09-01): now first-party.** The maintainer's own TR-6S main board
(`DD001 MAIN BOARD NAK ASSY 5100076052`) carries `Roland BMC` (date `2152`)
directly — the inference chain above is no longer needed. See the first-party
teardown section at the top.

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

Legible from package markings and, as of 2026-09-01, **confirmed first-party**
on the maintainer's own board (see the teardown section above):

- **SDRAM:** **ESMT M12L128168A-6T** (`AZS2P0FLE`, date `2104`)
  - 128 Mbit each (8M × 16) = 16 MB per device. The working assumption is **2×
    = 32 MB** total; one device is visible on the top side of the first-party
    photos, so the second (and thus the full 32 MB) is *inferred*, likely on the
    reverse — verify with a back-side photo.
  - 16-bit data bus per device; likely run as a 32-bit bus across the pair, or
    two independent 16-bit banks — TBD from trace-out.
- **NOR flash:** **Spansion / Infineon S29GL512S10TFI02** (marking `128BB085 D`,
  `©10 SPANSION`)
  - 512 Mbit = **64 MB** parallel NOR.
  - S29GL-S family, **TSOP-56** package, 3 V.
  - Located on the **top/component side of the main board, immediately beside
    the BMC** (first-party) — *not* the reverse side that the TR-8S footage
    suggested. Same-side + TSOP (not BGA) makes in-circuit or chip-off dumping
    materially easier. This is the primary carving target for `fw-analyze` and
    the key-recovery route (`cowbell-8o5`).

## Working memory-map hypotheses (unverified)

- 64 MB NOR mapped for XIP or copied to SDRAM at boot — unknown which.
- 32 MB SDRAM split between code/working set and ACB voice state — unknown.

See `architecture-questions.md` for the full open-questions log.
