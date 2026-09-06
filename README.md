# btcb2-miner

A solo miner for the **BIP-110 / BLAKE2b Bitcoin hard fork** (ticker BTCB2),
written from scratch in Rust, running against a local node.

It is the sibling of [solo-mac-miner](https://github.com/saeedet/mac-solo-miner),
which does the same for Bitcoin. That project's expected time to a block is
~180 million years. This one's is not, and that is the whole reason it exists.

## What this chain is

A minority hard fork that split from Bitcoin at **block 961,632** on 8 August
2026 and replaced SHA-256d with BLAKE2b. Every value below was read out of the
node's own source at tag `v29.4.1.knots20260508`, not from a blog post:

| | |
|---|---|
| Node software | [`bitcoinknots/bitcoin`](https://github.com/bitcoinknots/bitcoin) — a Bitcoin Core fork, so `getblocktemplate` / `submitblock` survive |
| Proof of work | `blake2b_nokey(out, 32, header, len)` — **single, unkeyed BLAKE2b-256**. Not double, no salt, no personalisation |
| Header | **164-byte "v2"**, flagged by `0x80000000` in the version field |
| Activation | `Blake2bHeight = 961640` (mainnet), `150308` (testnet), configurable on regtest |
| Difficulty easing | `Blake2bTargetShift = 22` — a ~4.2-million-fold easing at the fork |
| Fork message | `"8-30 NYPost Deride And Conquer"` — this chain's genesis-style headline |

Checkpointed in the release, which settles that the chain really produced
blocks:

```
961632  first BIP110 block
961639  last SHA256d block
961640  first BLAKE2b block   0000000000000050c1e5f69672f459293be14f46e5a494e7a8c8541396f18eeb
```

## Why the header grew from 80 bytes to 164

Not vanity. The upstream change lists the reasons: 128 bits of ASIC nonce plus
128 bits of machine nonce, an opt-in fix for block withholding, room for merge
mining, and a transaction-count commitment that properly fixes CVE-2017-12842.

Its serialised v2 field order, from `SERIALIZE_METHODS` in `primitives/block.h`:

```
nVersion  hashPrevBlock  hashMerkleRoot  nTime  nBits  nNonce
m_nonce2  m_nonce3  m_extranonce  m_time_offset  m_txcount
m_flags  m_xor_key_mask_clear_bits  m_xor_key  m_height  m_mm_rhs
```

**The consequence that matters most for a miner:** the extranonce lives *in the
header*. On Bitcoin, rolling the extranonce means rebuilding the coinbase, which
changes its txid, which changes the merkle root. Here the merkle root never
moves. That is simpler, and it changes what a pool has to send a miner.

## Honest assessment

**As engineering, this is more interesting than mining Bitcoin.** BLAKE2b has no
hardware acceleration on any consumer machine, but it is fast in software by
design — so a CPU is not competing against a decade of dedicated silicon. On a
chain this small, a laptop can plausibly find real blocks.

**As an investment it is close to worthless.** The fork drew about 2.53% miner
support. One small beta exchange listed it with bids near $82 against asks near
$190 — a spread over 130%, which is another way of saying it cannot be sold. No
major exchange, wallet, or Lightning implementation has committed to it. The
chain managed four blocks in its first week before being restarted.

Those are separate questions. Watching your own code win a real block is a good
reason to do this. Expecting to sell the proceeds is not.

## Planned architecture

Deliberately close to the Bitcoin project's, because most of it transfers:

```
knots node (BLAKE2b)  ──JSON-RPC──▶  pool  ──Stratum V1──▶  miner
```

| Crate | Responsibility |
|---|---|
| `blake2b` | BLAKE2b. Readable reference impl + optimised one, tested against each other |
| *(planned)* `btcb2-primitives` | The 164-byte header v2, transactions, merkle trees, targets |
| *(planned)* `node-rpc` | Typed JSON-RPC for `getblocktemplate` / `submitblock` |
| *(planned)* `mining` | Coinbase construction, block assembly, nonce search |
| *(planned)* `pool` / `miner` | The Stratum split |

## On making it generic

The eventual goal is one miner that handles Bitcoin, BTCB2, and whatever comes
next. That means a `Chain` trait owning the header type, the PoW function, and
the nonce layout, with everything else — RPC, merkle trees, transactions,
Stratum framing, threading, stats — shared.

It is deliberately **not** the first step. Designing an abstraction from two
chains, one of which is unwritten, produces a trait that fits neither. This
project builds BTCB2 concretely first and extracts the seam afterwards, from
differences actually encountered rather than imagined.

## Phases

- [ ] **0** — Toolchains, repo skeleton, BLAKE2b regtest node running
- [ ] **1** — `blake2b`: reproduces RFC 7693 vectors and real block hashes
- [ ] **2** — `btcb2-primitives`: round-trips a real 164-byte header
- [ ] **3** — Mine a regtest block the node accepts
- [ ] **4** — Split into pool + miner over Stratum V1
- [ ] **5** — Their testnet — a real block on a public network
- [ ] **6** — Mainnet, only after measuring the real difficulty
- [ ] **7** — Extract the `Chain` trait

Phases 0–4 need no chain download at all; regtest generates its own.
