# cowbell

Right-to-repair / alternative-firmware **research** for the Roland **TR-6S**
drum machine.

This is early-stage reverse engineering, **not** a working firmware. The goal is
to understand the platform well enough to eventually explore running original
DSP code — [PedalKernel](https://github.com/) -derived **WDF (Wave Digital
Filter) circuit models** — as alternative or supplementary firmware on hardware
the maintainer owns.

> **Status:** tooling for firmware *analysis* only. No flashing / write-to-device
> code exists, and none will be added until the update mechanism is confirmed
> safe to target (see the phase gate below).

## Scope & legal / ethical position

- This is **personal right-to-repair research** on a TR-6S the maintainer owns.
- **No Roland firmware, ROMs, or extracted proprietary binaries are distributed**
  in this repository, ever. The tooling operates on images the user supplies
  locally; `.gitignore` blocks `firmware/` and `dumps/` so nothing proprietary
  is committed by accident.
- The aim is **interoperability and understanding** of a platform the maintainer
  owns — not piracy, not redistribution, not circumventing protections to copy
  Roland's content.
- If you do not own the hardware, or your jurisdiction treats this differently,
  this is not for you.

## Repository layout

```
cowbell/
├── crates/
│   ├── fw-analyze/   # CLI: entropy, magic/header inspection, hexdump,
│   │                 #      checksum brute-forcing, ECB block-diff, binwalk wrapper
│   ├── fw-extract/   # CLI: identify + unpack Roland Win/Mac/tar installers,
│   │                 #      carve out the raw firmware payload
│   ├── tr-format/    # lib+CLI: lossless reader/writer for TR-6S/8S user data
│   │                 #      (backup container, kits, patterns) — plaintext only
│   ├── tr-studio/    # lib+CLI: ergonomic high-level API over tr-format
│   │                 #      (step grids, named voices, builders)
│   └── tr-sysex/     # lib+CLI: Roland RQ1/DT1 SysEx protocol (read/write
│                     #      messages) — message construction only, no MIDI I/O
└── docs/             # research notes (markdown, not code)
    ├── hardware.md               # chip ID findings (BMC vs E4E; RAM/flash)
    ├── firmware-format.md        # .bin structure, tar container, encryption, key
    ├── tr-format.md              # user-data (backup/kit/pattern) format
    ├── sysex.md                  # Roland RQ1/DT1 protocol notes
    ├── prior-art.md              # RE landscape + hardware-access recon
    └── architecture-questions.md # open-questions log
```

Two tracks live here: **firmware RE** (`fw-*`, encrypted `App1_Main`, gated on a
NOR dump) and **user-data interop** (`tr-*`, plaintext backups/patterns/SysEx —
a FOSS alternative to Roland's TR-EDITOR). Firmware code for the target itself
comes **later**, once the compute chip and its ISA are identified (tracked in
`docs/architecture-questions.md`).

## Build

```sh
cargo build            # both CLIs
cargo test             # unit tests (checksum vectors, etc.)
```

## The two tools

### `fw-extract` — get the payload out of an installer

```sh
fw-extract identify roland_tr6s_updater.exe      # sniff container type
fw-extract unpack   roland_tr6s_updater.exe --out extracted/
fw-extract carve    extracted/                   # rank candidate firmware blobs
```

It shells out to `7z` / `msiextract` / `xar` for the container it detects, then
carves by size + entropy heuristics to find the dense payload.

### `fw-analyze` — poke at the raw image

```sh
fw-analyze entropy  firmware/tr6s.bin            # find compressed/encrypted regions
fw-analyze inspect  firmware/tr6s.bin --scan     # magic bytes at 0 and embedded
fw-analyze hexdump  firmware/tr6s.bin --offset 0 --len 256
fw-analyze checksum firmware/tr6s.bin --expect 0x1a2b3c4d   # brute-force algo
fw-analyze diff     firmware/rev_a.bin firmware/rev_b.bin   # what changed
fw-analyze binwalk  firmware/tr6s.bin -- -e      # wrapper over binwalk if installed
```

Simple, deterministic work (entropy, hexdump, CRC/checksum, diff) is native
Rust; heavy signature carving delegates to `binwalk` when it is on `PATH`.

## Common loop

`just` recipes wrap the extract → analyze → note-findings loop; run `just` with
no arguments to list them. See [`justfile`](./justfile).

## Phase gate — why there's no flashing code

Writing to the device is deliberately out of scope until **all** of these are
answered (see `docs/`):

1. Firmware payload structure understood (header, sections, load address).
2. Presence/absence of **checksum and signing** determined.
3. Bootloader **verify-before-flash** behavior known.
4. A safe **recovery/DFU** path confirmed to exist.

Until then, cowbell only reads.

## License

[MIT](./LICENSE).
