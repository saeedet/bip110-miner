#!/usr/bin/env bash
#
# fork-crossing-test.sh — mine a regtest chain from genesis across the fork.
#
# The project's ordinary regtest chain is already past its activation height,
# so mining on it only ever exercises the post-fork path. Three things live
# below or exactly at that boundary and are invisible from up there:
#
#   * the 80-byte v1 header, hashed with SHA-256d rather than BLAKE2b
#   * the v1 -> v2 crossing itself
#   * the headline the coinbase must carry at exactly the activation height
#
# All three were broken at some point and none of them failed loudly. This
# builds a throwaway chain with the fork at height 5, mines through it, and
# checks the node's stored hashes against the miner's own.
#
# Usage:  ./scripts/fork-crossing-test.sh
#
# Leaves nothing behind: the datadir is a temp directory and is removed on exit.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KNOTS="${BTCB2_KNOTS:-$HOME/Projects/bitcoinknots/build/bin}"
HEADLINE="fork crossing test"
FORK_HEIGHT=5
BLOCKS=8

die() { echo "error: $*" >&2; exit 1; }

[[ -x "$KNOTS/bitcoind" ]] || die "no bitcoind at $KNOTS (set BTCB2_KNOTS)"

WORK="$(mktemp -d)"
CONF="$WORK/btcb2.conf"
DATADIR="$WORK/datadir"
mkdir -p "$DATADIR"

cleanup() {
  "$KNOTS/bitcoin-cli" -datadir="$DATADIR" -conf="$CONF" stop >/dev/null 2>&1 || true
  # Give it a moment to flush and release the datadir lock before removal.
  for _ in $(seq 1 20); do
    pgrep -f "datadir=$DATADIR" >/dev/null 2>&1 || break
    sleep 0.5
  done
  rm -rf "$WORK"
}
trap cleanup EXIT

# Derived from the project's own regtest config so the two cannot drift, with
# the activation height lowered and the headline made explicit.
sed -e "s/testactivationheight=blake2b@[0-9]*/testactivationheight=blake2b@$FORK_HEIGHT/" \
    -e "s/^blake2b_headline=.*/blake2b_headline=$HEADLINE/" \
    "$REPO_ROOT/config/btcb2.regtest.conf" > "$CONF"

cli() { "$KNOTS/bitcoin-cli" -datadir="$DATADIR" -conf="$CONF" "$@"; }

echo "fork at height $FORK_HEIGHT, datadir $DATADIR"
"$KNOTS/bitcoind" -datadir="$DATADIR" -conf="$CONF" -daemon >/dev/null

for _ in $(seq 1 60); do
  cli getblockcount >/dev/null 2>&1 && break
  sleep 1
done
cli getblockcount >/dev/null 2>&1 || die "the node never answered RPC"

# The miner prints the hash it computed itself; the node stores the hash it
# computed. Comparing them is the whole point — a miner can be wrong about the
# algorithm and still have a block accepted, because on regtest roughly half of
# all hashes meet the target anyway.
BTCB2_DATADIR="$DATADIR" BTCB2_HEADLINE="$HEADLINE" \
  "$REPO_ROOT/target/release/regtest-miner" "$BLOCKS" \
  | tee "$WORK/mined.txt"

echo
echo "checking the node agrees, block by block"

failures=0
for height in $(seq 1 "$BLOCKS"); do
  node_hash="$(cli getblockhash "$height")"

  if ! grep -q "$node_hash" "$WORK/mined.txt"; then
    echo "  MISMATCH at height $height: the node stored $node_hash,"
    echo "            which the miner never reported computing"
    failures=$((failures + 1))
    continue
  fi

  # The RPC reports `header_version` as 0 for a pre-fork header and 2 for a
  # post-fork one — 0 rather than 1, and present rather than omitted, which is
  # worth pinning down here because it is not what either name suggests.
  header="$(cli getblockheader "$node_hash")"
  version="$(printf '%s' "$header" | sed -n 's/.*"header_version": *\([0-9]*\).*/\1/p')"
  version="${version:-0}"

  expected=$([[ "$height" -ge "$FORK_HEIGHT" ]] && echo 2 || echo 0)
  if [[ "$version" != "$expected" ]]; then
    echo "  height $height: header_version $version, expected $expected"
    failures=$((failures + 1))
    continue
  fi

  label=$([[ "$expected" -eq 2 ]] && echo v2 || echo v1)
  echo "  height $height  ok  ($label header, hash matches)"
done

# The headline is required at exactly one height. Present anywhere else is
# harmless but wrong; absent at the fork block would be `bad-headline`.
echo
for height in $((FORK_HEIGHT - 1)) "$FORK_HEIGHT" $((FORK_HEIGHT + 1)); do
  coinbase="$(cli getblock "$(cli getblockhash "$height")" 2 \
    | sed -n 's/.*"coinbase": *"\([0-9a-f]*\)".*/\1/p' | head -1)"
  ascii="$(printf '%s' "$coinbase" | xxd -r -p | tr -c '[:print:]' '.')"

  if [[ "$height" -eq "$FORK_HEIGHT" ]]; then
    if [[ "$ascii" == *"$HEADLINE"* ]]; then
      echo "  height $height  ok  headline present at the fork block"
    else
      echo "  height $height  MISSING headline: $ascii"
      failures=$((failures + 1))
    fi
  elif [[ "$ascii" == *"$HEADLINE"* ]]; then
    echo "  height $height  headline present where it should not be: $ascii"
    failures=$((failures + 1))
  else
    echo "  height $height  ok  no headline, as expected"
  fi
done

echo
if [[ "$failures" -eq 0 ]]; then
  echo "PASS — $BLOCKS blocks across the fork, every hash agreed with the node"
else
  echo "FAIL — $failures problem(s)"
  exit 1
fi
