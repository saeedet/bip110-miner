#!/usr/bin/env bash
#
# mine.sh — start the pool and a miner together, and stop them together.
#
# Usage:
#   ./scripts/mine.sh [network] [--threads N|half|max] [--address ADDR]
#
# `network` is mainnet (default), testnet4, or regtest.
#
# The payout address is read, in order of preference, from:
#   1. --address
#   2. $BIP110_PAYOUT_ADDRESS
#   3. ~/.bip110-miner/payout.<network>
#
# The file is the convenient option: an address in a shell command ends up in
# shell history, and so does everything else on that line.
#
# The node is started if it is not already running, and is left running
# afterwards — it is not part of a mining session.
#
# Ctrl-C stops the miner and the pool together. Nothing is lost by stopping —
# mining is memoryless, so an hour now and an hour next month are worth exactly
# what two hours today would be.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

NETWORK="mainnet"
THREADS=""
ADDRESS=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    mainnet|testnet4|regtest) NETWORK="$1"; shift ;;
    --threads) THREADS="${2:-}"; shift 2 ;;
    --address) ADDRESS="${2:-}"; shift 2 ;;
    -h|--help) sed -n '3,22p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "error: unknown argument $1" >&2; exit 1 ;;
  esac
done

die() { echo "error: $*" >&2; exit 1; }

# --- The RPC port --------------------------------------------------------
#
# This chain kept Bitcoin's RPC ports, so a machine running nodes for both
# collides and one of them has to move. `config/bip110.mainnet.conf` moves this
# one to 8342, and the client has to be told the same thing or it goes looking
# at Bitcoin's node instead.
#
# This is the single sharpest footgun in the project, which is the main reason
# this script exists. Getting it wrong does not mine the wrong chain — the
# per-datadir cookie rejects the connection — but it does fail confusingly.
if [[ -z "${BIP110_RPCPORT:-}" ]]; then
  CONF="$REPO_ROOT/config/bip110.$NETWORK.conf"
  [[ -f "$CONF" ]] || die "no config for network '$NETWORK' (expected $CONF)"
  PORT="$(sed -n 's/^rpcport=\([0-9]*\).*/\1/p' "$CONF" | tail -1)"
  if [[ -n "$PORT" ]]; then
    export BIP110_RPCPORT="$PORT"
    echo "rpc port $PORT (from $(basename "$CONF"))"
  fi
fi

# --- The payout address --------------------------------------------------
ADDRESS_FILE="$HOME/.bip110-miner/payout.$NETWORK"
if [[ -z "$ADDRESS" ]]; then
  ADDRESS="${BIP110_PAYOUT_ADDRESS:-}"
fi
if [[ -z "$ADDRESS" && -r "$ADDRESS_FILE" ]]; then
  ADDRESS="$(tr -d '[:space:]' < "$ADDRESS_FILE")"
  echo "payout address from $ADDRESS_FILE"
fi
# Regtest generates its own throwaway address, so only the real networks need one.
if [[ -z "$ADDRESS" && "$NETWORK" != "regtest" ]]; then
  die "no payout address. Pass --address, set BIP110_PAYOUT_ADDRESS, or write one to $ADDRESS_FILE"
fi

# --- The node ------------------------------------------------------------
#
# Started if it is not already up, and deliberately NOT stopped on the way out.
# A node is not part of a mining session: stopping it would drop its peers and
# leave the chain to go stale, so the next run pays to catch up. The pool and
# the miner are the session; the node outlives it.
#
# Whether the node is *fit* to mine on is not decided here. The pool checks the
# chain, the sync state, the peer count and the tip's age in one place
# (crates/pool/src/readiness.rs), and a weaker second opinion here would only
# give two answers that can drift apart.
if ! "$REPO_ROOT/scripts/node.sh" cli "$NETWORK" getblockcount >/dev/null 2>&1; then
  echo "starting the $NETWORK node..."
  "$REPO_ROOT/scripts/node.sh" start "$NETWORK" >/dev/null 2>&1 \
    || die "could not start the $NETWORK node — try ./scripts/node.sh start $NETWORK to see why"
  echo "node up at height $("$REPO_ROOT/scripts/node.sh" cli "$NETWORK" getblockcount)"
