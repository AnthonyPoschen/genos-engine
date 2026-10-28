#!/usr/bin/env bash
# Score the sun-cycle and spin repros. Usage:
#   scripts/lighting/sun_cycle_grid.sh <debug-out-dir>
# where <debug-out-dir> holds sun-x100, sun-x10, sun-x1, spin-moving, spin-stop
# from examples/stress/scripts/{sun_cycle_grid,spin_converge}.rhai.
# Gates (worst frame over the whole cycle): grid <= 0.15 at every speed (100x is
# the stress case); spin-stop converges within 30 frames (0.5 s).
set -u
d=${1:?debug out dir}
here=$(dirname "$0")
rc=0
for s in 100 10 1; do
  stride=1; [ "$s" = 1 ] && stride=4   # 1x moves 0.05 deg a frame: compare 4 apart
  echo -n "sun x$s: "; python3 "$here/grid_metric.py" "$d/sun-x$s" --stride $stride --max-grid 0.15 || rc=1
done
echo -n "spin moving: "; python3 "$here/grid_metric.py" "$d/spin-moving" --max-grid 0.15 || rc=1
echo -n "spin stop: "; python3 "$here/grid_metric.py" "$d/spin-stop" --max-converge 30 || rc=1
exit $rc
