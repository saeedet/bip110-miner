#!/usr/bin/env bash
#
# capture-vectors.sh — regenerate the real-block test vectors.
#
# The tests in crates/bip110-primitives/tests/real_blocks.rs hold headers pulled
# off a live chain. Regtest chains do not survive a datadir wipe, so this
# regenerates them rather than leaving the tests pinned to a chain that no
# longer exists.
#
# Usage:  ./scripts/capture-vectors.sh [from] [to]   (defaults 8 14)
#
# Deliberately captures blocks either side of the BLAKE2b activation height, so
# the suite covers both the 80-byte v1 header and the 164-byte v2 one — and the
# transition between them, which is the part most likely to be got wrong.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

FROM="${1:-8}"
TO="${2:-14}"
OUT="crates/bip110-primitives/tests/real_blocks.rs"

./scripts/node.sh cli regtest getblockcount > /dev/null 2>&1 \
  || { echo "error: no regtest node. Start one with ./scripts/node.sh start regtest" >&2; exit 1; }

TIP="$(./scripts/node.sh cli regtest getblockcount)"
[[ "$TO" -le "$TIP" ]] || { echo "error: chain is only $TIP blocks; asked for $TO" >&2; exit 1; }

echo "capturing heights $FROM..$TO into $OUT"

python3 - "$OUT" "$FROM" "$TO" <<'PY'
import re, subprocess, sys
out, first, last = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])

def cli(*args):
    return subprocess.run(["./scripts/node.sh", "cli", "regtest", *args],
                          capture_output=True, text=True, check=True).stdout.strip()

rows = []
for h in range(first, last + 1):
    bh = cli("getblockhash", str(h))
    rows.append(f'    ({h}, "{cli("getblockheader", bh, "false")}",\n        "{bh}"),')

source = open(out).read()
replaced = re.sub(r"(const BLOCKS: &\[\(u32, &str, &str\)\] = &\[\n).*?(\n\];)",
                  lambda m: m.group(1) + "\n".join(rows) + m.group(2),
                  source, flags=re.S)
open(out, "w").write(replaced)
print(f"wrote {len(rows)} blocks")
PY
