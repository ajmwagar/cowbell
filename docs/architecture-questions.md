# Architecture — open questions log

A living checklist. Each item is something we do **not** yet know about the
TR-6S platform. Move items to `hardware.md` / `firmware-format.md` with evidence
as they get answered. Keep the question here (struck through) with a pointer, so
the reasoning trail survives.

## Compute chip

- [ ] **Exact part number** of the main compute chip. Is `ESC2` the real
      silicon, or a Roland label over a third-party SoC?
- [ ] **ESC2 vs BMC vs E4E** — which of these is a discrete part vs. a
      marketing name for a DSP block? (see `hardware.md`)
- [ ] **ISA / architecture** — ARM (Cortex-A/-M/-R?), a Tensilica/Xtensa-style
      DSP, SuperH legacy, or something custom?
- [ ] **Clock speed(s)** — core clock, DSP clock, memory clock.
- [ ] **On-chip vs external** boot ROM — is there an internal bootloader?

## Memory map

- [ ] How is the **64 MB NOR** presented — execute-in-place, or copied to SDRAM
      at boot?
- [ ] How is the **32 MB SDRAM** partitioned (code, heap, ACB voice state,
      audio buffers)?
- [ ] Are the two ESMT SDRAM devices a single 32-bit bus or two 16-bit banks?
- [ ] Physical **base addresses** of NOR, SDRAM, and MMIO peripherals.

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
