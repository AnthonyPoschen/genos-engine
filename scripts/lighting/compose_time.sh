#!/bin/sh
# GPU timestamp for lighting compose. Lavapipe times are not the number.
#
# GENOS_COMPOSE_TIME=1 runs the lighting debug binary and prints compose_ms
# from <out>/gi/frames.csv. Anything else prints the commits below and exits 0
# so CI does not bench. The binary's --help lists every argument. --gi-metrics
# wins over GENOS_GI_METRICS. --out wins over the default ./genos-debug.
# Nothing is written under $HOME.
#
# Commits, compose path:
#   959ec860  master. ~4 ms at 1440p, GI on.
#   e86a57b2  parent of the big step. Adds the coverage walk.
#   3c22b383  reject an empty-owner detail seed on every seed read. +46 ms.
#             Branches that contain it:
#               origin/cursor/d2-restir-port-note-aee3
#               origin/cursor/host-rhai-script-guards-f519
#               origin/cursor/restir-di-d2-port-52fc
#               origin/cursor/restir-live-history-proof-c1ce
#               origin/cursor/sh-mover-cell-aee3
#               origin/cursor/sh-probe-retire-cdf5
#               origin/cursor/sh-texel-probe-retire-7853
#               origin/cursor/sh-texel-ray-cut-c379
#               origin/cursor/sh-trilinear-neighbour-f7ed
#   b7ea9d7a  the seed, the coverage skip, and compose share one trilinear read
#             of eight cells. That is the walk the seed used to repeat.
#   66d817ce  later lighting-v3 head on restir-di-d2-port-52fc. ~180 ms.
#   be27e751  restir-live-history-proof-c1ce when this fix branched. ~180 ms.
#
# This fix should bring compose back near ~4 ms with GI on and sh_texel_detail
# off, and well under 180 ms with the flag on: one walk, not four.

set -eu

cat <<'EOF'
commits:
  959ec860 master ~4 ms
  e86a57b2 coverage walk
  3c22b383 empty-owner seed on every read, +46 ms
  b7ea9d7a eight-cell read shared by seed, coverage, and compose
  66d817ce ~180 ms (restir-di-d2-port-52fc)
  be27e751 ~180 ms (restir-live-history-proof-c1ce at branch point)
branches containing 3c22b383:
  origin/cursor/d2-restir-port-note-aee3
  origin/cursor/host-rhai-script-guards-f519
  origin/cursor/restir-di-d2-port-52fc
  origin/cursor/restir-live-history-proof-c1ce
  origin/cursor/sh-mover-cell-aee3
  origin/cursor/sh-probe-retire-cdf5
  origin/cursor/sh-texel-probe-retire-7853
  origin/cursor/sh-texel-ray-cut-c379
  origin/cursor/sh-trilinear-neighbour-f7ed
EOF

if [ "${GENOS_COMPOSE_TIME:-}" != "1" ]; then
    echo "skipped"
    exit 0
fi

out="${1:-./genos-debug}"
root=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
cd "$root"
cargo run --release -p genos-lighting -- \
    --headless \
    --preset high \
    --full-effort \
    --gi-metrics \
    --size 2560x1440 \
    --frames 30 \
    --out "$out"
csv="$out/gi/frames.csv"
echo "compose_ms from $csv"
awk -F, 'NR > 1 { print $1, $4 }' "$csv"
