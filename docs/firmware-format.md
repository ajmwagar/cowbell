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

Confirmed identical across **five** products (all `tar` + per-image 96-byte
`App*` header):

| Product | Update file | Model code | SoC | Members |
| ------- | ----------- | ---------- | --- | ------- |
| **TR-6S** | `TR6S_UP.bin` | `dd001` | BMC | `App1_Main` (enc) + `init_param` |
| **TR-8S** | `TR8S_UP.bin` | `rpg42` | BMC | `App1_Main` (enc) + `init_param` |
| **T-8 / Beat 8** | `T8_UPD.BIN` | `DD010` | E4E | `App_Main` (enc) + `App_Panel` (**plaintext ARM**) |
| **J-6** | `J6_UPD.BIN` | `DD011` | E4E | `App_Main` (enc) + `App_Panel` (**plaintext ARM**) |
| **E-4** | `E4_UPD.BIN` | `DD012` | E4E | `App_Main` (enc) + `App_Panel` (**plaintext ARM**) |

Naming note: the big boxes use `App1_Main` (the `1` implying multiple app
processors); the AIRA Compacts use `App_Main` + a separate `App_Panel`. The
TR-6S/TR-8S ship no panel sub-image in the update — their panel MCU is updated by
another path or bundled in the main image.

### Two encryption-key families — TR/BMC vs AIRA-Compact/E4E (2026-08-07)

Cross-correlating all five encrypted app images at 8-byte block granularity
reveals **two disjoint key domains**, tracking the SoC:

- **TR / BMC key:** TR-6S ↔ TR-8S share it (66,233 shared blocks; 15,936-byte
  identical run). See the KEY FINDING section below.
- **AIRA-Compact / E4E key:** T-8, J-6, and E-4 share it among themselves
  (T8↔J6 1,870 shared blocks; J6↔E4 1,732; T8↔E4 877 — far above chance given
  0% internal ECB repetition, and their first encrypted block is byte-identical
  `3DAAFBD228D29316`). All three load `App_Main` at `0x60000000`.
- **The two families are disjoint:** the T-8 (E4E) app shares **zero** blocks
  with the TR (BMC) apps. Different SoC → different bootloader → different key.

Implication: recovering **either** key unlocks a whole product family. The TR
key (our target) decrypts TR-6S + TR-8S; the Compact key would decrypt
T-8 + J-6 + E-4.

### AIRA Compact panel firmware is one shared codebase (plaintext ARM)

All three Compact `App_Panel` images are plaintext ARM Cortex-M with an identical
vector-table base. **T-8 and J-6 panels are 98.8% byte-identical** (260 differing
bytes / 20,508 — mostly relocated pointers + product IDs), so the T-8 panel RE
(below) applies to the J-6 essentially verbatim. The **E-4 panel** is a distinct
build (~18% identical) but the same STM32/toolchain and — confirmed in Ghidra —
**the same MIDI-over-UART command dispatcher**, with one addition: it handles
`0xC0` **Program Change** (patch select for the voice unit) on top of the T-8's
command set. So the MIDI-over-UART panel protocol is a **platform-wide Roland
convention**, tweaked per product.

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

### Key size & brute-force feasibility

- **Block size = 64-bit is CONFIRMED; key size is UNKNOWN.** These are separate
  numbers — "64-bit" here is the *block*, not the key. Key size is a property of
  the (still-unidentified) cipher and cannot be read off the ciphertext. It only
  becomes known once we see the decrypt routine (the NOR dump). Candidate key
  sizes: DES 56, 3DES 112/168, XTEA/TEA 128, CAST-128 128, IDEA 128, Blowfish
  up to 448.
- **Brute-force feasibility is a cliff at ~56 vs ~128 bits.** Only **single DES
  (56-bit)** is brute-forceable (~a day, ~$100 via a service like crack.sh).
  Every ≥112-bit option is computationally impossible: 2^128 ≈ 10^38 keys — no
  budget or timescale reaches it. Since the leading guesses (XTEA / custom
  Feistel) are ~128-bit, the honest expectation is **the key is NOT
  brute-forceable.**
