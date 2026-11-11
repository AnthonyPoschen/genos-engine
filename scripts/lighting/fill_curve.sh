#!/usr/bin/env bash
# Still hall plus lamp 3, headless on lavapipe.
#   scripts/lighting/fill_curve.sh OUT [--quarters on|off] [--budget N]
# OUT receives the pictures, run.log, and curve.json.
# GENOS_STRESS_BIN overrides the binary (default target/release/genos-stress).
# QUARTERS and BUDGET env vars are the same switches, and the flags win.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
out=$(realpath -m "${1:-$root/genos-debug/fill-curve}")
mkdir -p "$out" "$(dirname "$out")"
shift || true
quarters=${QUARTERS:-}
budget=${BUDGET:-}
while [[ $# -gt 0 ]]; do
  case "$1" in
    --quarters) quarters=$2; shift 2;;
    --budget) budget=$2; shift 2;;
    *) echo "unknown argument $1" >&2; exit 2;;
  esac
done
bin=${GENOS_STRESS_BIN:-$root/target/release/genos-stress}
[[ -x "$bin" ]] || (cd "$root" && cargo build --release -p genos-stress)
if [[ "${LAVAPIPE:-1}" == 1 ]]; then
  export VK_ICD_FILENAMES=${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/lvp_icd.json}
  export VK_DRIVER_FILES=${VK_DRIVER_FILES:-$VK_ICD_FILENAMES}
  if [[ -z "${WAYLAND_DISPLAY:-}" ]]; then
    export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-$(mktemp -d)}
    chmod 700 "$XDG_RUNTIME_DIR"
    weston --backend=headless --socket=genos-fill >"$out.weston.log" 2>&1 &
    weston_pid=$!
    trap 'kill $weston_pid 2>/dev/null || true' EXIT
    for _ in $(seq 40); do [[ -S "$XDG_RUNTIME_DIR/genos-fill" ]] && break; sleep 0.25; done
    export WAYLAND_DISPLAY=genos-fill
  fi
fi
export GENOS_LIGHTING_SC_TRACE=1
export GENOS_STRESS_DYNAMIC=${GENOS_STRESS_DYNAMIC:-0}
args=(
  --proof --no-panel --preset high --size 640x360
  --freeze-sun --still-boxes
  --script "$root/examples/stress/scripts/fill_curve.rhai"
  --report "$out"
)
if [[ -n "$quarters" ]]; then
  args+=(--lighting-bounded-quarters "$quarters")
fi
if [[ -n "$budget" ]]; then
  args+=(--surface-cache-ray-budget "$budget")
fi
set +e
"$bin" "${args[@]}" >"$out/run.log" 2>&1
code=$?
set -e
if [[ $code -ne 0 ]]; then
  echo "fill curve failed ($code), see $out/run.log" >&2
  tail -40 "$out/run.log" >&2
  exit "$code"
fi
label=flag_off
if [[ "$quarters" == "on" ]]; then
  label="bounded_quarters_${budget:-0}"
fi
python3 "$root/scripts/lighting/fill_curve.py" score "$out" \
  --reference "$root/docs/systems/lighting/experiments/img/hall-path-reference.png" \
  --label "$label" --log "$out/run.log" --json "$out/curve.json"
