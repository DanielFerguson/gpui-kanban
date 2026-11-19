#!/usr/bin/env bash
# Runs the stress matrix against the release build and prints the last few
# once-a-second [perf] lines of each run (the steady state).
#
# Keep the window visible while this runs: GPUI stops drawing windows that are
# fully covered, which would make the numbers meaningless.
#
#   scripts/bench.sh                     # 1k, 10k, 100k cards; idle and chaos
#   COUNTS="5000" SECS=10 scripts/bench.sh
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release --quiet

secs=${SECS:-6}
for n in ${COUNTS:-1000 10000 100000}; do
  for mode in idle chaos; do
    flags=(--stress "$n" --exit-after "$secs")
    if [[ $mode == chaos ]]; then flags+=(--chaos); fi
    echo "== $n cards, $mode"
    ./target/release/gpui-kanban "${flags[@]}" | grep '^\[perf\]' | tail -3 || echo "(no frames)"
  done
done
