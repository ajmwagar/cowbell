# Architecture — open questions log

A living checklist. Each item is something we do **not** yet know about the
TR-6S platform. Move items to `hardware.md` / `firmware-format.md` with evidence
as they get answered. Keep the question here (struck through) with a pointer, so
the reasoning trail survives.

## Compute chip

- [x] ~~**ESC2 vs BMC vs E4E** — which is a discrete part vs. a marketing
      name?~~ → **Two confirmed discrete Roland SoCs: BMC and E4E.** BMC is the
      big-TR-box chip (`Roland BMC`, lot 5100440716, TR-8S teardown); E4E is the
      AIRA Compact chip (`Roland E4E`, lot 5100069694, Beat 8). BMC is NOT just
      ACB marketing. ESC2 unobserved in any photo we hold. See `hardware.md`.
- [x] ~~**Is the TR-6S main SoC E4E?**~~ → **No — almost certainly BMC.** The
      key-sharing sibling TR-8S is photographed as `Roland BMC`, and TR-6S↔TR-8S
      share the key (16 KB identical ciphertext run), load map, and 66k blocks →
      same main SoC. Strong inference via the *key-sharing* sibling. A first-party
      TR-6S board photo would make it airtight; still outstanding.
- [ ] **ISA / architecture of the BMC audio SoC** (TR-6S/TR-8S) — ARM
      (Cortex-A/-M/-R?), Tensilica/Xtensa DSP, SuperH, or custom? *Leading
      hypothesis: ARM,* unconfirmed because `App1_Main` is encrypted. Evidence:
      (a) load address `0x60000000` = canonical ARM external-NOR base (shared by
      BMC and E4E images alike — a Roland-wide convention); (b) the panel MCU is
      confirmed ARM (below). BMC and E4E are distinct chips, so their ISAs are
      not guaranteed identical — but both use the same memory-map convention.
      NOTE: T-8 (E4E) uses a *different key*, so ciphertext comparison cannot
      relate BMC and E4E code; only decryption can.
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
