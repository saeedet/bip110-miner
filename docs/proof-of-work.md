# The proof-of-work construction

Transcribed from `src/primitives/block.cpp` in Bitcoin Knots at tag
`v29.4.1.knots20260508`, and verified against a live regtest chain.

**This is not "BLAKE2b instead of SHA-256d".** It is a five-stage pipeline using
*both* hash functions, and the staging is deliberate: it splits the header into
a part the mining machine never sees and a small part it grinds.

## Why it is shaped this way

Two comments in the source give the game away:

> *"These fields are invisible to the mining machine. This means the hasher
> cannot brick itself at some future block version, time, or difficulty."*

> *"Presumably the actual mining ASIC hardware sees these"*

Bitcoin ASICs hash the raw 80-byte header, so they encode assumptions about its
layout — which is why a version-bit or timestamp change can strand hardware.
Here the ASIC is handed an opaque digest plus nonces, and everything
consensus-shaped is folded in beforehand by the host. The hardware cannot go
stale because it never learns what it is hashing.

A third comment explains the XOR:

> *"The pooling miner doesn't know `m_xor_key` (only the hash of it) until it
> finds a block"*

That is the block-withholding fix. A pool operator can hand out work without
revealing the key, so a miner cannot tell whether a share it found is a *block*
and quietly discard it.

## The stages

`TaggedHash(tag)` is BIP340-style: `SHA256(SHA256(tag) ‖ SHA256(tag) ‖ data)` —
hence the recurring `0x40` of prefix in the assertions.

**0 — XOR key material** *(SHA-256)*
```
xor_key_hash = TaggedHash("Bitcoin block hash PoW XOR key")  << m_xor_key
xor_key_mask = TaggedHash("Bitcoin block hash PoW XOR mask") << m_xor_key
```
then the top `m_xor_key_mask_clear_bits` bits of the mask are zeroed. A null
`m_xor_key` leaves the mask zero, making the whole XOR a no-op.

**1 — `h1`, the consensus fields** *(SHA-256, 119 bytes of input)*
```
TaggedHash("Bitcoin block header 1")
  << GetCompleteVersion()          << prevblock_ordered_sane
  << m_height                      << hashMerkleRoot
  << GetTimeOnWire()               << uint8(0)   // reserved: 40-bit time
  << nBits                         << uint32(m_txcount)
  << m_flags                       << m_xor_key_mask_clear_bits
  << xor_key_hash.GetSHA256()
```

**2 — `h2`, the merge-mining hook** *(SHA-256)*
```
h2_hash = TaggedHash("Merge-mining hook") << h1.GetSHA256() << zeros << zeros << m_mm_rhs
```

**3 — first BLAKE2b, the Stratum half** *(52 bytes in)*
```
blake2b_nokey( uint32(0) ‖ h2_hash ‖ m_extranonce )  ->  hash
```
The source notes these are the fields sent to machines over Stratum v1: the
`uint32(0)` and `h2_hash` are the equivalent of `coinb1`, and `m_extranonce`
sits where the extranonce goes.

**4 — second BLAKE2b, what the hardware grinds**

Layout is selected by `m_flags & 3`:

| flags&3 | input |
|---|---|
| 0 | `prevblock_hidden`(first 6 bytes zeroed) ‖ nNonce ‖ m_nonce2 ‖ m_time_offset ‖ m_nonce3 ‖ hash |
| 1 | nNonce ‖ m_nonce2 ‖ m_nonce3 ‖ m_time_offset ‖ hash ‖ h2_hash |
| 2 | zeros×3 ‖ h2_hash ‖ nNonce ‖ m_nonce2 ‖ m_time_offset ‖ m_nonce3 ‖ hash |
| 3 | zeros×2 ‖ (as case 2) |

where `prevblock_hidden = TaggedHash("Bitcoin prevblock header, hashed") << prevblock_ordered_sane`.

**5 — mask and reverse**
```
final_hash[31 - i] = hash[i] ^ xor_key_mask[i]
```

## What this means for a miner

The natural midstate boundary is **explicit in the design** rather than
discovered, as it was with Bitcoin's 80-byte header:

- Stages 0–3 depend on the template and the extranonce. Compute once per
  extranonce.
- Stage 4 is the inner loop, varying `nNonce`, `m_nonce2`, `m_nonce3` and
  `m_time_offset` — 96 bits of nonce plus a time offset, over an input of
  roughly 100 bytes.
- Stage 5 is two cheap operations on 32 bytes.

So the hot loop is **one BLAKE2b over ~100 bytes**, and everything else is
amortised across the whole extranonce sweep.

## Consequences for this project

We need **both** hash functions: SHA-256 for the three tagged hashes, BLAKE2b
for stages 3 and 4. The SHA-256 side can be lifted from the sibling Bitcoin
project, where it is already tested against real block headers.