- **ECB weakness does not help.** ECB being a weak *mode* leaks *structure*
  (repetition), not the key; it does not lower brute-force cost. The underlying
  cipher is full-strength.
- **The DES long-shot (logged for completeness).** The dominant fill block
  `6B4C9A85 2C732831` is almost certainly `E(K, padding)` where padding is likely
  8× `0x00` or 8× `0xFF` — i.e. a **known plaintext/ciphertext pair**, exactly
  what a DES cracker needs. *If* the cipher is DES, feeding that one pair to
  crack.sh recovers the key in ~a day. Two assumptions ride on it (that it is
  DES, and the padding value); if it is a 128-bit cipher, one pair does nothing
  to 2^128. Cheap, low-probability, non-zero — a fallback-to-a-fallback.
- **Bottom line:** don't gate progress on brute-force. The key lives in the NOR
  bootloader and is **read directly** once dumped, whatever its size — a 128-bit
  key we could never brute-force is trivial to *read* off the chip that stores
  it. And the NOR may hold the already-decrypted firmware, making the key moot.

### "It's ECB, so XOR out the known plaintext and you have everything"

A recurring suggestion, and worth answering precisely because the instinct — ECB
is a weak mode, known plaintext helps — is right while the mechanism is not.

The argument runs: block ciphers generate a keystream from key material and XOR
it with the plaintext, so XOR-ing known plaintext back out recovers a key-only
keystream, no need to identify the cipher. That describes a **stream cipher, or
a block cipher in a keystream mode** (CTR / OFB / CFB). ECB is not one. In ECB
each block is `C = E_K(P)`: the plaintext goes *through* the cipher's rounds, and
there is no key-only stream behind it. `P ⊕ C` is just a number with no predictive
value for any other block.

**The evidence that says "ECB" is the same evidence that says "not a keystream."**
Under CTR/OFB the keystream never repeats across an image, so identical plaintext
at different offsets encrypts to *different* ciphertext and duplicate-block rates
collapse to ~0%. We measure **29.89% duplicates at 8 bytes**, one block occurring
**8,420 times**. Block repetition is the ECB fingerprint *because* ECB is not a
keystream mode. Were this CTR or OFB, the shortcut would work and the image would
already be open.

#### Measured refutation: the offset-independence test

Repetition alone doesn't finish the argument, because a **repeating XOR pad**
produces repeats too — and if that were the scheme, the suggestion would be
right. The two cases separate cleanly on *where* the repeats land:

- Under a keystream or pad of period `P`, two identical ciphertext blocks require
  identical plaintext **and** offsets congruent mod `P`, so every gap between
  occurrences is a multiple of `P`.
- Under ECB, `C = E_K(P)` depends only on block contents, so repeats land at
  **arbitrary** offsets and the gcd of the gaps collapses to the block size.

Measured on the 8,420 occurrences of the dominant block (`fw-analyze blockmode`):

| Test | Result |
| ---- | ------ |
| gcd of gaps between occurrences | **8** (= the block size) |
| gaps that are not a multiple of 16 | **7,165 of 8,419** |

So any pad must have period dividing 8. Each survivor dies individually:

| Period | Why it's excluded |
| ------ | ----------------- |
| 1, 2, 4 | A pad of period `p` would make the dominant block `8/p` identical chunks. It splits into 8 / 4 / 2 **distinct** chunks respectively. |
| 8 | XOR-ing the block out as a repeating pad at all 8 alignments leaves entropy **7.94–7.98/8** and ≤3.10% zero bytes — and that 3.10% is just the fill blocks mapping to themselves. A real pad would collapse the whole image to structure. |

**No keystream or XOR pad of any period explains the data.** ECB over a genuine
64-bit block cipher stands, and `P ⊕ C` yields nothing reusable.

Reproduce with:

```
fw-analyze blockmode <image> --offset 96 --len 2529168
```

