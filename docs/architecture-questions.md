# Architecture — open questions log

A living checklist. Each item is something we do **not** yet know about the
TR-6S platform. Move items to `hardware.md` / `firmware-format.md` with evidence
as they get answered. Keep the question here (struck through) with a pointer, so
the reasoning trail survives.

## Compute chip

- [x] **Exact part number** of the main compute chip. Is `ESC2` the real
      silicon, or a Roland label over a third-party SoC?
      → **BMC SoC** — dual-core ARM Cortex-M4/M7. See `bmc-family-findings.md`.
- [x] **ESC2 vs BMC vs E4E** — which of these is a discrete part vs. a
      marketing name for a DSP block? (see `hardware.md`)
      → **BMC** = main SoC family (MC-101/707/TR-8S/TR-6S/Fantom/Jupiter-X).
      **E4E** = separate SoC for AIRA Compacts (T-8, J-6, E-4). See `bmc-family-findings.md`.
- [x] **ISA / architecture** — ARM (Cortex-A/-M/-R?), a Tensilica/Xtensa-style
      DSP, SuperH legacy, or something custom?
      → **ARM Cortex-M4/M7, Thumb-2 only.** Dual-core: Core 0 = app/UI/SysEx,
      Core 1 = DSP/voice engine. See `bmc-family-findings.md`.
- [ ] **Clock speed(s)** — core clock, DSP clock, memory clock.
- [x] **On-chip vs external** boot ROM — is there an internal bootloader?
      → **External NOR flash.** Bootloader and shared kernel are not shipped
      in firmware update files. See `bmc-family-findings.md`.

## Memory map

- [x] How is the **64 MB NOR** presented — execute-in-place, or copied to SDRAM
      at boot?
      → **QSPI-XIP at 0x60000000.** Wave-ROM directory, parameter tables, and
      boot helpers are mapped for execute-in-place. See `bmc-family-findings.md`.
- [x] How is the **32 MB SDRAM** partitioned (code, heap, ACB voice state,
      audio buffers)?
      → **0x20000000–0x201xxxxx** (.data / .bss / heap / task queues).
      See `bmc-family-findings.md`.
- [ ] Are the two ESMT SDRAM devices a single 32-bit bus or two 16-bit banks?
- [x] Physical **base addresses** of NOR, SDRAM, and MMIO peripherals.
      → Full memory map in `bmc-family-findings.md` (SDRAM at 0x20000000,
      peripherals at 0x40000000, QSPI at 0x60000000, NVIC at 0xE0000000).

## Audio / ACB signal path

- [ ] Do **ACB voices** run on the main compute chip, or route through a
      **separate DSP coprocessor**?
- [ ] Where do the analog-circuit-behavior models actually execute — this
      determines whether PedalKernel-derived **WDF circuit models** could ever
      be hosted, and on what budget.
- [ ] Sample rate / block size of the internal audio engine.
- [ ] Codec/DAC part and its interface (I²S? TDM?).

## Update / boot security (feeds the phase gate)

- [ ] Is the firmware image **signed**? (see `firmware-format.md`)
- [ ] Does the bootloader **verify before flashing**?
- [ ] Is there a **recovery / DFU** mode reachable without opening the unit?
- [ ] What is the **write path** to NOR — and is it safe to target? (This is
      the gate for any future flashing work; nothing in this repo writes to the
      device until it is answered.)

## Toolchain / next steps

- [ ] Identify a JTAG/SWD or UART debug pad on the PCB.
- [ ] Confirm the compute chip's ISA so a disassembler target can be chosen.
