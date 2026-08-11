# TR-6S / TR-8S firmware encryption — crypto brief

*A consolidated, evidence-first report on the encrypted firmware image
(`App1_Main`). Scope: everything actually measured or tested. No decryption was
achieved, no key was recovered, and this brief does not claim either is close.
Every load-bearing statement is tagged with how we know it.*

**Confidence legend:** **[M]** measured/reproducible from the files ·
**[X]** cross-validated two independent ways · **[A]** analysis/derivation from
measured facts · **[O]** open / not yet determined.

---

## 1. Bottom line

- The encryption is a **real 64-bit block cipher in ECB mode**, with full
  diffusion. It is not a weak/homebrew scheme, not a keystream, and there is no
  cryptanalytic shortcut. **[X]**
- The key is **static and shared** across TR-6S *and* TR-8S, unchanged across
  every firmware version we have (2019→2023). **[M]**
- The key and decrypt routine are in **none** of the shipped files — they live
  only in the device bootloader. **[M/A]**
- The software attack surface is **exhausted**: cipher-ID, diffusion, constant
  sweep, and a crib-based weak-key search all completed, all negative for a
  break. **[M]**
- Recovering plaintext (or the key) requires **hardware access to the device**.
  Because the key is static and shared, one successful extraction from any unit
  would apply to all versions of both products. **[A]**

---

## 2. What the encryption is — CONFIRMED

### 2.1 64-bit block cipher, ECB mode **[X]**

- **Duplicate-block analysis.** Identical 8-byte ciphertext blocks recur ~**30%**
  of the time, and only ever on 8-byte boundaries — the ECB fingerprint (equal
  plaintext blocks → equal ciphertext blocks). Reproducible via
  `fw-analyze blockmode`.
- **Offset-independence.** A given ciphertext block's value depends only on its
  8-byte contents, not its position — consistent with ECB, inconsistent with any
  chained or keystream mode.

### 2.2 It is a *real* cipher, not a substitution or keystream **[M]**

Measured over the unique 8-byte blocks of the TR-6S image:

| test | result | meaning |
| ---- | ------ | ------- |
| byte entropy | **7.99991 / 8.0** | indistinguishable from uniform |
| χ² vs uniform | **233** (df 255) | textbook-perfect uniform fit — no frequency skew |
| Hamming-distance-1 block pairs | **0** (random exp. ≈ 2×10⁻⁸) | full avalanche |

The Hamming-distance test is decisive: real firmware is full of 8-byte blocks
that differ in a single byte; a substitution or no-diffusion cipher would echo
that into near-duplicate ciphertext. **Zero** such pairs means every input bit
affects the whole output block — a proper block cipher. This closes the door on
frequency analysis and on the "it's really just XOR/substitution" hypothesis.

### 2.3 What ciphertext analysis cannot give **[A]**

The **block size** (64-bit) is readable from ECB structure; the **cipher
identity** (DES vs Blowfish vs CAST vs XTEA vs …) and the **key size** are not.
A competent cipher's output is built to be statistically featureless, so no
ciphertext-only test distinguishes the algorithm. Those facts become knowable
only from the decrypt routine (i.e. the bootloader).

---

## 3. The key — static, shared, and off-image

### 3.1 One key across both products and all versions **[M]**

The dominant repeated ciphertext block (see §4) is **byte-identical** across
every TR image we have:

| image | dominant block | occurrences | ECB dup rate |
| ----- | -------------- | ----------- | ------------ |
| TR-6S v1.51 | `6b4c9a85 2c732831` | ~11.7k | 28.7% |
| TR-6S v2.00 | `6b4c9a85 2c732831` | ~8.3k | 29.9% |
| TR-8S v3.00 | `6b4c9a85 2c732831` | ~10.2k | 29.1% |

Identical crib ⇒ **same key + same cipher + same constant plaintext**. The key
was not rotated between products or across four years of releases. This is a
real weakness — but a *hardware* one: it raises the payoff of a single
extraction, not the odds of a software break.

### 3.2 The AIRA-Compact family is different **[M]**

