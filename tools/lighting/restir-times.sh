#!/bin/sh
# GPU stamps for the ReSTIR product shade at 1440p.
# Lavapipe timestamps are not a 4070 result. This script does not run unless
# GENOS_RESTIR_TIMES=1, so a lavapipe clone does not spend a bench on it.
#
# The light line is direct + hit light + the ReSTIR stamps. With
# restir_split_passes at 0 (the default), restir_temporal is empty and
# restir_initial is initial and temporal together. restir_spatial includes
# the checkerboard copy when that knob is 1.
#
# GENOS_RESTIR_COST=1 prints iteration counts on the first frames:
#   initial_iters, temporal_iters, spatial_phat, spatial_tries
#   previous_full_window_scan is the old full-resolution radius-2 pool scan.
set -eu
echo "restir-times: neighbors=${GENOS_RESTIR_SPATIAL_NEIGHBORS:-4} radius=${GENOS_RESTIR_SPATIAL_RADIUS:-2} scale=${GENOS_RESTIR_RESOLUTION_SCALE:-1} candidates=${GENOS_RESTIR_CANDIDATES:-4} checkerboard=${GENOS_RESTIR_SPATIAL_CHECKERBOARD:-0} split=${GENOS_RESTIR_SPLIT_PASSES:-0}"
if [ "${GENOS_RESTIR_TIMES:-}" != 1 ]; then
    echo "restir-times: set GENOS_RESTIR_TIMES=1 to run the 1440p stamps. Lavapipe time is not a gate."
    exit 0
fi
export GENOS_RESTIR_COST=1
root=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
cd "$root"
# GI off, then GI on. 90 frames, a stamp line every 30.
cargo run --release -p genos-stress -- --gpu-times 30 --size 2560x1440 --frames 90 --preset potato --bounces 0
cargo run --release -p genos-stress -- --gpu-times 30 --size 2560x1440 --frames 90 --preset high
