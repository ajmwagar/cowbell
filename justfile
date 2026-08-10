# cowbell — common task runner for the extract → analyze → note loop.
# Requires `just` (https://github.com/casey/just). Run `just` to list recipes.
#
# Nothing here writes to a device; every recipe is read-only analysis.

# Default: show available recipes.
default:
    @just --list

# --- build / check -----------------------------------------------------------

# Build both CLIs (debug).
build:
    cargo build

# Build optimized release binaries.
release:
    cargo build --release

# Run unit tests (checksum vectors, etc.).
test:
    cargo test

# Format + lint gate.
check:
    cargo fmt --all -- --check
    cargo clippy --all-targets -- -D warnings

# Format the workspace.
fmt:
    cargo fmt --all

# --- the research loop -------------------------------------------------------
# Typical flow:
#   just identify path/to/updater.exe
#   just unpack   path/to/updater.exe
#   just carve    extracted
#   just analyze  firmware/tr6s.bin

# Sniff an installer's container type.
identify package:
    cargo run -q -p fw-extract -- identify {{package}}

# Unpack a Roland installer into ./extracted (override with out=...).
unpack package out="extracted":
    cargo run -q -p fw-extract -- unpack {{package}} --out {{out}}

# Rank candidate firmware payloads in an unpacked tree or blob.
carve path:
    cargo run -q -p fw-extract -- carve {{path}}

# Quick-look analysis pass over a raw image: entropy + magic scan + head dump.
analyze image:
    cargo run -q -p fw-analyze -- entropy {{image}}
    cargo run -q -p fw-analyze -- inspect {{image}} --scan
    cargo run -q -p fw-analyze -- hexdump {{image}} --len 256

# Individual analysis passes.
entropy image:
    cargo run -q -p fw-analyze -- entropy {{image}}

inspect image:
    cargo run -q -p fw-analyze -- inspect {{image}} --scan

hexdump image len="256":
    cargo run -q -p fw-analyze -- hexdump {{image}} --len {{len}}

# Diff two firmware revisions.
diff a b:
    cargo run -q -p fw-analyze -- diff {{a}} {{b}}

# Brute-force which checksum algorithm reproduces an expected value.
checksum image expect:
    cargo run -q -p fw-analyze -- checksum {{image}} --expect {{expect}}

# --- SysEx (RQ1/DT1) — message construction only, never sent to a device ------

# Build an RQ1 data-request (read) message; ADDR may be `kit`/`pattern` or hex.
sysex-rq1 addr size:
    cargo run -q -p tr-sysex -- rq1 {{addr}} --size {{size}}

# Parse a Roland SysEx byte string (hex, or @file) and verify its checksum.
sysex-parse input:
    cargo run -q -p tr-sysex -- parse "{{input}}"

# Compute the Roland checksum over an address+data run of hex bytes.
sysex-checksum bytes:
    cargo run -q -p tr-sysex -- checksum "{{bytes}}"

# Open the research notes.
notes:
    @echo "docs/hardware.md docs/firmware-format.md docs/tr-format.md docs/sysex.md docs/prior-art.md docs/architecture-questions.md"
