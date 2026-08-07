# Firmware format — `.bin` structure & update mechanism

Findings on how the TR-6S firmware update is packaged and what (if anything)
protects its integrity. All observations are from the maintainer's own updater
downloads and hardware; no Roland binaries are committed to this repo.

## Update delivery (what the user runs)

Roland distributes the updater as a platform-specific installer that, when run,
places a firmware file onto the unit — typically over USB mass-storage or a
vendor updater app.

- **Windows:** self-extracting `.exe` (InstallShield/NSIS lineage) or `.msi`.
- **macOS:** `.dmg` containing an app, or a flat `.pkg` (`xar!` archive).

`fw-extract identify` sniffs which of these you have; `fw-extract unpack` shells
out to the matching tool (`7z` / `msiextract` / `xar`); `fw-extract carve` then
locates the dense payload blob inside the unpacked tree.

## Container — RESOLVED (sys v2.00)

`TR6S_UP.bin` is **not a firmware image**. Despite the extension it is a
**GNU tar archive** (3,911,680 B) holding two members:

| Member | Size | Nature |
| ------ | ---- | ------ |
| `./_tmp/dd001_m0c0a_up.bin` | 2,529,328 B | Main firmware — **encrypted** |
| `./_tmp/dd001_init_param.bin` | 1,376,256 B | Init/parameter data — **plaintext** |

`dd001` is the Roland internal model code for the TR-6S; the `_tmp/` prefix
suggests the archive is assembled on a build host and unrolled to scratch space
by the updater.

### Cross-platform: the container format is a Roland-wide convention

Confirmed identical across three products (all `tar` + per-image 96-byte
`App*` header):

| Product | Update file | Model code | Members |
| ------- | ----------- | ---------- | ------- |
| **TR-6S** | `TR6S_UP.bin` | `dd001` | `App1_Main` (enc) + `init_param` |
| **TR-8S** | `TR8S_UP.bin` | `rpg42` | `App1_Main` (enc) + `init_param` |
| **T-8 / Beat 8** | `T8_UPD.BIN` | `DD010` | `App_Main` (enc) + `App_Panel` (**plaintext ARM**) |

Naming note: the big boxes use `App1_Main` (the `1` implying multiple app
processors); the T-8 uses `App_Main` + a separate `App_Panel`. The TR-6S/TR-8S
ship no panel sub-image in the update — their panel MCU is updated by another
path or bundled in the main image.

## Payload structure — RESOLVED (`dd001_m0c0a_up.bin`)

| Range | Size | Contents |
| ----- | ---- | -------- |
| `0x000000`–`0x00005F` | 96 B | Plaintext header |
| `0x000060`–`0x2697EF` | 2,529,168 B | Encrypted payload |
| `0x2697F0`–EOF | 64 B | Plaintext trailer |

Header fields (little-endian throughout — answers the endianness question):

| Offset | Value | Meaning |
| ------ | ----- | ------- |
| `0x00` | `"App1_Main"` | Image name. The `App1` prefix implies sibling images (`App2`? a bootloader image?) exist in the same format. |
| `0x10` | `"----/--/-- --:--"` | Build-date field, deliberately blanked |
| `0x20` | `"0.010001"` | Format/version string |
| `0x28` | `0x60000000` | **Load address** |
| `0x30` | `0x00060040` | Section descriptor (offset `0x40`, flags `0x0006`?) |
| `0x40` | `0x00060060` | Section descriptor (offset `0x60` = payload start) |
| `0x44` | `0x00269790` | **Payload length** (2,529,168 — exact, `0x60 + len` = `0x2697F0`) |
| `0x48` | `0x60000000` | Load address (repeat) |
| `0x50` | `0x46DE65B3` | **Checksum** (algorithm unidentified — see below) |
| `0x54` | `0xB6020001` | Flags |

Trailer (last 64 B, plaintext ASCII):

```
Roland DD001_C0A   VER.2.00   BLD.0C1D   Commit:c1dc3-mod
```

A VCS commit hash in shipping firmware is a useful build-provenance marker for
correlating future image revisions.

## Encryption — RESOLVED: 64-bit block cipher, ECB mode

The payload is **encrypted, not compressed**. Evidence:

1. **Entropy 7.982/8.0** overall, essentially flat across the image.
2. **Simple XOR is ruled out.** XOR-ing the payload against the most frequent
   8-byte block at all 8 alignments leaves entropy at 7.997 — no alignment
   recovers structure.
