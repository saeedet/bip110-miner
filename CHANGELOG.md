# Changelog

All notable changes to this project. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

The first release, as one command anyone can run.

### Added

- `bip110-miner`: one command that sets up whatever is missing and then mines,
  with `setup`, `status`, `wallet`, `doctor` and `stop` alongside it.
- Setup in the terminal, asking only about what isn't ready: installs Bitcoin
  Knots 29.4.2 after checking the download against the signed release's hash;
  creates a passphrase-protected wallet that loads on every start and is backed
  up, or takes a pasted address the node checks first; and offers a quick start
  from a UTXO snapshot, then follows the sync until mining can begin.
- A live dashboard that fits 80×24 and updates in place: hashrate with a graph,
  the odds in plain words, lifetime totals, recent events, a screen for a found
  block, and help on `?`. Keys to pause and to change the power level.
- Power levels (eco, balanced, max) instead of thread counts, remembered
  between runs.
- Plain log output whenever the output isn't a terminal, or with `--plain`.
- The Mac is kept awake while mining, unless `--allow-sleep` is given.
- Lifetime totals per network, so testing never inflates the mainnet figures.
- Downloads resume after an interruption. A busy RPC or mining port is moved
  to a free one instead of failing.

### Changed

- Requires Bitcoin Knots 29.4.2 or newer, which knows the coinbase-maturity soft
  fork that activated at block 973,440. Older nodes are refused.
- The regtest and testnet4 nodes use their own RPC ports (18453 and 48342) so a
  Bitcoin node can run beside them.
- The pool and miner are libraries, reporting through a shared event stream;
  the `pool` and `miner` binaries remain for debugging.

### Fixed

- Quitting no longer waits for the next block before the hashing threads stop.
- A node one block behind its own headers is no longer treated as syncing.
- A pool whose node falls behind pauses until it recovers, rather than exiting.

## Before the first release

Developed in stages, each with its result checked against real data:

- The BLAKE2b and SHA-256 primitives and the five-stage proof of work,
  reproducing real block hashes.
- The 164-byte v2 header, and mining on both sides of the fork on regtest.
- The pool and miner, split over Stratum V1.
- testnet4 block 150,616, mined here and accepted by an independent node.
- A mainnet node synced from a UTXO snapshot.
