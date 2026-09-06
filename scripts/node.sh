#!/usr/bin/env bash
#
# node.sh — control the BLAKE2b-fork node this project uses.
#
# The binary is Bitcoin Knots built from source, because no package manager
# ships it and the BLAKE2b proof-of-work only exists in that fork. Point
# BTCB2_KNOTS at the build if it lives somewhere other than the default.
#
# A DEDICATED data directory (~/.btcb2) keeps this entirely separate from any
# Bitcoin node on the machine. That matters more than usual here: this fork
# shares Bitcoin's P2P network and message format, so a datadir mix-up would
# not fail loudly — it would quietly sync the wrong chain.
#
# Usage:
#   ./scripts/node.sh start  [network]
#   ./scripts/node.sh stop   [network]
#   ./scripts/node.sh status [network]
#   ./scripts/node.sh cli    [network] <args...>
#
# `network` is regtest (default), testnet, or mainnet.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMMAND="${1:-}"
NETWORK="${2:-regtest}"

DATADIR="${BTCB2_DATADIR:-$HOME/.btcb2}"
CONF="$REPO_ROOT/config/btcb2.$NETWORK.conf"
KNOTS="${BTCB2_KNOTS:-$HOME/Projects/bitcoinknots/build/bin}"

die() { echo "error: $*" >&2; exit 1; }

[[ -x "$KNOTS/bitcoind" ]] || die "no bitcoind at $KNOTS. Build Knots, or set BTCB2_KNOTS."
[[ -f "$CONF" ]] || die "no config for network '$NETWORK' (expected $CONF)"

ARGS=(-datadir="$DATADIR" -conf="$CONF")

case "$COMMAND" in
  start)
    mkdir -p "$DATADIR"
    echo "starting bitcoind (Knots/BLAKE2b)  network=$NETWORK  datadir=$DATADIR"
    "$KNOTS/bitcoind" "${ARGS[@]}" -daemon

    # bitcoind forks immediately, but RPC is not up until the block index has
    # loaded, so poll rather than guess.
    for _ in $(seq 1 300); do
      if "$KNOTS/bitcoin-cli" "${ARGS[@]}" getblockchaininfo >/dev/null 2>&1; then
        echo "RPC is up."
        exec "$KNOTS/bitcoin-cli" "${ARGS[@]}" getblockchaininfo
      fi
      sleep 1
    done
    die "bitcoind did not answer RPC in 300s — check $DATADIR/$NETWORK/debug.log"
    ;;

  stop)   "$KNOTS/bitcoin-cli" "${ARGS[@]}" stop ;;
  status) "$KNOTS/bitcoin-cli" "${ARGS[@]}" getblockchaininfo ;;
  cli)    "$KNOTS/bitcoin-cli" "${ARGS[@]}" "${@:3}" ;;

  *) sed -n '3,22p' "${BASH_SOURCE[0]}"; exit 1 ;;
esac
