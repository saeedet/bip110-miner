#!/usr/bin/env bash
#
# sync-status.sh — where the node is, including the background history sync.
#
# Usage:
#   ./scripts/sync-status.sh [network] [sample-seconds]
#
# After `loadtxoutset` a node runs TWO chainstates, and `getblockchaininfo`
# only ever describes one of them:
#
#   snapshot    starts at the snapshot height, syncs to the tip. This is the
#               one that mines. It is "done" long before the node is.
#   background  starts at genesis and validates up to the snapshot height,
#               turning "I trust this snapshot" into "I checked it myself".
#
# So a node can report `initialblockdownload: false`, mine happily, and still
# be pulling several hundred gigabytes. That is normal and not a fault — but it
# is invisible unless you ask `getchainstates`, which is what this script is for.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NETWORK="${1:-mainnet}"
SAMPLE="${2:-30}"

cli() { "$REPO_ROOT/scripts/node.sh" cli "$NETWORK" "$@"; }

cli getblockcount >/dev/null 2>&1 || {
  echo "the $NETWORK node is not running (./scripts/node.sh start $NETWORK)" >&2
  exit 1
}

cli getblockchaininfo    > /tmp/.sync-status-bci.json
cli getchainstates       > /tmp/.sync-status-cs0.json
PEERS="$(cli getconnectioncount)"

# The background chainstate stops at the snapshot's BASE height, not at the
# chain tip — it exists only to verify the snapshot, and its work is done the
# moment it reaches the block the snapshot was taken at. `getchainstates` names
# that block but not its height, so resolve it.
SNAPSHOT_HASH="$(python3 -c "
import json
cs = json.load(open('/tmp/.sync-status-cs0.json'))['chainstates']
print(next((c['snapshot_blockhash'] for c in cs if c.get('snapshot_blockhash')), ''))
")"
BASE_HEIGHT=""
if [[ -n "$SNAPSHOT_HASH" ]]; then
  BASE_HEIGHT="$(cli getblockheader "$SNAPSHOT_HASH" | python3 -c "import sys,json;print(json.load(sys.stdin)['height'])" 2>/dev/null || true)"
fi

# A sample rather than an average: block rates swing wildly minute to minute,
# mostly because pruning stalls block connection while it flushes and deletes.
# Treat the ETA as an order of magnitude, not a countdown.
sleep "$SAMPLE"
cli getchainstates > /tmp/.sync-status-cs1.json

BIP110_PEERS="$PEERS" SAMPLE="$SAMPLE" BASE_HEIGHT="$BASE_HEIGHT" python3 - <<'PY'
import json, os

bci = json.load(open('/tmp/.sync-status-bci.json'))
cs0 = json.load(open('/tmp/.sync-status-cs0.json'))['chainstates']
cs1 = json.load(open('/tmp/.sync-status-cs1.json'))['chainstates']
sample = float(os.environ['SAMPLE'])
headers = bci['headers']

def pick(states, background):
    for c in states:
        if bool(c.get('snapshot_blockhash')) != background:
            return c
    return None

def human(seconds):
    for scale, name in ((86400, 'days'), (3600, 'hours'), (60, 'minutes')):
        if seconds >= scale:
            return f'{seconds/scale:.1f} {name}'
    return f'{seconds:.0f} seconds'

print(f"chain      : {bci['chain']}  height {bci['blocks']} / {headers}")
print(f"ibd        : {bci['initialblockdownload']}   peers {os.environ['BIP110_PEERS']}")
print(f"disk       : {bci['size_on_disk']/1e9:.1f} GB (pruning below {bci.get('pruneheight')})")

snap0, snap1 = pick(cs0, False), pick(cs1, False)
back0, back1 = pick(cs0, True), pick(cs1, True)

if back0 is None:
    print("\nno background chainstate — this node was not started from a snapshot,")
    print("so its history is already validated.")
else:
    # The target is the snapshot's base height. Using the chain tip instead
    # would understate progress and never reach 100%, since the background
    # chainstate stops as soon as it has verified the snapshot.
    target = int(os.environ.get('BASE_HEIGHT') or 0) or headers
    base = back1['blocks']
    rate = (back1['blocks'] - back0['blocks']) / sample * 60

    # Two progress figures, because they disagree by a lot and the block count
    # is the flattering one. `verificationprogress` is weighted by transaction
    # count, and Bitcoin's first few hundred thousand blocks are nearly empty —
    # so being half way by height is nowhere near half way by work.
    by_height = base * 100 / target
    by_work = back1['verificationprogress'] * 100

    print(f"\nbackground history validation")
    print(f"  progress : {base:,} of {target:,} blocks  ({by_height:.1f}% by height)")
    print(f"  ...but   : {by_work:.1f}% by transaction-weighted work — early blocks")
    print(f"             are nearly empty, so the height figure flatters it")
    print(f"  rate     : {rate:,.0f} blocks/min (sampled over {sample:.0f}s)")
    if rate > 0:
        print(f"  eta      : ~{human((target-base)/rate*60)} at that rate, but expect")
        print(f"             longer — later blocks are far bigger than these")
    else:
        print(f"  eta      : no blocks connected during this {sample:.0f}s sample.")
        print( "             Usually a prune flush rather than a stall; re-run,")
        print( "             or sample longer: ./scripts/sync-status.sh main 120")
    print("  note     : this does not affect mining. The chainstate that mines")
    print("             is already at the tip.")

if snap1:
    # Headers from the *same* call as the chainstate, so a tip that moved
    # between RPCs cannot show up as the miner falling behind.
    behind = json.load(open('/tmp/.sync-status-cs1.json'))['headers'] - snap1['blocks']
    state = 'at the tip' if behind <= 1 else f'{behind} blocks behind'
    print(f"\nmining chainstate: {snap1['blocks']:,} — {state}")
PY
