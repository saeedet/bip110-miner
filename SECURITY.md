# Security

## Reporting a problem

Please report security problems privately, through GitHub: the **Security**
tab of this repository, then **Report a vulnerability**. Don't open a public
issue for anything that could cost someone coins or expose their machine.

Only the latest release is supported.

A consensus bug in the node itself belongs with
[Bitcoin Knots](https://github.com/bitcoinknots/bitcoin/security).

## What this program does with your money and secrets

**It never holds a private key.** Wallets live in the node. When setup creates
one, the passphrase is typed into the program, shown only as dots, and sent
straight to your own node over its local RPC connection. It is never displayed,
written to a file, logged, or put on a command line where other programs or
your shell's history could see it.

**A reward address is checked before it is used.** The node validates it when
it is saved and again when mining starts, because a mistyped or wrong-network
address would produce perfectly valid blocks that pay nobody.

**Downloads are checked before they are trusted.**

- Bitcoin Knots is installed only if the downloaded file's SHA-256 matches the
  one built into this program. That hash was taken from the release's signed
  `SHA256SUMS`, after verifying the signature against the maintainer's key
  (fingerprint `1A3E 761F 19D2 CC77 85C5 502E A291 A2C4 5D0C 504A`). A file that
  doesn't match is deleted, never run.
- The quick-start UTXO snapshot comes from a mirror, but the mirror is not
  trusted: the node refuses any snapshot whose hash differs from the one
  compiled into it.

**Nothing listens to the outside world.** The pool accepts miners on 127.0.0.1
only. The node's RPC is local and needs the cookie file in its data folder, and
the mainnet node doesn't accept inbound peer connections.

**Your choices stay out of the repository.** Settings, the reward address and
lifetime totals live in `~/.bip110-miner`, and the blockchain and any wallet in
`~/.bip110`, never in this repository's folder.
