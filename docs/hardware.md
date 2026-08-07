# Hardware — TR-6S chip ID findings

Running notes on the parts inside the Roland TR-6S, based on visual inspection
and package markings of the maintainer's own unit. Nothing here is confirmed by
Roland; treat every line as "best current understanding" until corroborated.

## Compute / main SoC — OPEN

The identity of the main compute chip is the central open question. Candidates
seen referenced across Roland's post-AIRA generation:

| Label seen | Notes |
| ---------- | ----- |
| **ESC2**   | Roland-branded custom ("Engine Sound Chip"?) part used in several AIRA / boutique units. Most likely candidate. |
| **BMC**    | "Behavior Modeling Core" branding — appears in ACB marketing; may be a marketing name for a DSP block rather than a discrete part. |
| **E4E**    | Seen on some teardown photos; relationship to ESC2 unclear (successor? sibling?). |

Key unknowns tracked in `architecture-questions.md`: actual silicon vendor,
ISA, clock, and whether it is a single SoC or a main CPU + DSP coprocessor.

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