(The related claim that block ciphers "XOR key material at the end" describes
**key whitening** — AES's AddRoundKey, Blowfish's P-array, the TEA family. It is
one layer of a round function, not the whole transform; the data still traverses
nonlinear key-dependent rounds, so it cannot be peeled off with a single XOR.)

**What ECB does give us, stated correctly:** a *codebook*. A known
plaintext/ciphertext pair lets you decrypt and forge exactly that block value
wherever it occurs — which is real leverage, and already exploited above: the
fill block is `E(K, padding)`, it identifies erased/padded regions, and it is the
one pair a DES cracker would need. It does not extend to blocks we have never
seen.

And the codebook is far shallower than "you probably have all of it" implies:

- The **10 most frequent block values cover 6.40%** of the image (20,238 of
  316,146 blocks) — and that 6.4% is fill, the least interesting bytes in the file.
- **89.9% of distinct block values occur exactly once** (199,188 of 221,652).

So even granting a known-plaintext pair for every one of the top ten blocks — far
more than we have — you decrypt 6% of the image and learn nothing about the code.
The codebook is 2^64 rows wide and we hold roughly one useful row.

#### Worked example: XOR-ing the padding out (both `00` and `FF`)

Run the "XOR the known plaintext out to get the key" idea against a cipher where
we *pick* the key, so the answer is checkable. Using **XTEA** (a 64-bit-block
candidate) with a key we chose:

| assumed pad | candidate `KS = fill ⊕ pad` | decrypts a *different* block? | `pad ⊕ C_pad` = `secret ⊕ C_secret`? |
| ----------- | --------------------------- | ----------------------------- | ------------------------------------- |
| `00…` | `ead425c2a35c0919` | ❌ garbage | `ead4…` ≠ `7490…` |
| `FF…` | `959fbb86ad6a12a3` | ❌ garbage | `959f…` ≠ `7490…` |

The last column is the crux. If a key-only keystream existed (`C = P ⊕ KS`), then
`P ⊕ C` would be the *same* value for every block — it isn't. `FF` yields a
genuinely different candidate than `00`, and both fail, because the padding value
only picks *which single block's plaintext you're guessing*, never how many. The
identical procedure on a real stream cipher recovers the block perfectly; on ECB
it can't, because `C = E_K(P)` runs the plaintext through the cipher's rounds —
there is no keystream term to cancel. Reproduce:

```python
def xtea_enc(v0, v1, key):           # a real 64-bit block cipher (a candidate)
    d, s, M = 0x9E3779B9, 0, 0xffffffff
    for _ in range(32):
        v0 = (v0 + ((((v1 << 4) & M ^ v1 >> 5) + v1) ^ (s + key[s & 3]))) & M
        s = (s + d) & M
        v1 = (v1 + ((((v0 << 4) & M ^ v0 >> 5) + v0) ^ (s + key[(s >> 11) & 3]))) & M
    return v0, v1

KEY = [0x13371337, 0xCAFEBABE, 0xDEADBEEF, 0x0BADC0DE]   # we know it -> checkable
E = lambda b: b"".join(x.to_bytes(4, "big") for x in
                       xtea_enc(int.from_bytes(b[:4], "big"),
                                int.from_bytes(b[4:], "big"), KEY))
xor = lambda a, b: bytes(x ^ y for x, y in zip(a, b))
secret, C_secret = b"TR6S\0\0\0\5", E(b"TR6S\0\0\0\5")
for pad in (bytes(8), b"\xff" * 8):
    KS = xor(E(pad), pad)                       # candidate keystream from the pad
    print(xor(C_secret, KS) == secret)          # decrypt another block? -> False, False
```

#### Can we guess more plaintext — ARM opcodes, an ISA, a vector table?

Tempting, and worth stating exactly why it doesn't extend the decryption:

