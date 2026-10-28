#!/usr/bin/env bash
# Lighting harness and oracle (compute-rethink stage A), headless on any Vulkan
# driver; CI and the box use lavapipe under a headless Weston.
#   scripts/lighting/oracle_run.sh [quick|full] [OUT]
# quick: sun 100x and 10x, a reference every 12th frame (about 1 h on lavapipe).
# full: adds sun 1x (7200 frames) and a reference on every frame (many hours).
# Writes OUT/metrics.json (compact, one object) and prints it.
# GENOS_STRESS_BIN overrides the binary (default target/release/genos-stress).
# SPEC=1 also runs the specular scenarios (lighting_oracle_spec.rhai, stage S0).
# LAVAPIPE=0 keeps the session's Vulkan driver and display.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
profile=${1:-quick}
out=$(realpath -m "${2:-$root/genos-debug/oracle/$profile}")
case "$profile" in quick|full) ;; *) echo "profile is quick or full" >&2; exit 2;; esac
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
work=$(mktemp -d)
scripts=(lighting_oracle)
[[ "${SPEC:-0}" == 1 ]] && scripts+=(lighting_oracle_spec)
for name in "${scripts[@]}"; do
for pass in temporal oracle; do
  script="$work/${name}_$pass.rhai"
  sed -e "s/^let PASS = .*/let PASS = \"$pass\";/" -e "s/^let PROFILE = .*/let PROFILE = \"$profile\";/" \
    "$root/examples/stress/scripts/$name.rhai" >"$script"
  # gi_metrics writes genos-debug/gi/ under the working directory.
  (cd "$work" && "$bin" --proof --no-panel --size 160x90 --report "$out" --script "$script") \
    >"$out/$name-$pass.log" 2>&1 || { echo "$name $pass failed, see $out/$name-$pass.log and $out/report.md" >&2; exit 1; }
  cp "$out/report.md" "$out/report-$name-$pass.md"
done
done
mkdir -p "$out/gi"; cp "$work"/genos-debug/gi/*-frames.csv "$out/gi/" 2>/dev/null || true
rm -rf "$work"
python3 "$root/scripts/lighting/oracle_metrics.py" "$out" | tee "$out/metrics.json"
