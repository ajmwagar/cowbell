# Architecture — open questions log

A living checklist. Each item is something we do **not** yet know about the
TR-6S platform. Move items to `hardware.md` / `firmware-format.md` with evidence
as they get answered. Keep the question here (struck through) with a pointer, so
the reasoning trail survives.

## Compute chip

- [ ] **Exact part number** of the main compute chip. Is `ESC2` the real
      silicon, or a Roland label over a third-party SoC?
- [x] ~~**ESC2 vs BMC vs E4E** — which is a discrete part vs. a marketing
      name?~~ → **E4E is a confirmed discrete Roland SoC** (BGA, marked
      `Roland E4E`, lot 5100069694, date 2304), seen on an AIRA Compact Beat 8.
      Whether the *TR-6S* main chip is E4E or ESC2 still needs a direct read of
      the TR-6S board. See `hardware.md`.
- [ ] **ISA / architecture of the E4E audio SoC** — ARM (Cortex-A/-M/-R?), a
      Tensilica/Xtensa DSP, SuperH legacy, or custom? *Leading hypothesis: ARM,*
      but still unconfirmed because the `App_Main`/`App1_Main` audio images are
      encrypted. Evidence: (a) load address `0x60000000` = canonical ARM
      external-NOR base — and this address is now tied directly to **confirmed
      E4E silicon**: the T-8 (whose main SoC is a photographed `Roland E4E`)
      loads its `App_Main` at `0x60000000`, identical to both TR boxes; (b) the
      panel MCU is now **confirmed ARM** (below).
- [ ] **Is the TR-6S main SoC actually an E4E?** Still unconfirmed — E4E is
      confirmed only on the T-8. But TR-6S and TR-8S share 66k code/ROM blocks
      (same DSP engine → same main-SoC ISA as each other), and all three
      products load `App_Main` at `0x60000000`. Consistent with a shared E4E
      family across the line; needs a direct read of the TR-6S board to close.
      NOTE: because the T-8 uses a *different key*, ciphertext comparison
      **cannot** tell us whether the T-8 runs the same E4E code as the TRs —
      only decryption can.
- [x] ~~**ISA of the panel/IO MCU**~~ → **ARM Cortex-M, confirmed from
      firmware.** The T-8 `App_Panel` image is plaintext, decodes as a valid
      Cortex-M vector table + Thumb code at base `0x08000000` (196 functions in
      Ghidra). Matches the STM32G0 chip photo. See `firmware-format.md`.
- [ ] **Clock speed(s)** — core clock, DSP clock, memory clock.
- [ ] **On-chip vs external** boot ROM — is there an internal bootloader?

## Memory map

- [ ] How is the **64 MB NOR** presented — execute-in-place, or copied to SDRAM
      at boot?
- [ ] How is the **32 MB SDRAM** partitioned (code, heap, ACB voice state,
      audio buffers)?
- [ ] Are the two ESMT SDRAM devices a single 32-bit bus or two 16-bit banks?
- [ ] Physical **base addresses** of NOR, SDRAM, and MMIO peripherals.
      *Partial:* the `App1_Main` header declares a load address of
      **`0x60000000`** for the main application image, and the init-parameter
      blob targets **`0x0033FFD0`**. Two points on the map, but which physical
      device each lands in is still unconfirmed. See `firmware-format.md`.
- [x] ~~**Endianness** of firmware header length/offset fields.~~ →
      **Little-endian**, confirmed across every header field in both payloads
      (`firmware-format.md`).

## Audio / ACB signal path

- [ ] Do **ACB voices** run on the main compute chip, or route through a
      **separate DSP coprocessor**?
- [ ] Where do the analog-circuit-behavior models actually execute — this
      determines whether PedalKernel-derived **WDF circuit models** could ever
      be hosted, and on what budget.
- [ ] Sample rate / block size of the internal audio engine.
- [ ] Codec/DAC part and its interface (I²S? TDM?).

## Update / boot security (feeds the phase gate)

- [x] ~~Is the distributed image **encrypted**?~~ → **Yes.** The `App1_Main`
      payload is encrypted under a **64-bit block cipher in ECB mode**
      (29.9% duplicate 8-byte blocks; AES, XOR, and compression all ruled out).
      Evidence in `firmware-format.md`.
- [x] ~~Is there a **checksum** field?~~ → **Yes**, `0x46DE65B3` at header
      `0x50`. Algorithm still unidentified — no standard algorithm matches the
      *ciphertext*, which suggests it covers the decrypted plaintext.
- [ ] Is the firmware image **signed**? Encryption is confirmed; signing is a
      separate property and remains unresolved. No distinct signature block is
      visible, but absence of evidence is not evidence of absence here.
- [ ] **What cipher, and where is the key?** The decryption routine and key live
      in the bootloader resident in NOR — not shipped in the update archive.
      This is now the single biggest blocker. **Upgraded leverage:** the TR-6S
      and TR-8S share one key (66,233 identical ciphertext blocks), so recovering
      it once decrypts both — and any TR-8S NOR dump is a known-plaintext oracle
      for the TR-6S shared regions. See `firmware-format.md` "KEY FINDING".
- [ ] Does the bootloader **verify before flashing**?
- [ ] Is there a **recovery / DFU** mode reachable without opening the unit?
- [ ] What is the **write path** to NOR — and is it safe to target? (This is
      the gate for any future flashing work; nothing in this repo writes to the
      device until it is answered.)

## Toolchain / next steps

- [ ] **Reference-platform route: the AIRA Compact Beat 8** (shares the E4E,
      confirmed via third-party photos; **we do not own one**). Split by cost:
      - *Free, do now:* pull Roland's Beat 8 / AIRA Compact firmware update and
        run it through fw-extract/fw-analyze. Same tar+`App1` container? Same
        64-bit ECB cipher, or plaintext? A cheaper product may be unprotected or
        share a recoverable key — needs no hardware.
      - *Requires acquiring a unit:* identify the `251/P2368` SOIC-8 beside the
        E4E and, if SPI NOR, in-circuit clip-dump it for E4E native code → ISA.
        Blocked on buying a Beat 8; not actionable today.
- [ ] **Dump the S29GL512S NOR in hardware.** The guaranteed path for the TR-6S
      itself, but now a *fallback* behind the Beat 8 route: the update archive
      contains no plaintext code, so ISA/memory-map/boot-flow all depend on
      reading real code off silicon. 56-ball TSOP — in-circuit read or chip-off.
- [ ] Identify a JTAG/SWD or UART debug pad on the PCB.
- [ ] Confirm the compute chip's ISA so a disassembler target can be chosen.
      **Not answerable from the update image** — the only code it contains is
      behind the cipher.
- [ ] Locate sibling images. The `App1_Main` name implies an `App2`/bootloader
      image in the same 96-byte-header format; other Roland `dd001`-family
      updates may ship them.