1. **You can't verify a code guess without the key.** Guessing "this block is an
   ARM prologue" has no check — the only self-verifying guess is content forced by
   frequency/structure (the padding block) or a byte-exact **positional crib**
   (known value at a known offset). We have the padding; we have no confirmed crib
   (the ISA is unknown, so even the reset-vector layout is a guess).
2. **Even a *correct* guess only decodes its exact repeats.** ECB is a codebook
   keyed by block value: a confirmed `(P, C)` decrypts every block whose ciphertext
   is `C`, and nothing else. Code blocks are almost all unique — measured on our
   *plaintext* ARM panel firmware, **97% of 8-byte blocks occur exactly once** — so
   a correct code-block guess would decode ~1 block. Guessing buys coverage only on
   *repetitive* content (padding, zero-runs, constant tables), which is the least
   informative part.
3. **ECB blocks are independent, so cribs can't be chained — this is the deep
   reason.** In a stream cipher / two-time-pad, `C₁ ⊕ C₂ = P₁ ⊕ P₂`: a relationship
   *between* blocks that lets you drag a crib and validate it against the whole
   message. ECB has no such relationship — `Cᵢ = E_K(Pᵢ)` in isolation — so one
   crib is one codebook row that tells you nothing about any other block. Crib-
   dragging, the thing that makes guessed plaintext cascade elsewhere, **does not
   propagate here.**

**What guessing/ISA-comparison *does* buy — a structural fingerprint, measured.**
ECB preserves block *frequency*, so we can compare the encrypted image's
repetition profile against known plaintext without decrypting:

| Content | 8-byte-block duplicate rate |
| ------- | --------------------------- |
| Plaintext ARM code (our panel MCUs) | **~3%** (top block = zero-padding) |
| Encrypted TR-6S `App1_Main` | **~30%** |

