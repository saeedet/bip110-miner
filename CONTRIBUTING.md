# Contributing

Thanks for looking. This project is as much for reading as for running, so
clear code and honest explanations matter as much as features.

## Building and testing

You need an Apple Silicon Mac and [Rust](https://rustup.rs). Everything CI
checks can be run locally:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked --release
```

The dashboard and setup screens have snapshot tests: each screen is drawn at a
fixed size and compared, as text, with a file under `snapshots/`. After a
deliberate change to a screen, accept it with

```bash
UPDATE_SNAPSHOTS=1 cargo test -p bip110-miner
```

and review the changed files before committing them.

## Trying changes safely

Regtest is a private chain on your own machine, where blocks take a handful of
hashes. It needs no download and touches nothing real:

```bash
bip110-miner --network regtest
```

`scripts/fork-crossing-test.sh` mines a fresh chain across the fork. On regtest
roughly half of all hashes meet the target, so a block being accepted proves
little — a miner computing the wrong hash entirely still gets blocks in. What
proves the code right is the node's stored hash matching the miner's, block by
block, which is what that script checks.

## Code

- **Small files, one job each,** documented at the top with what they do and
  why. Every public item has a doc comment; the build warns otherwise.
- **Comments explain why,** not what. If a value was measured or read out of the
  node's source, say where.
- **Everything Bitcoin-specific is written here.** Dependencies are for generic
  jobs that teach nothing about mining — JSON, argument parsing, drawing the
  terminal. Please open an issue before adding one.
- **macOS assumptions live in `crates/cli/src/platform.rs`,** so a port to
  another system has one file to fill in.

### Shared with btc-miner

[btc-miner](https://github.com/saeedet/btc-miner) shares most of the command:
the `events` crate is identical, and `dashboard/` and `setup/` match file for
file. What differs between the chains lives in `chain.rs` and
`setup/install.rs`. A change to the shared parts should go to both projects.

## Commits

One logical change per commit, with a subject in the imperative ("Fix…",
"Add…") and a body that says why. Formatting changes go in their own commit.
