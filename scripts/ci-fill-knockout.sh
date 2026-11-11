#!/usr/bin/env bash
# The presented-frame fill gate passes with the scheduler on and fails when
# cache_fill is knocked out. Lavapipe, one test, headless Weston.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
if [[ -z "${WAYLAND_DISPLAY:-}" ]]; then
  export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-$(mktemp -d)}
  chmod 700 "$XDG_RUNTIME_DIR"
  weston --backend=headless --socket=genos-knock >"$root/genos-debug/fill-knockout-weston.log" 2>&1 &
  weston_pid=$!
  trap 'kill $weston_pid 2>/dev/null || true' EXIT
  for _ in $(seq 40); do [[ -S "$XDG_RUNTIME_DIR/genos-knock" ]] && break; sleep 0.25; done
  export WAYLAND_DISPLAY=genos-knock
fi
export VK_ICD_FILENAMES=${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/lvp_icd.json}
export VK_DRIVER_FILES=${VK_DRIVER_FILES:-$VK_ICD_FILENAMES}
mkdir -p "$root/genos-debug"

echo "fill knockout: guards on"
cargo test -p genos-stress --test fill_knockout -- --test-threads=1 --nocapture

echo "fill knockout: cache_fill removed"
set +e
GENOS_KNOCKOUT=cache_fill cargo test -p genos-stress --test fill_knockout --features knockout -- --test-threads=1 --nocapture
code=$?
set -e
if [[ $code -eq 0 ]]; then
  echo "fill knockout: cache_fill removed and the presented frame still passed" >&2
  exit 1
fi
echo "fill knockout: guards on passed, cache_fill knocked out went red ($code)"