3. **ECB signature at 8-byte granularity.** Duplicate-block analysis:

   | Block size | Total blocks | Unique | Duplicate instances |
   | ---------- | ------------ | ------ | ------------------- |
   | 8 bytes | 316,146 | 221,652 | 94,494 (**29.89%**) |
   | 16 bytes | 158,073 | 130,835 | 27,238 (17.23%) |

   The duplicate rate *falls* at 16 bytes and the top 16-byte patterns are
   merely concatenated pairs of the 8-byte ones — so the fundamental cipher
   block is **8 bytes**. This **rules out AES** (128-bit block).
4. The single block `6B4C9A852C732831` occurs **8,420 times** — the classic
   `E(K, constant)` leak of ECB mode encrypting erased/padded flash regions.
5. Payload length is exactly divisible by 8.
6. Compression is independently excluded: an LZ-family compressor would collapse
   those 8,420 identical blocks, and no known compression magic is present.

Candidate algorithms (all 64-bit block): **DES, 3DES, Blowfish, TEA/XTEA,
CAST-128, IDEA**, or a custom Feistel construction.

**Consequence:** there is no plaintext code anywhere in this file, at any
offset, under any processor spec. Disassembly of the update image is impossible
until the key is recovered, and the key + decryption routine live in the
**bootloader resident in NOR flash** — which is *not* shipped in this archive.

### KEY FINDING: TR-6S and TR-8S share the encryption key (2026-08-07)

Cross-correlating the encrypted `App1_Main` payloads of the TR-6S (`dd001`) and
TR-8S (`rpg42`) at 8-byte block granularity:

- TR-6S unique blocks: 221,660 · TR-8S unique blocks: 224,927
- **Shared identical ciphertext blocks: 66,233**
- The dominant fill block `6B4C9A852C732831` is present in **both** (TR-6S
  ×8,420; TR-8S ×10,306).

66k identical 8-byte ciphertext blocks across two independent images is only
possible if identical plaintext encrypts to identical ciphertext under the
**same key** — i.e. TR-6S and TR-8S use **one shared platform-family key** in
ECB mode, not per-device or per-model keys.

**Positional confirmation (hardens "same key" to certainty).** Comparing the
two ciphertexts at the *same* offset (not just as sets):

- 4,359 blocks (1.4%) are byte-identical at the same offset, and one run is
  **15,936 bytes of contiguous identical ciphertext at offset `0x252FB8`**
  (plus ~760 B runs near `0x1D1318`). A 16 KB identical ciphertext run at a
  fixed offset cannot occur by chance → **same key is now proven, not inferred.**
- The *low* positional overlap (1.4%) alongside the *high* set overlap means the
  shared content sits at **different offsets** in each image — sibling builds
  that share code/data but are laid out differently (different feature sets,
  code sizes), exactly as expected for two products on one platform.
- Plaintext `init_param` positional byte-equality is similar: 1.8%.

Practical implications:

1. **Recover the key once, decrypt both** (and likely other RPG-/DD-family units).
2. The shared blocks map the boundary between shared-platform content and
   device-specific content **without any decryption** — a free structural map.
3. Any plaintext obtained for one unit (e.g. a future NOR dump) is a
   **known-plaintext oracle** for the shared blocks of the other.

**What this establishes about the SoC — the main chip is BMC.** The shared
platform (key, container, CRC-32, load map `0x60000000`,
`init_param`@`0x0033FFD0`, substantial shared content) means TR-6S and TR-8S
almost certainly run the **same main SoC**. A YouTube teardown of the TR-8S
shows that SoC directly: **`Roland BMC`** (lot 5100440716). Because it is the
*key-sharing* sibling, this pins the **TR-6S main chip to BMC** — a stronger
chain than the earlier E4E lead (which came from the different-key T-8). Caveat
for rigor: ECB block-sharing alone can't separate shared *code* from shared
ISA-independent *data*, and we have no first-party TR-6S photo — but the
key-sharing + TR-8S BMC photo make BMC the well-supported answer. See
`hardware.md`.

The **T-8 (`DD010`) `App_Main` uses a DIFFERENT key** — 0 shared blocks with
either TR box, and 0% ECB duplication. This is now explained: the T-8 main SoC
is **E4E** (Beat 8 photo), a *different chip* from the TR boxes' BMC, hence a
different bootloader/key. So the AIRA Compact line is neither a key shortcut nor
representative of the TR-6S silicon — it is a separate part, useful only as a
generic ARM/toolchain reference (see below).

