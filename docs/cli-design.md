# CLI design

Applies to both `bip110-miner` and its sibling
[`btc-miner`](https://github.com/saeedet/btc-miner). Mockups are drawn at exact
terminal sizes, and the numbers in them are illustrative.

## Principles

1. **One command.** Run `bip110-miner`. It checks what is set up, asks about
   whatever isn't, and then mines. There is no separate setup step to
   remember, no file to edit, and no environment variable to export.
2. **One screen, updated in place.** Every screen fits 80×24 and redraws on
   resize. Larger terminals get more room, not more screens.
3. **No jargon without a translation.** Press `?` anywhere for a
   plain-language explanation of what's on screen.
4. **Visual, never misleading.** Progress bars only for things that really
   progress linearly, like a download. Odds are shown as odds. "28 of 62 zero
   bits" never gets a bar, because a bar would show 45% when the true gap is
   about 17 billion times.
5. **Careful with secrets.** Passphrases are typed masked, are never logged,
   and never appear on a command line or in shell history.
6. **Plain output when it isn't a terminal**, so it can run under logs, tmux
   or a service and still be readable.

## How a run flows

```
bip110-miner
  │
  ├─ checklist   This Mac · Node software · Blockchain · Reward address
  │      └─ anything missing or out of date? → ask about it, right there
  │
  ├─ blockchain not caught up? → progress screen; mining starts on its own
  │
  └─ dashboard
```

The next run with everything in place goes straight to the dashboard. If
something later needs attention — the node is out of date, the address file
has gone — only that item is asked about again.

Choices are remembered in one config file (`~/.bip110-miner/config.toml`),
written by the CLI. Nobody has to open it.

## Screens

### 1. First run — asking only for what's missing

```
╭─ bip110-miner · getting set up ───────────────────────────────────── 2 of 4 ─╮
│                                                                              │
│  Welcome! This turns your Mac into a BIP-110 miner.                          │
│  Honest version up front: finding a block is a lottery win, not a paycheck.  │
│                                                                              │
│  I'll only ask about what isn't set up yet.                                  │
│                                                                              │
│    ✔  This Mac          Apple M3 · 8 cores · 146 GB free                     │
│    ●  Node software     not installed                       ← now            │
│    ○  Blockchain        not downloaded                                       │
│    ○  Reward address    not set                                              │
│                                                                              │
├──────────────────────────────────────────────────────────────────────────────┤
│  The node is the program that talks to the BIP-110 network and checks        │
│  every block for itself. It's called Bitcoin Knots, and it's free.           │
│                                                                              │
│    ▸ Download the official build and verify its signature    (36 MB)         │
│      I'll install it myself — show me how                                    │
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
╰─ ↑↓ choose · enter confirm · ? why · q quit ─────────────────────────────────╯
```

### 2. Something needs attention later

The same checklist reappears when anything stops being true. This one is a
real case: Knots 29.4.2 changed a consensus rule at block 973,440, and a node
that hasn't been updated could build blocks the network rejects.

```
╭─ bip110-miner · needs attention ─────────────────────────────────────────────╮
│                                                                              │
│    ✔  This Mac          Apple M3 · 8 cores · 146 GB free                     │
│    ●  Node software     Knots 29.4.1 — out of date          ← now            │
│    ✔  Blockchain        in sync                                              │
│    ✔  Reward address    bc1qw50…f3t4                                         │
│                                                                              │
├──────────────────────────────────────────────────────────────────────────────┤
│  The network changed a rule at block 973,440, and this version of Knots      │
│  doesn't know about it. Mining with it could produce a block the rest of     │
│  the network rejects, so I've paused mining until it's updated.              │
│                                                                              │
│  What changed: newly mined coins now wait about 45 days (6,480 blocks)       │
│  before they can be spent, instead of about 16 hours.                        │
│                                                                              │
│    ▸ Update to Knots 29.4.2 and verify its signature     (36 MB)             │
│      Show me the release notes first                                         │
│      Not now — quit                                                          │
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
╰─ ↑↓ choose · enter confirm · ? why · q quit ─────────────────────────────────╯
```

### 3. Where rewards go

```
╭─ bip110-miner · getting set up ───────────────────────────────────── 4 of 4 ─╮
│                                                                              │
│    ✔  This Mac          Apple M3 · 8 cores · 146 GB free                     │
│    ✔  Node software     Knots 29.4.2 (signature verified)                    │
│    ✔  Blockchain        in sync                                              │
│    ●  Reward address    not set                              ← now           │
│                                                                              │
├──────────────────────────────────────────────────────────────────────────────┤
│  If your Mac ever finds a block, the reward goes to an address you own.      │
│                                                                              │
│  For BIP-110 coins, the safest home is a wallet on this node. A normal       │
│  Bitcoin wallet app will accept the address, but can't spend the coins.      │
│                                                                              │
│    ▸ Create a wallet here, protected by a passphrase   (recommended)         │
│      I already have a BIP-110 address — let me paste it                      │
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
╰─ ↑↓ choose · enter confirm · ? why · q quit ─────────────────────────────────╯
```

### 4. Creating a wallet

```
╭─ bip110-miner · creating your wallet ────────────────────────────────────────╮
│                                                                              │
│  Choose a passphrase. It encrypts the wallet stored on this Mac.             │
│                                                                              │
│      Passphrase     ••••••••••••••                                           │
│      Once more      ••••••••••••••                         ✔ match           │
│                                                                              │
│  • It's never shown, saved, logged, or sent anywhere.                        │
│  • You only need it to SPEND rewards, never to mine.                         │
│  • If you lose it, coins in this wallet are gone for good.                   │
│    Write it down somewhere safe now.                                         │
│                                                                              │
├──────────────────────────────────────────────────────────────────────────────┤
│  Next I'll save a backup copy of the wallet. Keep it somewhere other         │
│  than this Mac — a USB stick, or your password manager.                      │
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
╰─ enter continue · esc back ──────────────────────────────────────────────────╯
```

### 5. Getting the blockchain

Progress here is measured by work, not block count. The first half of the
chain is nearly empty blocks, so a count-based bar would race to 50% and then
crawl.

```
╭─ bip110-miner ──────────────────────────── MAINNET · getting the blockchain ─╮
│                                                                              │
│  Before mining, your Mac needs its own copy of the blockchain, so it can     │
│  check every block itself instead of trusting anyone else.                   │
│                                                                              │
│    Quick start        ████████████████████████████  done    9.0 GB           │
│    Catching up        ██████████████████░░░░░░░░░░  64%     ~1 h 50 m        │
│                       block 950,112 of 976,034 · 6.4 MB/s                    │
│                                                                              │
│  ⏵ Mining starts on its own when this reaches 100%.                          │
│                                                                              │
├─ LATER, IN THE BACKGROUND ───────────────────────────────────────────────────┤
│    Checking history   ░░░░░░░░░░░░░░░░░░░░░░░░░░░░  starts after catch-up    │
│                       re-verifies every block since 2009                     │
│                       you can mine while it runs                             │
│                                                                              │
│  Disk 5.1 GB of 10 GB budget · 146 GB free                                   │
│  Safe to quit at any time: progress is kept and picks up next run.           │
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
╰─ q quit · ? what's happening ────────────────────────────────────────────────╯
```

### 6. The dashboard — 80×24

The RECENT panel grows to fill whatever height the terminal has.

```
╭─ bip110-miner ────────────────────────────── MAINNET · 14:23 UTC · up 2h14m ─╮
│  NODE  ● in sync · block 976,034 · 12 peers · Knots 29.4.2 · 5.1 GB          │
├─ MINING ─────────────────────────────────────────────────────────────────────┤
│   26.1 MH/s  ▁▂▃▅▆▇▇▆▇▇▇▆▇▇▇▆▇▇▇▆▇▇▆▇▇▇▆▇▇▇▇▆▇▇▇  last 10 min                │
│   power  ● balanced (6 of 8 cores)          job: block 976,035 · 203 txs     │
├─ YOUR CHANCES ───────────────────────────────────────────────────────────────┤
│   This session    about 1 in 4.3 billion of finding a block                  │
│   On average      one block every ≈ 5,946 years at this speed                │
│   Best hash yet   28 of the 62 zero bits needed                              │
│                   (each missing bit doubles it: 2³⁴ ≈ 17 billion× short)     │
├─ ALL TIME ───────────────────────────────────────────────────────────────────┤
│   3.04 T hashes · 12 sessions · best ever 34 bits                            │
│   rewards to bc1qw50…f3t4 · spendable 45 days after a block is found         │
├─ RECENT ─────────────────────────────────────────────────────────────────────┤
│   14:22  someone else found block 976,034 → new job, nothing lost            │
│   14:15  someone else found block 976,033 → new job, nothing lost            │
│   14:12  new personal best: 28 zero bits                                     │
│   14:10  power set to balanced (6 of 8 cores)                                │
│   14:09  node in sync — mining started                                       │
│   14:08  node catching up: 4 blocks behind, mining paused                    │
│   14:08  bip110-miner 0.1.0 started                                          │
│                                                                              │
│                                                                              │
╰─ q quit · p pause · +/- power · ? what am I looking at ──────────────────────╯
```

### 6b. The dashboard — 120×40

```
╭─ bip110-miner ────────────────────────────────────────────────────────────────── MAINNET · 14:23:05 UTC · up 2h 14m ─╮
│  NODE  ● in sync · block 976,034 · tip 3 min old · 12 peers (9 BIP-110) · Knots 29.4.2 · 5.1 GB of 10 GB             │
├─ MINING ─────────────────────────────────────────────────┬─ YOUR CHANCES ────────────────────────────────────────────┤
│                                                          │                                                           │
│   26.1 MH/s   (average 25.8 this session)                │   This session                                            │
│   ▁▂▃▅▆▇▇▆▇▇▇▆▇▇▇▆▇▇▇▆▇▇▆▇▇▇▆▇▇▇▇▆▇▇▇▆▇▇                 │      about 1 in 4.3 billion of finding a block            │
│   last 30 minutes                                        │                                                           │
│                                                          │   On average                                              │
│   power   ○ eco   ● balanced   ○ max                     │      one block every ≈ 5,946 years at this speed          │
│           6 of 8 cores · press + or - to change          │                                                           │
│                                                          │   Best hash this session                                  │
│   job     block 976,035 · 203 transactions               │      28 of the 62 zero bits a block needs                 │
│           built by your node 52 s ago                    │      each missing bit doubles it: 2³⁴ ≈ 17 billion× short │
│                                                          │                                                           │
├─ ALL TIME ───────────────────────────────────────────────┴───────────────────────────────────────────────────────────┤
│   3.04 T hashes over 12 sessions · best ever 34 zero bits (2³² short) · about 1 in 15 million of a block so far      │
│   rewards go to bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4 · spendable 45 days after a block is found                │
├─ RECENT ─────────────────────────────────────────────────────────────────────────────────────────────────────────────┤
│   14:22:51  someone else found block 976,034 → new job, nothing lost                                                 │
│   14:15:09  someone else found block 976,033 → new job, nothing lost                                                 │
│   14:12:07  new personal best: 28 zero bits                                                                          │
│   14:10:44  power set to balanced (6 of 8 cores)                                                                     │
│   14:09:40  node in sync — mining started                                                                            │
│   14:08:31  node catching up: 4 blocks behind, mining paused                                                         │
│   14:08:02  bip110-miner 0.1.0 started                                                                               │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
╰─ q quit · p pause · +/- power · ? what am I looking at ──────────────────────────────────────────────────────────────╯
```

There is no "difficulty" figure. Knots 29.4.2 removed it for BLAKE2b blocks,
since SHA-256d difficulty units mean nothing here, and replaced it with the
expected number of hashes per block. That is what the odds are computed from,
and it says more to a newcomer than an abstract number would.

### 7. You found a block

```
╭─ bip110-miner ───────────────────────────────────────────────────── MAINNET ─╮
│                                                                              │
│                                                                              │
│                         ★  YOU FOUND A BLOCK  ★                              │
│                                                                              │
│               block 976,412 · 14:22:51 UTC · accepted by your node           │
│                                                                              │
│          reward    3.125 + 0.0012 in fees  →  bc1qw50…f3t4                   │
│          spendable after block 982,892 (about 45 days from now)              │
│          confirmations   0 — I'll keep watching it                           │
│                                                                              │
│          The chance of that this session was about 1 in 2 million.           │
│                                                                              │
├──────────────────────────────────────────────────────────────────────────────┤
│   If you haven't backed up your wallet yet, do it now:                       │
│        bip110-miner wallet backup                                            │
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
╰─ enter back to mining ───────────────────────────────────────────────────────╯
```

### 8. Help (`?`)

```
╭─ bip110-miner · what am I looking at? ───────────────────────────────────────╮
│                                                                              │
│  HASH       Your Mac scrambles the block's contents into a long number.      │
│             Each try is a "hash". It does about 26 million a second.         │
│                                                                              │
│  ZERO BITS  A block counts only if that number starts with enough            │
│             zeros. Each extra zero bit makes it twice as hard, which is      │
│             why 28 of 62 is nowhere near halfway.                            │
│                                                                              │
│  THE ODDS   Every hash is a fresh lottery ticket. Past tries don't           │
│             bring you closer, so stopping and restarting costs nothing.      │
│                                                                              │
│  THE NODE   Your own copy of the network's rules and history. It's           │
│             what lets you mine without trusting anyone.                      │
│                                                                              │
│  REWARDS    Paid only if you find a block — there is no partial credit,      │
│             and no slow drip of earnings. It's all or nothing.               │
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
│                                                                              │
╰─ esc close ──────────────────────────────────────────────────────────────────╯
```

## Keys

| Key | Does |
|---|---|
| `q` | quit — the node keeps running so the next start is instant |
| `p` | pause / resume mining |
| `+` `-` | power: eco · balanced · max |
| `?` | explain what's on screen |
| `esc` | back / close |

## Plain mode

When output isn't a terminal, the same information prints as ruled blocks:

```
──────────────────────────────────────────────────── 14:23:05 UTC
    26.10 MH/s (avg  25.80)   session    1.23G   best 28/62 bits
  0000000eb5b0f142a0b1db21b614adc0a69dfe262718468aee23847e4f4f806f
  lifetime    3.04T   best ever 34/62 bits (2^28 short)   ~1 in 1.5e7 of a block
```

## How btc-miner differs

Same screens and keys. The differences are in what gets checked and said:

| | bip110-miner | btc-miner |
|---|---|---|
| Node | Bitcoin Knots, downloaded and signature-checked | Bitcoin Core, via Homebrew |
| Recommended reward address | a wallet on the node (other apps can't spend fork coins) | paste one from any Bitcoin wallet app; a node wallet is optional |
| Rewards spendable after | 6,480 blocks, about 45 days | 100 blocks, about 16 hours |
| Expected time to a block (8 cores) | thousands of years | about 180 million years — said just as plainly |
