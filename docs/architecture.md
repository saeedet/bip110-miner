# How bip110-miner fits together

For using the program, see the [README](../README.md). For the chain's rules
and how each one shows up here, see [chain.md](chain.md).

## One command, three parts

```
                      ┌──────────────── bip110-miner ─────────────────┐
Knots node  ──RPC──▶  │  pool  ──Stratum V1──▶  miner                 │
(~/.bip110)           │    │                      │                   │
                      │    └────── events ────────┴──▶  dashboard     │
                      │                                 or plain log  │
                      └───────────────────────────────────────────────┘
```

- **The node** is Bitcoin Knots, a separate program that keeps running between
  mining sessions so the next start is instant. It validates everything and
  builds block templates.
- **The pool** asks the node for a template, turns it into Stratum jobs, checks
  the shares that come back, and submits any that make a block.
- **The miner** hashes. It knows nothing about blocks or the node — only the
  jobs the pool sends it.

The pool and miner talk real Stratum V1 over TCP, even inside one process. That
keeps the protocol boundary honest: the miner could be swapped for any other
Stratum client without the pool changing. The pool listens on 127.0.0.1 only.

Neither half prints anything itself. Both emit `Event`s into a sink. The plain
sink prints the same lines the programs always printed; the dashboard folds the
same events into one screen. So a log and the dashboard can never disagree about
what happened.

## Crate map

| Crate | Responsibility |
|---|---|
| `blake2b` | BLAKE2b. Readable reference implementation and an optimised one, tested against each other |
| `sha256` | SHA-256 and BIP340 tagged hashes — the proof of work needs three of them |
| `bip110-primitives` | Both header formats and **both** proof-of-work algorithms: SHA-256d below the fork height, the five-stage BLAKE2b pipeline above it. Merkle trees, transactions, targets. Pure, no I/O |
| `node-rpc` | Typed JSON-RPC for `getblocktemplate` / `submitblock` and friends. Cookie auth; hand-rolled HTTP and base64 |
| `mining` | Coinbase construction, block assembly, nonce search. Pure, no I/O |
| `stratum` | Stratum V1 wire types, shared by pool and miner so they cannot disagree |
| `events` | What the pool and miner have to say, as data, and the plain renderer. Identical in btc-miner |
| `pool` | The solo pool: a node on one side, Stratum on the other. A library, with a thin `pool` binary |
| `miner` | The hashing client, with live controls for pause, stop and thread count. A library, with a thin `miner` binary |
| `cli` | The `bip110-miner` command: setup, the dashboard, and the other subcommands |
| `regtest-miner` | A self-contained miner for a local regtest chain, both sides of the fork |

Everything Bitcoin-specific is written here. The few dependencies do generic
jobs that teach nothing about mining: JSON (`serde`), argument parsing (`clap`),
the settings file (`toml`), Ctrl-C (`ctrlc`), and drawing the terminal
(`ratatui`).

## Inside the command

| Module | What it does |
|---|---|
| `setup/` | The checklist that runs before mining and asks only about what's missing |
| `dashboard/` | The live screen: `state.rs` turns events into what's shown, `view.rs` lays it out |
| `commands/` | One file per subcommand |
| `config.rs` | The settings file, and which source wins: flag, then environment, then file |
| `node.rs` | Finding, starting and stopping the node, and checking its version |
| `chain.rs` | Everything that differs from btc-miner's chain, in one file |
| `platform.rs` | Everything that assumes macOS, in one file — the place a Linux or Windows port starts |

The dashboard and setup screens are shared with btc-miner, file for file; only
`chain.rs` and `setup/install.rs` differ.

## Running the pieces by hand

The `pool` and `miner` binaries still exist, and are what to reach for when
debugging one side alone:

```bash
cargo run --release -p pool -- --network regtest
cargo run --release -p miner -- --pool 127.0.0.1:3334 --worker test.0
```

Port 3334, not Stratum's customary 3333: this chain already shares Bitcoin's
P2P magic and RPC ports — a hazard, not a convenience — and there was no reason
to add a third collision with btc-miner on the same machine. For the same
reason this project's regtest and testnet4 nodes use their own RPC ports (18453
and 48342), and its mainnet node uses 8342.

`scripts/` holds the development tools that predate the command:
`node.sh` controls a node with a repo config, `mine.sh` runs the pool and miner
as separate processes, `fork-crossing-test.sh` mines a fresh chain across the
fork and compares every block hash with the node's, and `sync-status.sh`
reports a sync.

## How it was built

- **0** — Toolchains, repo skeleton, BLAKE2b regtest node running
- **1** — `blake2b` + `sha256`: RFC 7693 vectors, BIP340 tagged hashes, and the full proof of work reproducing real block hashes
- **2** — `bip110-primitives`: the 164-byte header
- **3** — Mine a regtest block the node accepts — and one on each side of the fork
- **4** — Split into pool + miner over Stratum V1
- **5** — testnet4: mined block 150,616, accepted by an independent node
- **6** — Mainnet synced via assumeutxo
- **7** — One command, a live dashboard, and setup that asks for what's missing