### T-8 `App_Panel` — PLAINTEXT ARM Cortex-M (the ISA is confirmed)

The T-8 update carries a second image, `App_Panel`, that is **not encrypted**
(entropy 6.72, 2.7% ECB dup = zero-padding only). It is the panel/IO MCU
firmware and it is textbook **ARM Cortex-M**:

- Payload `[0x40:]` is a Cortex-M vector table at load base `0x08000000`:
  initial SP `0x20001DE8` (SRAM), Reset `0x080000D9`, NMI `0x08002933`,
  HardFault `0x080028ED` — all Thumb (bit0 set). Flash `0x08000000` + SRAM
  `0x20000000` is the canonical STM32 map, matching the STM32G0 chip photo.
- Reset stub decodes cleanly: `ldr r0,[SystemInit]; blx r0; ldr r0,[start]; bx r0`.
- Imported to the Ghidra project as `ARM:LE:32:Cortex` (base `0x08000000`);
  auto-analysis found **196 functions** — the image is correctly based and real.

This **confirms the ARM ISA** for the Roland panel MCU directly from firmware
(no longer just an inference from the chip photo or the `0x60000000` address).
It does *not* prove the audio SoCs (BMC on the TRs, E4E on the T-8) are ARM —
those images are still encrypted — but it shows Roland uses ARM on these
platforms and strengthens the working hypothesis that the audio SoCs are ARM too.

Carved image: `firmwares/t8_sys_v102/extracted/DD010_pnl_ARM.bin` (gitignored).

#### App_Panel reverse-engineering results (2026-08-07)

Analysed in Ghidra (`DD010_pnl_ARM.bin`, 196 functions). Findings:

- **Toolchain: STM32Cube HAL + GCC/newlib.** `start` is the textbook GCC crt0
  (data/bss init → `main`); the USART interrupt handler is byte-for-byte the HAL
  `HAL_UART_IRQHandler` (operates on a HAL UART handle struct — reads `ISR` at
  reg offset `0x1C`, `CR1`, `CR3`; PE/FE/NE/ORE error handling; Rx/Tx ISR
  callbacks). So Roland builds these panels with CubeMX-generated scaffolding.
- **Peripheral map** (from the IRQ vector table, STM32G0x1 layout — real handlers
  only): EXTI4_15 (GPIO events), DMA1_Ch1, ADC, TIM1, TIM3, TIM14, USART1. SPI
  slots are the default handler → **the inter-processor link is UART, not SPI.**
- **Panel → BMC protocol = MIDI (or MIDI-derived) bytes over USART1**, DMA'd out
  (matches the DMA1_Ch1 handler) through a 128-byte ping-pong buffer
  (`PanelTxMidiMessage(buf,len)` @ `0x08003EC8`). `main` is a poll→serialize→TX
  super-loop emitting 3-byte packets. Status/opcode constants near `0x08004C90`:
  - `0x90`/`0x91`/`0x93` = Note On, ch 0/1/3 (button/pad groups, press+velocity)
  - `0x80` = Note Off, ch 0
  - `0xFE` = Active Sensing (1-byte periodic keepalive)
  - encoder deltas clamped to signed 7-bit (`−0x40..0x3F`) before send.
- **Division of labour confirmed:** this MCU is only the front-panel I/O
  concentrator (matrix scan via the `74HC138`, LED drive, encoder/analog reads);
  the **BMC is the master** (sequencer, ACB engine, audio/USB/MIDI/storage) and
  drives the panel over the same UART.

Why this matters for the project: alt-firmware that replaces or augments the
BMC would need to speak this UART-MIDI panel protocol to drive the UI — and it
is now a documented, well-understood interface. Caveat: this is the **T-8's**
panel firmware; the TR-6S panel is the same design family but a distinct build
(the TR update ships no panel sub-image, so the TR panel is flashed by another
path — likely pushed by the BMC over the same UART).

## Integrity / signing

- [x] Is there a **checksum/CRC** field? — **Yes**, at header `0x50` for
      `App1_Main` (9-char name) / `0x2C` for the compact `App*` (8-char name)
      layout. TR-6S `0x46DE65B3`, TR-8S `0xE63B9547`, T-8 App_Panel `0x53C790FB`,
      T-8 App_Main `0x0BF49F16`.
- [x] **Algorithm — RESOLVED: standard CRC-32** (poly `0x04C11DB7`, reflected
      in/out, init & xorout `0xFFFFFFFF` — i.e. zlib `crc32`), computed over the
      image **body** (payload after the header, length = the header length
      field). Recovered by known-plaintext: CRC-32 over the plaintext `App_Panel`
      body equals its `0x2C` field **exactly** (`0x53C790FB`).
