# bip110-miner

Mine the **BIP-110 fork of Bitcoin** (the BLAKE2b chain, ticker BTCB2) on your
Mac with one command. No Bitcoin experience needed: it sets up everything it
depends on, asks only about what's missing, and then shows a single live screen.

Everything Bitcoin-specific — the hash functions, the 164-byte header, the
block assembly, the pool protocol, the mining loop — is written from scratch in
Rust, so the whole path from a block template to a valid block can be read and
checked. Its sibling, [btc-miner](https://github.com/saeedet/btc-miner), does
the same for Bitcoin itself.

> **Honest version up front.** Finding a block is a lottery win, not a paycheck.
> A recent Mac would expect one roughly every 25,000 years at October 2026's
> difficulty. There is no partial credit and no slow drip of earnings. Run it
> because it's interesting to watch your own computer take part, not to earn.

```
╭─ bip110-miner ────────────────────────────── MAINNET · 14:23 UTC · up 2h14m ─╮
│  NODE  ● in sync · block 976,034 · 12 peers · Knots 29.4.2 · 5.1 GB          │
├─ MINING ─────────────────────────────────────────────────────────────────────┤
│   26.1 MH/s  ████████████████████████████████████  last 10 min               │
│   power  ● balanced (6 of 8 cores)         job: block 976,035 · 203 txs      │
├─ YOUR CHANCES ───────────────────────────────────────────────────────────────┤
│   This session    about 1 in 24 million of finding a block                   │
│   On average      one block every ≈ 5,945 years at this speed                │
│   Best hash yet   28 of the 62 zero bits needed                              │
│                   (each missing bit doubles it: 2³⁴ ≈ 17 billion× short)     │
├─ ALL TIME ───────────────────────────────────────────────────────────────────┤
│   3.04T hashes · 12 sessions · best ever 34 zero bits                        │
│   rewards to bc1qw508…f3t4 · spendable 45 days after a block is found        │
├─ RECENT ─────────────────────────────────────────────────────────────────────┤
│   14:22  someone else found block 976,034 → new job, nothing lost            │
│   14:15  someone else found block 976,033 → new job, nothing lost            │
│   14:12  new personal best: 28 zero bits                                     │
╰─ q quit · p pause · +/- power · ? what am I looking at ──────────────────────╯
```

*The live dashboard. The numbers here are illustrative.*

## What you need

- A Mac with Apple Silicon (M1 or later), on macOS.
- About **32 GB** of free disk for the blockchain on mainnet.
- An internet connection. The first sync downloads tens of gigabytes, though
  the node keeps only the most recent 10 GB of blocks.

## Install

There's no packaged release yet, so for now it is built from source. That needs
[Rust](https://rustup.rs):

```bash
git clone https://github.com/saeedet/bip110-miner.git
cd bip110-miner
cargo install --locked --path crates/cli
```

That puts `bip110-miner` in `~/.cargo/bin`.

## First run

```bash
bip110-miner
```

It checks four things, and asks about any that aren't ready:

1. **This computer** — the chip, and whether the disk has room.
2. **Node software** — the node is the program that talks to the network and
   checks every block itself. It's called Bitcoin Knots. If it's missing or out
   of date, it offers to download Knots 29.4.2 and installs it only if the file
   is byte for byte the official, signed release.
3. **Reward address** — where a reward goes if you find a block. It offers to
   create a wallet on your node, protected by a passphrase you choose. That's
   the recommended choice: an ordinary Bitcoin wallet app will accept a
   BIP-110 address, but it can't spend coins on this chain.
4. **Blockchain** — your node needs its own copy. A quick start loads a
   snapshot (about 10 GB, checked by the node itself) and gets you mining in
   hours instead of days. Mining starts on its own as soon as it's caught up.

Next time, with everything in place, it goes straight to the dashboard.

## Everyday use

| Command | What it does |
|---|---|
| `bip110-miner` | Set up anything missing, then mine |
| `bip110-miner setup` | Go through setup again — this is how you change where rewards go |
| `bip110-miner status` | Where the node and the chain are |
| `bip110-miner wallet` | Your reward address, and whether this node can spend it |
| `bip110-miner doctor` | Check everything mining depends on, with fixes for anything wrong |
| `bip110-miner stop` | Shut the node down |

While mining:

| Key | Does |
|---|---|
| `q` | Quit. The node keeps running, so the next start is instant |
| `p` | Pause or resume |
| `+` `-` | Power: eco, balanced or max. Remembered for next time |
| `?` | Explain what's on screen |

Useful options: `--power eco|balanced|max`, `--threads N`, `--allow-sleep` (by
default the Mac is kept awake while mining), and `--plain` for a log instead of
the dashboard. When the output isn't a terminal — a log file, a service — it
prints the log automatically. `--network testnet4` or `--network regtest` mines a
test network instead.

Stopping costs nothing. Every hash is a fresh lottery ticket, so an hour today
and an hour next month are worth exactly what two hours now would be.

## Where things live

| Path | What |
|---|---|
| `~/.bip110-miner/config.toml` | Your choices: network, power, reward address |
| `~/.bip110-miner/lifetime.json` | Hashes and best results across every session |
| `~/.bip110-miner/knots/` | The Knots version it installed |
| `~/.bip110` | The node's data: the blockchain, and any wallet on it |
| `~/bip110-rewards.backup` | A backup of the wallet setup created. Keep a copy elsewhere |

`~/.bip110` is deliberately not Knots' default folder. This chain shares
Bitcoin's network, and a node pointed at a Bitcoin data folder would start,
connect, and quietly disagree about which chain is real.

## Uninstall

```bash
bip110-miner stop
cargo uninstall bip110-miner
```

Then delete `~/.bip110-miner`, and `~/.bip110` for the blockchain. **If your
wallet holds anything, back it up and move it first** — it lives in `~/.bip110`.

## Learn more

- [docs/chain.md](docs/chain.md) — what this chain is, the rules a miner must
  follow, how Stratum maps onto it, and the measured odds
- [docs/proof-of-work.md](docs/proof-of-work.md) — the five-stage proof of work
- [docs/architecture.md](docs/architecture.md) — how the code fits together
- [docs/cli-design.md](docs/cli-design.md) — the screens, as designed

[Contributing](CONTRIBUTING.md) · [Security](SECURITY.md) ·
[Changelog](CHANGELOG.md) · [MIT License](LICENSE)
