# Firmware format — `.bin` structure & update mechanism

Findings on how the TR-6S firmware update is packaged and what (if anything)
protects its integrity. All observations are from the maintainer's own updater
downloads and hardware; no Roland binaries are committed to this repo.

## Update delivery (what the user runs)

Roland distributes the updater as a platform-specific installer that, when run,
places a firmware file (historically a `.svd`/`.bin`-style payload) onto the
unit — typically over USB mass-storage or a vendor updater app.

- **Windows:** self-extracting `.exe` (InstallShield/NSIS lineage) or `.msi`.
- **macOS:** `.dmg` containing an app, or a flat `.pkg` (`xar!` archive).

`fw-extract identify` sniffs which of these you have; `fw-extract unpack` shells
out to the matching tool (`7z` / `msiextract` / `xar`); `fw-extract carve` then
locates the dense payload blob inside the unpacked tree.

## Payload structure — OPEN

Unknowns to resolve with `fw-analyze`:

- [ ] Is there a **header** (version, length, load address, part number)?
- [ ] Single monolithic image, or **multiple sections** (bootloader / app /
      sample ROM) concatenated? Run `fw-analyze entropy` to find boundaries.
- [ ] Endianness and word size of any length/offset fields.

## Integrity / signing — OPEN

Central safety question before *any* write-to-device work is considered:

- [ ] Is there a **checksum/CRC** field? Use `fw-analyze checksum --expect`
      to brute-force common algorithms once a candidate field is located.
- [ ] Is the image **cryptographically signed** (RSA/ECDSA)? High-entropy
      fixed-size trailing block near the end would be a hint.
- [ ] Does the bootloader **verify** before applying, or apply blindly?
- [ ] Is there **anti-rollback** (monotonic version enforcement)?

Until the presence/absence of signing and the update flow are understood, this
project stays strictly read-only — see the phase gate in the README.

## Method log

_(Append dated entries here as findings accumulate — e.g. "entropy scan shows a
flat ~7.99 region from 0x0040_0000 onward → likely compressed sample ROM".)_
