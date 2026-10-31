#!/usr/bin/env bash
# Lighting harness and oracle (compute-rethink stage A), headless on any Vulkan
# driver; CI and the box use lavapipe under a headless Weston.
#   scripts/lighting/oracle_run.sh [quick|full] [OUT]
# quick: sun 100x and 10x, a reference every 12th frame (about 1 h on lavapipe).
# full: adds sun 1x (7200 frames) and a reference on every frame (many hours).
# lamps: lighting_oracle_lamps.rhai, spin and switch with 32 lamps so pixels take
# the random lamp path instead of the exact sum (about 30 min on lavapipe).
# Writes OUT/metrics.json (compact, one object) and prints it.
# ORACLE_REF_EVERY_FRAME="sun-x100 ..." traces those scenarios on every frame, so
# oracle_metrics.py can report grid_excess. ORACLE_ONLY="sun-x100 ..." runs only
# those scenarios (quick and full profiles).
# GENOS_STRESS_BIN overrides the binary (default target/release/genos-stress).
# LAVAPIPE=0 keeps the session's Vulkan driver and display.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
profile=${1:-quick}
out=$(realpath -m "${2:-$root/genos-debug/oracle/$profile}")
case "$profile" in quick|full) source=lighting_oracle.rhai;; lamps) source=lighting_oracle_lamps.rhai;;
  *) echo "profile is quick, full or lamps" >&2; exit 2;; esac
bin=${GENOS_STRESS_BIN:-$root/target/release/genos-stress}
[[ -x "$bin" ]] || (cd "$root" && cargo build --release -p genos-stress)
if [[ "${LAVAPIPE:-1}" == 1 ]]; then
  export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json
  export VK_DRIVER_FILES=$VK_ICD_FILENAMES
  if [[ -z "${WAYLAND_DISPLAY:-}" ]]; then
    export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-$(mktemp -d)}
    chmod 700 "$XDG_RUNTIME_DIR"
    weston --backend=headless --socket=genos-oracle >"$out.weston.log" 2>&1 &
    weston_pid=$!
    trap 'kill $weston_pid 2>/dev/null || true' EXIT
    for _ in $(seq 20); do [[ -S "$XDG_RUNTIME_DIR/genos-oracle" ]] && break; sleep 0.5; done
    export WAYLAND_DISPLAY=genos-oracle
  fi
fi
export GENOS_GI_METRICS=1
mkdir -p "$out"
rhai_list() { local o="" w; for w in $1; do o+="${o:+, }\"$w\""; done; echo "[$o]"; }
work=$(mktemp -d)
for pass in temporal oracle; do
  script="$work/${source%.rhai}_$pass.rhai"
  sed -e "s/^let EVERY_FRAME = .*/let EVERY_FRAME = $(rhai_list "${ORACLE_REF_EVERY_FRAME:-}");/" \
    -e "s/^let ONLY = .*/let ONLY = $(rhai_list "${ORACLE_ONLY:-}");/" \
    -e "s/^let PASS = .*/let PASS = \"$pass\";/" -e "s/^let PROFILE = .*/let PROFILE = \"$profile\";/" \
    "$root/examples/stress/scripts/$source" >"$script"
  # gi_metrics writes genos-debug/gi/ under the working directory.
  (cd "$work" && "$bin" --proof --no-panel --size 160x90 --report "$out" --script "$script") \
    >"$out/$pass.log" 2>&1 || { echo "pass $pass failed, see $out/$pass.log and $out/report.md" >&2; exit 1; }
  cp "$out/report.md" "$out/report-$pass.md"
done
mkdir -p "$out/gi"; cp "$work"/genos-debug/gi/*-frames.csv "$out/gi/" 2>/dev/null || true
rm -rf "$work"
python3 "$root/scripts/lighting/oracle_metrics.py" "$out" | tee "$out/metrics.json"
