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
| Proof of work | A **five-stage pipeline** using *both* SHA-256 and BLAKE2b — see [docs/proof-of-work.md](docs/proof-of-work.md). Not a drop-in hash swap |
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
| `sha256` | SHA-256 and BIP340 tagged hashes — the PoW needs three of them. Portable from the sibling project, where it is already tested against real block headers |
| `btcb2-primitives` | Both header formats and **both** proof-of-work algorithms — SHA-256d below the fork height, the five-stage BLAKE2b pipeline above it. Merkle trees, transactions, targets. Pure, no I/O |
| `node-rpc` | Typed JSON-RPC for `getblocktemplate` / `submitblock`. Cookie auth; hand-rolled HTTP and base64 |
| `mining` | Coinbase construction, block assembly, nonce search. Pure, no I/O |
| `regtest-miner` | End-to-end miner for a local regtest chain, both sides of the fork |
| `stratum` | Stratum V1 wire types, shared by pool and miner so they cannot disagree |
| `pool` | The solo pool: a node on one side, Stratum on the other |
| `miner` | The hashing client. Knows nothing about blocks or the node |

## On making it generic

The eventual goal is one miner that handles Bitcoin, BTCB2, and whatever comes
next. That means a `Chain` trait owning the header type, the PoW function, and
the nonce layout, with everything else — RPC, merkle trees, transactions,
Stratum framing, threading, stats — shared.

It is deliberately **not** the first step. Designing an abstraction from two
chains, one of which is unwritten, produces a trait that fits neither. This
project builds BTCB2 concretely first and extracts the seam afterwards, from
differences actually encountered rather than imagined.

## What mining this chain actually requires

Four rules a Bitcoin miner does not have, all of them found by reading
`validation.cpp` rather than by guessing:

| Rule | What happens if you miss it |
|---|---|
| The client must declare the `blake2b` rule to `getblocktemplate` | RPC error −8; no template at all |
| The v2 header's `txcount` must equal the real transaction count | `bad-txnlist-size` |
| At *exactly* the activation height, the coinbase `scriptSig` must contain the chain's headline | `bad-headline` |
| From the activation height until RDTS expires, block weight is capped at 800,000 rather than 4,000,000 | `bad-blk-weight-reduced_data` |

The last one comes free if you build from the node's template, since the node
applies the cap when assembling it.

Two things about the nonce space are worth knowing before writing a search
loop. Above the fork there are **128 grindable bits**, not 32 — `nonce`,
`nonce2`, `nonce3`, and `time_offset`, none of which touch the merkle root, so
the coinbase never has to be rebuilt. `time_offset` qualifies because the flag
that would make it consensus-relevant is ours to leave clear, and nothing in
validation constrains it otherwise; `scripts/fork-crossing-test.sh` mines with
it non-zero on every run so that claim keeps being tested rather than trusted.

Below the fork there is **one** word again, and rolling the other three is not
merely useless — they are not serialised, so a search that rolls them
recomputes a single hash until its budget runs out. No error, no block.

## What Stratum V1 looks like on this chain

Stratum predates the fork by fifteen years and still fits, which turns out to
be the clearest way to see what changed. `mining.notify` keeps all nine of its
positional parameters. Two of them go **empty**.

| Slot | Bitcoin | Here |
|---|---|---|
| `prevhash` | the previous block hash | `prevblock_hidden`, first 6 bytes blanked |
| `coinb1` | coinbase bytes before the extranonce | a zero word, then `h2` — every consensus field, digested |
| `coinb2` | coinbase bytes after the extranonce | **empty** |
| `merkle_branch` | the path from the coinbase leaf to the root | **empty** |
| `version` | the header version | the four stage-4 layout flags |
| `nbits` | the block's target | unchanged |
| `ntime` | the header timestamp | `time_offset` — a full 32 free bits |
| `clean_jobs` | discard older jobs | unchanged |

`coinb2` and `merkle_branch` exist so a miner can splice an extranonce into the
middle of the coinbase and then repair the merkle root. This fork put the
extranonce in the header, so there is nothing to splice and nothing to repair.
The pool's job builder loses the sentinel search, the split, the branch, and
the runtime cross-check that guards all three in the sibling Bitcoin project.

The mapping is not invented here. The node's own source says where these bytes
belong, in `primitives/block.cpp`:

```
// These fields get sent to mining machines over Sv1
ss << (uint32_t)0;   // Final 3 bytes are part of Sv1 "coinb1"
ss << h2_hash;       // Remainder of Sv1 "coinb1"
ss << m_extranonce;  // Sv1 "extranonce"
```

### What a miner is not told

Not the block version, the timestamp, the merkle root, or the real previous
block hash — all folded into `h2` before the job exists. The source is explicit
that this is the goal: *"the hasher cannot brick itself at some future block
version, time, or difficulty."*

Nor the XOR mask, and that one cuts the other way. A pool publishes the *hash*
of its XOR key and keeps the key, so a miner can see that a share is good but
not that it is a block — which makes block withholding impossible rather than
merely detectable. A solo miner is its own pool and uses a null key, so no mask
exists.

The two words with no Stratum slot, `nonce2` and `nonce3`, simply stay zero.
Between the 32-bit nonce, the 32-bit offset, and the 64-bit `extranonce2` there
are 2^128 hashes reachable anyway.

### Running it

Two processes, real TCP between them:

```bash
cargo run --release -p pool -- --network regtest
```

```bash
cargo run --release -p miner -- --pool 127.0.0.1:3334 --worker mac.0
```

Port 3334, not Stratum's customary 3333. This chain already shares Bitcoin's
P2P magic and RPC ports — a hazard, not a convenience — and there was no reason
to add a third collision when both miners might run side by side.

## Testing across the fork

The ordinary regtest chain sits above its activation height, so mining on it
only ever exercises the post-fork path:

```bash
./scripts/fork-crossing-test.sh
```

builds a throwaway chain with the fork at height 5, mines through it, and
checks the node's stored hash against the one the miner computed, block by
block. That comparison is the point. On regtest roughly half of all hashes meet
the target, so a miner using the *wrong algorithm entirely* still gets blocks
accepted — which is exactly how this project shipped a v1 path that hashed
pre-fork headers with BLAKE2b and reported hashes the node did not agree with.
Acceptance is not evidence; agreement is.

## Phases

- [x] **0** — Toolchains, repo skeleton, BLAKE2b regtest node running
- [x] **1** — `blake2b` + `sha256`: RFC 7693 vectors, BIP340 tagged hashes, and the full PoW pipeline reproducing real block hashes
- [x] **2** — `btcb2-primitives`: the 164-byte header, and the PoW reproducing real block hashes
- [x] **3** — Mine a regtest block the node accepts — *and one on each side of the fork*
- [x] **4** — Split into pool + miner over Stratum V1
- [ ] **5** — Their testnet — a real block on a public network
- [ ] **6** — Mainnet, only after measuring the real difficulty
- [ ] **7** — Extract the `Chain` trait

Phases 0–4 need no chain download at all; regtest generates its own.
