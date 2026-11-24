#!/usr/bin/env bash
# Fails when a benchmark is slower than at the given base commit.
#
# Both commits run on the same machine: absolute times of a committed baseline would
# compare different runners.
set -euo pipefail

# Above the run-to-run noise of shared CI runners.
MAX_REGRESSION_PCT=20

base="${1:?usage: bench_gate.sh <base-commit>}"
root="$(git rev-parse --show-toplevel)"
worktree="$(mktemp -d)"
trap 'git -C "$root" worktree remove --force "$worktree"' EXIT
git -C "$root" worktree add --quiet --detach "$worktree" "$base"

export CARGO_TARGET_DIR="$root/target/bench-gate"
(cd "$worktree" && cargo bench --quiet -- --save-baseline base)
(cd "$root" && cargo bench --quiet -- --baseline-lenient base)

python3 - "$CARGO_TARGET_DIR/criterion" "$MAX_REGRESSION_PCT" <<'PYTHON'
import json, pathlib, sys

limit = float(sys.argv[2])
changes = {
    path.parent.parent.name: json.loads(path.read_text())["median"]["point_estimate"] * 100
    for path in pathlib.Path(sys.argv[1]).glob("*/change/estimates.json")
}
if not changes:
    sys.exit("no benchmark was compared")
for name, change in changes.items():
    print(f"{name}: {change:+.1f}% (limit {limit:+.0f}%)")
sys.exit(any(change > limit for change in changes.values()))
PYTHON
