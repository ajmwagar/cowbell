# Prior art — Roland RE landscape & hardware-access recon

Survey of what the public community has (and hasn't) done on the modern Roland
ACB / ZEN-Core platform, plus the practical recon checklist for finding a debug
port on the hardware. Captured 2026-08-07 so we don't re-run the searches.

## Bottom line

**No public prior art exists for a debug port (JTAG/SWD/UART) or a firmware
decryption on the modern Roland ACB/ZEN-Core platform** — not the TR-8S, TR-6S,
MC-707/101, Fantom, or Jupiter-X. Searched specifically for teardown pinouts,
serial-console boot logs, chip dumps, and firmware-decrypt repos; none found.

What *does* exist is entirely at the **software/protocol layer, above firmware**:

- [mugenkidou/AIRA_Modular_Effects](https://github.com/mugenkidou/AIRA_Modular_Effects)
  — verified: **SysEx/MIDI protocol documentation only.** No JTAG/UART/chip
  dump/firmware decryption.
- [Awesome-MC-707](https://github.com/ricardofeynman/Awesome-MC-707) and various
  editors — tips, preset/pattern tooling, MIDI. Same layer.
- [Roland Clan "firmware analysis" thread](https://forums.rolandclan.com/viewtopic.php?f=55&t=54291)
  — about *EXP expansion formats*; users report even the **format** RE stalled
  ("no one had reverse-engineered the EXP format") and voice concern Roland
  might act on RE.

### Cipher-identification OSINT sweep (2026-08-09) — nothing on the crypto

Searched specifically for the *cipher/algorithm* (not just debug ports): TR-8S /
ZEN-Core / BMC firmware **encryption**, `App1_Main`, and firmware-decrypt efforts
on the sibling grooveboxes (MC-707/101, Fantom, Jupiter-X). **No public work
identifies the cipher, key, or decryption of any BMC/ZEN-Core firmware image.**
So the algorithm cannot be identified by OSINT — it stays an unknown until the
NOR/bootloader is dumped (`cowbell-8o5`) and read with FindCrypt/capa/signsrch.
This *reinforces* the plan; it does not change it.

**Adjacent find — sibling data-format tooling (not firmware).**
[`DrKnackerator/RolandZenDecodeXML`](https://github.com/DrKnackerator/RolandZenDecodeXML)
(+ "ZenInspector") decode ZEN-Core **user-data/project** files — Jupiter-X/Xm,
Fantom, Juno-X, Zenology, and MC-707/101 `PRJ`/`SVZ` (extracting ZCore tone
data). No encryption/cipher involved; it parses Roland's **editor XML schema into
byte offsets + SysEx addresses** — the *same method* this project uses (our
`Script.xml` oracle → `tr-format` offsets, `device-sysex.md` → `tr-sysex`
addresses). Useful as **corroboration and a cross-check** for the plaintext
data-format work (do ZEN-Core's `SVZ`/`PRJ` tone/kit structures echo the TR
backup's `TONE`/`KIT`?), and irrelevant to the crypto. A lead to evaluate for
`tr-format`/`tr-sysex`, not yet inspected in depth.

## Why the hardware layer is unexplored

1. **Roland channels modding into official tools** — the
   [AIRA Modular Customizer](https://www.roland.com/global/products/aira_modular_customizer/),
   ZEN-Core/[Zenology](https://en.wikipedia.org/wiki/Roland_Zenology), and older
   System-1/8 "plug-outs." Sanctioned customization drains the hacking pressure.
2. **The HW-hacking community skews cheap-and-networked** (routers/IoT/consoles).
   Boutique $500–700 synths are niche and expensive to sacrifice, so almost
   nobody has opened one with a logic analyzer and published.

## What this means for cowbell

- **No map to copy.** Going the hardware route is *original* reverse-engineering,
  not a recipe. Higher effort; risk is on us.
- **"No writeup" != "locked."** It almost certainly means *nobody has tried and
  published*, not that Roland fused off debug. The port could be wide open; we
  won't know until we probe. We would be the first to check.
- **cowbell is ahead of the public record.** As far as the open web shows, our
  findings — tar container, 64-bit ECB cipher, shared TR-6S/TR-8S key,
  CRC-32-over-plaintext, BMC vs E4E SoC ID, MIDI-over-UART panel protocol — are
  further into the TR-6S/8S than anything published.

Transferable resources are generic methodology only:
[UART root shell](https://riverloopsecurity.com/blog/2020/01/hw-101-uart/),
[firmware RE technique](https://github.com/swisskyrepo/HardwareAllTheThings),
[Hackaday Roland tag](https://hackaday.com/tag/roland/).

## Board recon (from TR-8S teardown footage — reference, not first-party)

Confirmed on the TR-8S CPU board (YouTube teardown; **not** the maintainer's
hardware, and the TR-6S board differs):

- Main SoC = **Roland BMC** (large BGA), lot 5100440716.
- **ISSI SDRAM** adjacent (working memory).
- SD-card slot present (TR-8S only; the TR-6S has none — a known layout diff).
- Panel/FFC + JST connectors along the top edge; audio jacks on the lower board.
- The **S29GL512S NOR was not visible on the CPU-board top face** → most likely
  on the **reverse side**. Flash package (TSOP-56 vs BGA-64) still unconfirmed.

At teardown-footage resolution, **individual debug pads (JTAG/SWD/UART) cannot
be resolved.** Debug-pad identification must be done first-party on the TR-6S.

## First-party recon checklist (do this when opening the TR-6S)

Photos to capture:

1. **Close-up of the BMC neighborhood** (within ~1 inch of the SoC) — debug pads
   almost always cluster here. Highest-value shot.
2. **Both faces of the CPU board** — the NOR is likely on the back; debug pads
   can be on either side.
3. Any **unpopulated header** (row of 4–10 empty plated holes/pads), **test-point
   cluster** (small round pads), or silkscreen reading `TP` / `JP` / `CN` /
   `SWD` / `JTAG`, or a lone inline group of ~4 pads (classic UART).

Finding pads with a multimeter / logic analyzer (beats eyeballing photos):

- **UART:** a group of 3–4 pads near the SoC. Powered on, one sits ~3.3 V (VCC),
  one at 0 V (GND), and **TX flickers during boot** (a cheap logic analyzer or
  scope catches the boot chatter). A USB-UART adapter then reads the console.
- **SWD** (if the BMC is ARM, our leading hypothesis): a SWDIO/SWCLK pair, often
  beside GND/VCC — frequently a 4–5-pad inline group. An ST-Link/J-Link attaches.
- **Orient** by continuity-checking candidate GND pads against chassis ground.

Goal of the debug route: attach, halt the CPU after boot (controllers already
initialized), and read the NOR at `0x60000000` (and SDRAM) straight out — **no
chip-off**. This is the cheap, non-destructive path to try before the NOR dump
(`cowbell-8o5`); chip-off is the fallback if debug is locked/absent.

## Sources

- <https://github.com/mugenkidou/AIRA_Modular_Effects>
- <https://github.com/ricardofeynman/Awesome-MC-707>
- <https://forums.rolandclan.com/viewtopic.php?f=55&t=54291>
- <https://www.roland.com/global/products/aira_modular_customizer/>
- <https://en.wikipedia.org/wiki/Roland_Zenology>
- <https://riverloopsecurity.com/blog/2020/01/hw-101-uart/>
- <https://github.com/swisskyrepo/HardwareAllTheThings>
- <https://hackaday.com/tag/roland/>