else
  echo "node already running at height $("$REPO_ROOT/scripts/node.sh" cli "$NETWORK" getblockcount)"
fi

# --- Build ---------------------------------------------------------------
#
# The whole point of this project is editing the code, and running a stale
# binary against fresh source is a confusing way to spend an evening.
echo "building..."
cargo build --release --quiet

# A pool left over from a previous run holds the port and the next start fails
# with "Address already in use", which reads like a bug in the pool rather than
# a leftover process.
if pgrep -f "target/release/pool --network" >/dev/null 2>&1; then
  echo "stopping a pool left over from a previous run"
  pkill -f "target/release/pool --network" || true
  sleep 1
fi

# --- Run -----------------------------------------------------------------
POOL_LOG="$(mktemp -t bip110-pool)"

# Everything started here must die with the script. The miner below is NOT
# exec'd: exec would replace this shell, taking the trap with it, and the pool
# would outlive Ctrl-C and hold the port against the next run.
CLEANED_UP=false

cleanup() {
  # A signal handler that exits triggers the EXIT trap as well, so this runs
  # twice without a guard: two pkills racing and duplicate messages.
  [[ "$CLEANED_UP" == true ]] && return
  CLEANED_UP=true
  echo
  echo "stopping..."

  # Kill every child rather than named PIDs. In `a | b &`, `$!` is the PID of
  # `b` only, so killing it leaves `a` alive — and a surviving `tail -f` holds
  # this script's stdout open, so whatever reads it never sees EOF.
  pkill -P $$ 2>/dev/null || true
  wait 2>/dev/null || true

  echo "pool log kept at $POOL_LOG"
}
# A signal handler that does not exit would let bash resume after the
# interrupted command, so stopping is made explicit. The statuses are the
# conventional 128 + signal number, kept distinct so a supervisor can tell
# "the user interrupted this" from "we terminated it".
trap 'cleanup; exit 130' INT
trap 'cleanup; exit 143' TERM
trap cleanup EXIT

POOL_ARGS=(--network "$NETWORK")
[[ -n "$ADDRESS" ]] && POOL_ARGS+=(--address "$ADDRESS")

./target/release/pool "${POOL_ARGS[@]}" > "$POOL_LOG" 2>&1 &

# Wait for the pool to announce that it has *built a job*, not merely that its
# process exists. A pool whose node is unreachable can bind the port and look
# perfectly healthy while never producing work, and a miner pointed at it would
# hash nothing while reporting a fine hashrate.
echo "waiting for the pool to get work..."
READY=""
for _ in $(seq 1 90); do
  grep -q "POOL READY" "$POOL_LOG" 2>/dev/null && { READY=1; break; }
  pgrep -P $$ -f "target/release/pool" >/dev/null 2>&1 || break
  sleep 1
done

if [[ -z "$READY" ]]; then
  echo "--- pool log ---" >&2
  cat "$POOL_LOG" >&2
  die "the pool never produced a job (is the node running and synced?)"
fi

sed -n '1,8p' "$POOL_LOG"

# The pool's own output keeps flowing alongside the miner's, so a block being
# found is visible without going to look for the log.
tail -f "$POOL_LOG" &

MINER_ARGS=(--pool 127.0.0.1:3334 --worker "$(hostname -s).0")
[[ -n "$THREADS" ]] && MINER_ARGS+=(--threads "$THREADS")

# Backgrounded and waited on rather than run in the foreground: bash defers
# trap handlers until a foreground child exits, so Ctrl-C would otherwise do
# nothing until the miner happened to stop by itself.
./target/release/miner "${MINER_ARGS[@]}" &
wait $!