The AIRA-Compact apps (T-8 / J-6 / E-4, the E4E platform) are **not ECB** —
every 8-byte block is unique (0% repetition), i.e. a chained mode or
compressed-then-encrypted. So the TR and AIRA families differ in *mode*, not
just key, and the AIRA images offer no crib. (TR/BMC and AIRA/E4E are two
separate encryption families.)

### 3.3 The key is in the bootloader, not any shipped file **[M/A]**

- **Cipher-constant sweep** across every shipped plaintext component (panel-MCU
  ARM images, `init_param`, updater metadata): **no** Blowfish π-constants,
  TEA/XTEA/RC5 `0x9e3779b9`, AES S-box, DES S-box, CAST S-box, or Camellia
  sigma anywhere. The crypto is compiled into none of the shipped plaintext. **[M]**
- The **panel MCUs** are plaintext ARM but contain no crypto — they don't do the
  app decryption (a separate concern from the main SoC's boot). **[M]**
- **TR Editor** (desktop app) statically links OpenSSL/BoringSSL, but that is for
  Roland-Cloud **TLS** — its Blowfish/SEED/AES constants are *not* evidence about
  the firmware cipher. **[A]**
- By elimination, the key + decrypt routine live in the device's **NOR/boot
  region**, which is never shipped in an update. **[A]**

---

## 4. The crib (known plaintext/ciphertext pair) **[M]**

The dominant block `6b4c9a85 2c732831` is `E(key, constant)` — the ECB image of a
large constant plaintext region (erased/zero-filled flash). The constant is
almost certainly all-`00` or all-`FF`. This is a genuine known plaintext/
ciphertext pair and is retained as the one asset any future brute-force would
need as a verifier. It does **not** enable decryption of anything else (see §6).

---

## 5. Attacks run, and their outcomes

Every one of these was executed to completion on the files we hold. Negative
results are still results — together they establish that the software surface is
closed.

| attack | method | outcome |
| ------ | ------ | ------- |
| **Mode/cipher ID** | duplicate-block + offset-independence | 64-bit block, ECB **[M]** |
| **Diffusion / substitution** | entropy, χ², Hamming-1 (§2.2) | real cipher, full avalanche **[M]** |
| **Keystream / stream-cipher hypotheses** | period + XOR-pad tests | ruled out on every measurement **[M]** |
| **Cipher-constant sweep** | signature scan of all plaintext files | no cipher constants present **[M]** |
| **Crib-based weak/default-key search** | ~1,300 candidate keys (all-0/-FF, Roland/AIRA/model-ID/`App1_Main` strings, ASCII tokens scraped from the binaries, header bytes, DES weak/semi-weak keys) × 6 constant guesses × **DES, 3DES, Blowfish, CAST-128, XTEA, TEA** | **no match** — the key is not a guessable default for any common 64-bit cipher **[M]** |

*(IDEA/GOST were not run — no library and 128-/256-bit keys respectively — but a
default-key hit is the only way they'd fall, which is exactly what the search
ruled out for the ciphers it did cover.)*

---

## 6. Approaches that are mathematically closed

Recorded so they are not re-attempted — each is *settled*, with the reason.

- **"Guess plaintext / ASM and let it cascade."** In ECB there is no cascade: a
  correct plaintext guess decodes only that one 8-byte block (blocks are
  independent — no `C1⊕C2 = P1⊕P2` relation as in a stream cipher). ARM code is
  ~97% unique 8-byte blocks, so a code guess would decode ~1 block and stop. **[A]**
- **"Check whether a plaintext guess matches."** Verifying a guess means
  *encrypting* it (needs the key) or having the device encrypt our bytes (a
  chosen-plaintext oracle — the device only *decrypts* updates, so no such
  oracle exists). We cannot test guesses. **[A]**
- **"Full plaintext would reveal the key."** No. A complete known-plaintext set
  is exactly what a real cipher is designed to survive; it does not recover the
  key faster than brute force. And if we *had* full plaintext, the key would be
  moot — decryption's whole purpose is the plaintext. **[A]**
- **Brute force.** Feasible only for a DES-class (≤56-bit) key *and* only with an
  identified cipher. We cannot identify the cipher from ciphertext, and the
  candidates include 128-bit-key options — so brute force is not a path without
  first learning the cipher from the bootloader. **[A]**

---

## 7. Container & update structure — CONFIRMED **[M]**

- The updater (`TR6S_UP.bin`) is a **ustar TAR** of components; the main firmware
  member is `App1_Main` = `dd001_m0c0a_up.bin`.
- Within it: a small plaintext header (`App1_Main`, version `0.010001`, a
  placeholder date), then the **encrypted body** from `~0x100` to `~0x269000`
  (~2.5 MB), then encrypted zero-padding, then a plaintext **build-stamp footer**:
  `Roland DD001_C0A VER.2.00 BLD.0C1D Commit:c1dc3-mod…`.
- The footer is a **human-readable version string, not a signature**. Immediately
  before it sits a lone **16-byte value** (`e39f2867…6380e9e3`) — a candidate
  image checksum/MAC, but **16 bytes is far too short to be an RSA (256–512 B) or
  ECDSA (64–72 B) signature.**
- **No asymmetric signature is visible anywhere in the plaintext wrapper.** A
  post-decrypt signature *inside* the encrypted body cannot be ruled out (§8). **[O]**

---

## 8. Running custom firmware — where the real barrier is

*This section is analysis (**[A]**) framing the options; it does not report a
completed bypass.*

The important distinction: **ECB provides confidentiality, not authentication.**
The bootloader decrypts and jumps; it has no inherent way to tell whether the
plaintext is "legitimately" the vendor's. So the practical barrier to running
custom code is not the cipher key per se — it is **control of the boot chain**:

- The **key is only strictly required** to produce a stock-format encrypted image
  that an *unmodified* device accepts through the normal updater (e.g. a
  redistributable custom firmware). For that you also need to defeat any
  post-decrypt integrity check.
- **On your own hardware, the key is the *hardest* path, not a required one.**
  Key-free routes that don't touch the cipher: patch the (plaintext) bootloader
  in writable NOR to load unencrypted code; glitch past the decrypt/verify step;
  or exploit a bug in the running firmware (needs the plaintext from a RAM dump,
  but no key and no flash write).
- A **hidden signature inside the encrypted payload is possible [O]**, but it only
  bites if the thing that *checks* it is immutable. That check runs in the
  bootloader, so it collapses into a single open question:

> **Is the root of trust in immutable silicon (a mask-ROM secure boot that
> verifies the bootloader) or in patchable flash?** **[O]**

If flash, the bootloader — and any embedded check — can be rewritten. If
silicon, encryption and any embedded signature stand together and the attack
moves to the boot ROM. Identifying the SoC and reading the NOR answers it.

Circumstantially, the posture reads **obfuscation-grade, not hardened**: ECB mode
(a weak mode a security-conscious vendor would not choose), a plaintext build/
git-commit footer left outside any protected region, and at most a 16-byte
integrity value. That *leans* against a robust asymmetric secure-boot chain — but
it is a lean, not proof.

---

## 9. What would actually move this — and it's all hardware

Stated for completeness, not as an action plan:

- **Read decrypted firmware from RAM** (via an unlocked debug port, or a
  glitch-to-dump). Yields the plaintext **without the key** — the goal of
  decryption, achieved by a memory read.
- **Dump the NOR / boot region.** Yields the bootloader → its decrypt routine and
  the key itself, and (crucially) whether there is a post-decrypt signature check
  and whether the bootloader is silicon-verified.

Both require physical access and equipment; neither is a software technique we
have overlooked. The static shared key (§3.1) means a single such extraction
would cover every TR-6S and TR-8S version.

---

## 10. Provenance & discipline

- No real firmware or backup bytes are committed to the repository — only
  synthetic fixtures. This brief cites analytical results and a single 8-byte
  crib value, not firmware payload.
- All measurements are reproducible from the maintainer's own copies of the
  shipped updater packages via `fw-analyze` / `fw-extract` and the scripts used
  in this investigation. No firmware was downloaded and no device was written.
- Tracker: the open hardware item is `cowbell-8o5`; the cross-family
  characterization is `cowbell-hu6`. Deeper narrative lives in
  `docs/firmware-format.md`.