Real instruction streams barely repeat at 8-byte granularity; the image's 10×
higher duplication is **not code** — it is the fill block plus the large static
data band the [relocation diff](#structural-mapping-from-cross-version-diffs-no-decryption)
found. So "using ARM plaintext" *does* teach us something: the encrypted payload
is code **plus** a lot of repetitive data/tables, not a pure code image — but that
is a content-type inference from frequency, not a decryption.

**Bottom line:** guessing plaintext yields the one padding crib we already hold
(the DES-56 long-shot input), decodes only repeats, and cannot chain. It does not
open the image. The key comes off the chip (`cowbell-8o5`).

#### Does Ghidra corroborate the cipher? No — and that's the expected answer

The ECB conclusion is a **black-box statistical property of the ciphertext**; it
never depended on disassembly, which is its strength (it is reproducible via
`fw-analyze blockmode`). Ghidra cannot confirm the *cipher identity*, because the
code that would — the decrypt routine and key — lives in the **BMC bootloader in
NOR**, which is not dumped (the object of bead `cowbell-8o5`). The encrypted
`App1_Main` is ciphertext with nothing to disassemble, and `dd001_init_param.bin`
is pure `DATA` (0 functions).

What the loaded programs *do* establish is the **negative space**, and one trap to
avoid. Sweeping every program in the Ghidra project (`tr6s.gpr`) for the candidate
ciphers' signature constants and crypto-named routines:

- **TR Editor (`TREditor_x86_64`, the desktop app) statically links all of
  OpenSSL** — AES-NI, SHA-1/256/512, ChaCha20/Poly1305, RC4, Keccak, SEED,
  Blowfish, GCM/GHASH, X25519, plus the full TLS ciphersuite string table. This is
  the app's **Roland Cloud TLS networking**, not firmware handling. Every crypto
  constant traces to it: the `0x9E3779B9` "TEA/XTEA delta" is inside
  `_SEED_set_key` (SEED reuses the same golden-ratio constant), and the
  `0x243F6A88` Blowfish P-array sits in an OpenSSL data table. The desktop app
  never decrypts firmware — consistent with `ML::CRolandMessage::CheckSum` being
  the **SysEx** checksum, not a firmware routine. The device decrypts its own
  updates.
- **The plaintext ARM panel-MCU firmware (`DD010`/`DD012_pnl_ARM`) has zero cipher
  constants and zero crypto-named functions** — it is the IO/panel processor, off
  the crypto path.

Two consequences:

1. **Corroboration in the negative.** The key and cipher are provably absent from
   everything we hold, which is exactly why key recovery targets a NOR dump and
   not any shipped file.
2. **A false-positive trap, flagged.** Because OpenSSL bundles Blowfish, SEED, AES
   et al., their constants appearing in the *desktop app* say **nothing** about the
   firmware cipher. "TR Editor contains Blowfish, so the firmware is Blowfish" is a
   misread of statically-linked library code — do not treat it as evidence.

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

### The key is stable across firmware versions (2026-08-08)

With three TR-8S update versions on hand (**v1.05, v1.13, v3.00** — spanning
2018→2023) the "recover once, decrypt everything" claim is now tested across
*time*, not just across models. `fw-analyze blockmode` on each `App1_Main`:

| Image | Payload | ECB dup @8 | Fill block `6B4C9A85…` |
| ----- | ------- | ---------- | ---------------------- |
| TR-8S v1.05 | 2,121,552 | 26.68 % | ×9,329 |
| TR-8S v1.13 | 2,139,600 | 26.81 % | ×9,308 |
| TR-8S v3.00 | 2,535,360 | 29.03 % | ×10,306 |
| TR-6S v2.00 | 2,529,168 | 29.89 % | ×8,420 |

The **same fill block `6B4C9A852C732831`** — `E(K, padding)` — appears in every
version and both models, and so do the next five most-frequent blocks
(`cf5ee0c2…`, `97e1dea2…`, `445d8d01…`, `93b181af…`). Two different keys
producing the same `E(K, pad)` is a 2⁻⁶⁴ coincidence; that it repeats for six
distinct blocks across four images is conclusive: **one key, unchanged from
v1.05 through v3.00 and shared TR-6S↔TR-8S.** Shared *unique* blocks track the
same story — consecutive versions overlap ~46 % (v1.05∩v1.13 = 88,817), and even
v1.05∩v3.00 (five years apart) share 72,125; the cross-model v3.00∩TR-6S-v2.00 =
66,232 reproduces the earlier finding.

This settles the "would older firmware help?" question empirically:

- **Not a path to the key.** Every version is ECB under the *same* key; more
  same-key ciphertext never lowers the brute-force cost. No version shipped the
  main app unencrypted (all 27–30 % ECB-duplicated).
- **It is decryption insurance.** Dump the key once from *any* unit or version's
  NOR and it decrypts every version of both products; the cross-version shared
  blocks then let one decrypted image reveal the unchanged blocks of all the
  others for free. Collecting versions is worthwhile *after* the key, not before.
- Reproduce: `fw-analyze blockmode <m0c0a> --offset 96 --len <payloadlen>`.

Note this is `fw-analyze`, not Ghidra: the images are ciphertext, so there is
nothing to disassemble until the key is out (see the Ghidra negative-space note
above). A *plaintext* cross-version diff is possible via the decompressed
`init_param` — useful for tracking data-format evolution, but that feeds
`tr-format`, not the crypto.

### Structural mapping from cross-version diffs (no decryption)

Because the key is stable, ECB makes the encrypted images *diffable*: identical
plaintext → identical ciphertext block, so `fw-analyze diff --block 8 --app1
--relocate` segments an image into regions that are unchanged, relocated, or
genuinely new **without decrypting anything**. Same-*offset* comparison is nearly
useless here (an update inserts bytes and shifts everything after), so the
**relocation pass** anchors on blocks unique to both versions and finds the
matched runs regardless of where they moved.

**TR-6S v1.51 → v2.00** (payload +69,040 B): ~57 % of blocks are shared content,
almost all *relocated* (only 2.7 % sit at the same offset). The shift plateaus
increase monotonically with offset — `+384 → +528 → +13,312 → +17,024 → +37,648
→ +66,832 → +68,720` — the fingerprint of content inserted at **several** points,
each pushing everything after it further down. So it is a *pervasive* update, not
a localised patch. The largest single unchanged region is a **64 KB block that
relocated as a unit** (`0x1a5100 → 0x1b5610`), followed by a cluster of KB-scale
runs all at `+66,832` in `0x1a2dd8..0x1da1f0` — a ~230 KB band of **static data**
(tables/resources) that survived byte-for-byte and only moved.

**TR-8S across versions** (same method): v1.05 → v1.13 (a minor bump) already
touches code throughout — 55 % shared, shifts clustered at `+13–17 KB`, no
localised patch. v1.13 → v3.00 (major) inserts one **~284 KB block** (`+284,024`
dominates) plus distributed changes — a large feature/data addition.

**Cross-model at matched feature generation.** Per the release history, **TR-8S
v3.00 ≈ TR-6S v2.00** (same timeframe/features; the 8S runs one major version
ahead). Diffing them: **42 % shared blocks** — the highest cross-model overlap —
and the dominant shift is near-zero (`+672 B`), i.e. the shared content sits at
almost the same offsets. That is the **shared BMC platform** (common libraries,
tables, bootloader-adjacent code) laid out alike; the other ~58 % is
product-specific (6 vs 11 voices, different sample sets/UI). This quantifies the
platform-vs-product boundary the key-sharing already implied — now at aligned
feature levels.

**Limit, stated honestly:** block-diff reveals *where* content is identical,
relocated, or changed — structure and update mechanics — not what any region
*means*. "Static band = data, churned region = code" is inference from
version-stability and contiguity, not proof, and nothing inside a *changed*
region is recoverable while encrypted. What it does give: a segmentation, and a
priority list of large version-stable regions that become the best
known-plaintext targets the moment any plaintext arrives.

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
- **The panel↔main link is a bidirectional MIDI stream over USART1 @ 115200
  baud, 8N1** (`USART1_UART_Init` sets `BRR`-equivalent `0x1C200` = 115200).
  BOTH directions are MIDI — confirmed by decoding both handlers, not inferred.

  **Panel → main (input events)** — `PanelTxMidiMessage(buf,len)` @ `0x08003EC8`,
  DMA'd out (matches the DMA1_Ch1 handler) via a 128-byte ping-pong buffer.
  `main` is a poll→serialize→TX super-loop emitting 3-byte packets. Opcodes
  (constants near `0x08004C90`):
  - `0x90`/`0x91`/`0x93` = Note On, ch 0/1/3 (button/pad groups, press+velocity)
  - `0x80` = Note Off, ch 0
  - `0xFE` = Active Sensing (1-byte periodic keepalive)
  - encoder deltas clamped to signed 7-bit (`−0x40..0x3F`) before send.

  **Main → panel (LED/display commands)** — RX is byte-at-a-time HAL
  `Receive_IT` (`HAL_UART_RxCpltCallback` @ `0x08002808`) into a ring buffer,
  parsed by `PanelParseMidiRx` @ `0x08003460` (a running-status MIDI state
  machine: `≥0x80` = status, `0xF_` = system/real-time, else data; assembles
  3-byte messages) and dispatched by `PanelDispatchMidiCommand` @ `0x08003FC0`:
  - `0xA0` Poly Aftertouch `[idx, val]` → `PanelSetRgbLed`: idx 0..30 selects a
    pad; a 3-byte-per-LED table maps it to 3 PWM channels (**RGB**), val = level.
  - `0xB0` Control Change `[cc, val]` → `PanelSetIndicatorLed`: discrete GPIO
    LEDs (cc 0/1/2 → specific bits), on when val≠0.
  - `0xE0` Pitch Bend → 14-bit parameter.
  - `0xFA` Start / `0xFB` Continue → transport/tempo sync.
  - `0xFF` Reset → triggers a re-handshake (sets `main` event bit `0x80`).
- **RTOS present:** RX buffering uses FreeRTOS-style queue primitives
  (`...FromISR` privilege/exception-number detection).
- **Division of labour confirmed:** this MCU is only the front-panel I/O
  concentrator (matrix scan via the `74HC138`, RGB pad + indicator LED drive,
  encoder/analog reads); the **BMC is the master** (sequencer, ACB engine,
  audio/USB/MIDI/storage) and drives the panel over this UART-MIDI link.

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
      CRC-32 → compare**.
- [x] **Exact body range pinned (re-verified 2026-08-08).** `zlib.crc32(body)`
      with `body = image[H : H + plen]`, where `H` is the header size
      (**`0x40`** for the compact 8-char-name layout, **`0x60`** for `App1_Main`)
      and `plen` is the payload-length field (`0x24` compact / `0x44` App1_Main).
      Confirmed on the plaintext App_Panel: `crc32(DD010_pnl[0x40:0x40+0x5034]) =
      0x53C790FB`, matching its `0x2C` field, via both `zlib` and `fw-analyze
      checksum`. **Ready verifier for key recovery:** the correct TR-6S
      decryption is the one where `crc32(plain[0x60:0x60+plen]) == 0x46DE65B3`.

  **What this oracle is — and is not.** It is a *self-validating decryption
  oracle*: any candidate (key, cipher) is confirmed instantly — decrypt, CRC-32,
  compare — with no plaintext eyeballing, and it lets cipher hypotheses be tested
  cheaply. It **disambiguates the DES-56 long-shot**: a single known-plaintext
  block (the padding → fill pair) can yield a few spurious DES keys from a
  cracker, and the whole-image CRC-32 rejects all but the true key
  (`2⁻³²` false-accept). But it is a **verifier, not a shortcut** — it says *when*
  you have won, not *how*, and does not shrink the keyspace. CRC-32 is 32 linear
  bits over ~20 M plaintext bits (no plaintext reconstruction), it is over
  plaintext while ECB is non-linear (no computable ciphertext-side handle, so you
  must decrypt first), and it is not a queryable padding-oracle-style leak. So it
  makes DES-56 clean and unambiguous, but a perfect verifier over an unsearchable
  128-bit keyspace still cannot open the image.
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
| `0x08` | `compressed_size[3]` | `0x0006B858` ×3 (all equal) |
| `0x14` | `decompressed_size[3]` | `0x0033FFD0` ×3 (all equal) — **the LZSS output length, not a load address** |
| `0x20` | `reserved[4]` | zero |
| `0x30` | payload | 3 × `0x6B858` **LZSS-compressed** sections |
| `0x142938` | tail | 54,984 B zero pad + three LE `0x0006B858` footer words |

**The three sections are byte-identical** (verified by SHA-256:
`76a160f2aeb0347d001c1dfdad1a04f1…`) ⇒ **triple redundancy** for
wear-levelling / integrity fallback, not three distinct banks.

**Each section is LZSS-compressed and decompresses to a full factory-default
TR-6S backup image** (Okumura `lzss.c`: 4096-byte ring pre-filled `0x00`, initial
pos `0xFEE`, LSB-first flags with `1`=literal, 2-byte match = 12-bit offset +
`(b1 & 0xF)+3` length). Decoding from file offset `0x30` consumes the `0x6B858`
section exactly and emits exactly `0x33FFD0` bytes — a container with `TR6S`
magic whose `SYS `/`PTN `/`KIT `/`TONE` chunks sit at backup offsets, and whose
`SYS ` chunk is **byte-identical** to a real SD backup's. Independently
reproduced (see `docs/tr-format.md`, "Reconciling `SYS ` with `init_param`").

This **corrects two earlier readings**: the `0x14` field is the *decompressed
size*, not a load address; and the "ASCII tags in binary tables / ascending
`uint16` at stride `0x12`" description was an **artifact of reading LZSS match
tokens as data** — the payload is not a table, it is a compressed backup. The
practical win: `init_param` is a free **second corpus** — the factory-default
values for every backup section — that `tr-format` parses once decompressed.

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
