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

## First-party TR-6S teardown (2026-09-01) — corrections to the above

The maintainer opened their own TR-6S and photographed the boards. This
**supersedes the TR-8S-footage guesses** for the TR-6S itself (full detail in
`hardware.md`):

- **NOR is on the TOP/component side**, right beside the BMC — *not* the reverse
  side the TR-8S footage implied. Package **confirmed TSOP-56** (`Spansion
  S29GL512S10TFI02`). So a top-side in-circuit clip or chip-off is viable
  without flipping/desoldering around the SoC.
- **Main SoC confirmed `Roland BMC`** on the actual TR-6S (date `2152`); boards
  silkscreened `DD001` (= TR-6S).
- **A microSD socket *is* present** on the DD001 main board — correcting the
  "TR-6S has none" note above (there's no *user-facing* slot, but the board has
  an internal socket). Possible easy data route; investigate.
- **Debug lead:** an `SW1` DIP switch silkscreened **`TMS`** (a JTAG signal)
  sits near the BMC — the highest-priority spot to probe per the checklist below.

## First-party recon checklist (mostly done — remaining probing)

Photos captured (2026-09-01): BMC neighborhood, main-board top face, jack board.
Still to do: **back face of the main board** (locate the 2nd SDRAM + any
reverse-side pads), and active probing of the `SW1`/`TMS` cluster.

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
