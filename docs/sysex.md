# SysEx — Roland RQ1/DT1 protocol (TR-6S / TR-8S)

Notes on Roland's MIDI System-Exclusive read/write protocol for the TR devices,
and the [`tr-sysex`](../crates/tr-sysex) crate that implements it. This is a
**software/protocol-layer** track (above firmware) — no encryption, no Roland
IP; just the message format an editor uses to talk to a device the user owns.

Why it matters: **RQ1 is a read primitive over USB.** If the device answers RQ1
requests, large parts of its parameter memory can be read **without opening the
unit** — a cheap, non-destructive complement to the NOR-dump hardware route
(`cowbell-8o5`). It also gives `tr-studio` a path to sync directly with the
device later, instead of only editing SD-card backups.

## Where this comes from

TR Editor (Roland's official Mac/Win editor, a JUCE C++ app analyzed locally for
interoperability — not committed) exposes these senders in its symbols:

- `FKoaSendRq1` — send an **RQ1** (Request data 1).
- `FKoaSendDt1` — send a **DT1** (Data set 1).
- `FKoaRequestRq1Dt1` — the request/response round-trip (send RQ1, await DT1).
- `ML::CRolandMessage::CheckSum` — the checksum routine (sum, negate, `& 0x7F`).

RQ1/DT1 are Roland's long-standing "universal" SysEx commands (same shape used
since the GS era), so the *framing and checksum are known with confidence*; only
the device-specific fields (model id, address map) need recovery.

## Message format

```text
F0 41 <dev> <model...> <cmd> <addr(4)> <body...> <checksum> F7
```

| Field | Bytes | Notes |
| ----- | ----- | ----- |
| `F0` | 1 | SysEx start |
| `41` | 1 | Roland manufacturer id |
| `dev` | 1 | device id (`0x10` default; `0x00..=0x1F`) |
| `model` | 1 or 4 | model id — **UNCONFIRMED for TR-6S** (see below) |
| `cmd` | 1 | **RQ1 = `0x11`**, **DT1 = `0x12`** |
| `addr` | 4 | target address, each byte 7-bit (`0x00..=0x7F`) |
| `body` | n | **RQ1:** 4-byte size to read · **DT1:** the data to write |
| `checksum` | 1 | Roland checksum over `addr + body` |
| `F7` | 1 | SysEx end |

**Checksum.** Sum the address+body bytes; the checksum is the value making the
total a multiple of 128: `(128 - (sum % 128)) % 128`. Verified against Roland's
own published worked example (address `40 00 7F`, data `00` → checksum `41`) in
the crate's unit tests.

**Everything is 7-bit.** MIDI data bytes are `0x00..=0x7F`, so addresses,
sizes, and parameter values are all carried 7 bits at a time. This is exactly
why `tr-format`'s `Script.xml` value-sizing rule is "7-bit-safe"
(`⌈bits(range_max) / 7⌉`): a 0–1023 field needs 2 bytes over MIDI, a 16-bit mask
needs 3. `tr-sysex` reuses `tr_format::schema_value_size` so the two crates
agree on how a value spans the wire — a field read via RQ1 unpacks with the same
width tr-format uses to locate it in a backup.

## Address space

The section base addresses are the same ones `tr-format` reads from
`Script.xml`, so device memory and the backup file share one coordinate system:

| Section | Base address | Status |
| ------- | ------------ | ------ |
| Kit | `03 00 00 00` | base confirmed (`Script.xml`) — exposed as `addr::KIT` |
| Pattern | `04 00 00 00` | base confirmed (`Script.xml`) — exposed as `addr::PATTERN` |
| SYS / TONE / FX | — | not yet mapped to SysEx addresses |

Per-field offsets within a section come from the same schema; callers add them
with `Address::offset` (which carries at 128, not 256, per Roland's 7-bit
addressing).

## Open questions (do NOT guess these)

1. **TR-6S/TR-8S model id bytes.** The crate ships `ModelId::TR6S_PLACEHOLDER`
   (all-zero, obviously fake) and the CLI warns when it's used. The real id
   comes from decompiling TR Editor's `FKoaSendRq1`/`FKoaSendDt1` (the imported
   `TREditor_x86_64` in the Ghidra project) or from capturing a real editor↔
   device SysEx exchange.
2. **Does the TR-6S honor RQ1 over its USB-MIDI port?** Some devices only accept
   DT1 pushes and never answer RQ1. Settle empirically once hardware is on hand.
3. **Full address map** beyond the two section bases.
4. **Whether DT1 writes to volatile edit-buffer vs. persistent memory**, and any
   handshake/temporary-area protocol. Matters a lot before any *write* is sent.

## Scope & safety

`tr-sysex` **only builds and parses byte strings** — no MIDI I/O, no device
access. Generating a DT1 (a *write* message) prints bytes but sends nothing; the
CLI still frames it as intent-to-modify. Actually transmitting SysEx over
USB-MIDI — and any DT1 write in particular — is a separate, hardware-gated step,
consistent with the project's no-write-until-verified stance.

## Using the crate

```sh
# Build an RQ1 to read one 0x520-byte kit record from the kit section base:
tr-sysex rq1 kit --size 0x520

# Parse any Roland SysEx (hex, or @file) and verify its checksum:
tr-sysex parse "F0 41 10 00 00 00 00 11 03 00 00 00 00 00 0A 20 53 F7"

# Compute the Roland checksum over an address+data run:
tr-sysex checksum "40 00 7F 00"      # -> 0x41

# Build a DT1 write (prints bytes only; nothing is sent):
tr-sysex dt1 pattern --data "01 02 03 7F"
```

Pass `--model <hex>` once the real id is known to drop the placeholder warning.