- [x] **Checksum is over PLAINTEXT, not ciphertext — confirmed.** CRC-32 over
      the *ciphertext* body of the encrypted images matches no header field, at
      any range (TR-6S body → `0x25F6CEE3` ≠ `0x46DE65B3`; T-8 App_Main body →
      `0x015424F4` ≠ `0x0BF49F16`). So the bootloader flow is **decrypt →
      CRC-32 → compare**. Practical payoff: this is a **decryption oracle** —
      the correct key/cipher is the one whose output CRC-32 equals the header
      field, so key-recovery attempts self-validate with no plaintext eyeballing.
- [ ] Is the image **cryptographically signed** (RSA/ECDSA)? No distinct
      fixed-size signature block is visible; the plaintext trailer is build
      metadata, not a signature. Encryption is confirmed, signing is not — and
      the two are independent. Unresolved.
- [ ] Does the bootloader **verify** before applying, or apply blindly?
- [ ] Is there **anti-rollback** (monotonic version enforcement)?

Until signing and the update flow are understood, this project stays strictly
read-only — see the phase gate in the README.

## `dd001_init_param.bin` — RESOLVED, and unencrypted

Entropy 6.71 with legible ASCII (`INIT`, `TR6S`, `pSYS `, `USERS01`). This is
the one component readable today.

| Offset | Field | Value |
| ------ | ----- | ----- |
| `0x00` | `magic[4]` | `"INIT"` |
| `0x04` | `total_size` | `0x00150000` (1,376,256) — exactly the file size |
| `0x08` | `section_size[3]` | `0x0006B858` ×3 (all equal) |
| `0x14` | `load_addr[3]` | `0x0033FFD0` ×3 (all equal) |
| `0x20` | `reserved[4]` | zero |
| `0x30` | payload | 3 × `0x6B858` sections |
| `0x142938` | tail | 54,984 B zero pad + three LE `0x0006B858` footer words |

**The three sections are byte-identical** (verified by SHA-256:
`76a160f2aeb0347d001c1dfdad1a04f1…`). Identical sizes, identical target
addresses, identical content ⇒ this is **triple redundancy** for
wear-levelling / integrity fallback, *not* three distinct parameter banks.

Section content: ASCII tags embedded in binary tables, with prominent runs of
LE `uint16` ascending at a constant stride of `0x12` (e.g. `df11 df23 df35
df47`), each run repeating 11× per copy — consistent with indexed
parameter/offset tables.

Imported into the Ghidra project (`tr6s_ghidra`) as **`DATA:LE:64:default`**
with the header struct `TR6S_InitParamHeader` applied and section boundaries
labelled. The DATA language is deliberate: the file holds no code, and the
platform ISA remains unknown — **the language ID is not an architecture claim.**

## Method log

**2026-08-07 — container + crypto characterisation (sys v2.00).**
Identified `TR6S_UP.bin` as tar, not a raw image. Parsed the `App1_Main`
header, establishing little-endian fields and load address `0x60000000`.
Established via duplicate-block analysis that the payload is ECB-mode
encrypted under a 64-bit block cipher, ruling out XOR obfuscation, AES, and
compression. Fully mapped `dd001_init_param.bin` and confirmed its triple
redundancy. Checksum algorithm remains unidentified against ciphertext,
suggesting it is computed over plaintext.

**2026-08-07 — cross-platform comparison (T-8 v1.02, TR-8S v3.00).**
Added two sibling images. Established that (1) TR-6S and TR-8S **share the
encryption key** (66,233 identical ciphertext blocks) — recover once, decrypt
both; (2) the T-8 uses a different key; (3) the T-8 `App_Panel` is **plaintext
ARM Cortex-M**, imported to Ghidra (196 functions), **confirming the panel-MCU
ISA** and the STM32-class memory map. The container format is a Roland-wide
tar+`App*`-header convention.

_Next:_ two live fronts. (a) **Key recovery** — still requires a NOR/bootloader
dump of a TR-6S *or TR-8S* (either works, shared key), so the hardware dump is
no longer strictly gated on the TR-6S. (b) **Reverse the plaintext T-8
`App_Panel`** for free to learn Roland's toolchain, checksum routine, and
peripheral conventions — the checksum algorithm found there may match the
`0x50` header field across all images.
