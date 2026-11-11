#!/usr/bin/env bash
# The presented-frame fill gate passes with the scheduler on and fails when
# cache_fill is knocked out. Lavapipe, one test, headless Weston.
#
# The knocked-out run counts as red only when the test itself failed on its
# presented-frame assertion. A compile error, a missing Vulkan driver or a
# crash also exits non-zero, and none of those proves the gate reads the frame.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
# Before the weston redirect: genos-debug/ is gitignored, so a clean checkout has none.
mkdir -p "$root/genos-debug"
if [[ -z "${WAYLAND_DISPLAY:-}" ]]; then
  export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-$(mktemp -d)}
  chmod 700 "$XDG_RUNTIME_DIR"
  weston --backend=headless --socket=genos-knock >"$root/genos-debug/fill-knockout-weston.log" 2>&1 &
  weston_pid=$!
  trap 'kill $weston_pid 2>/dev/null || true' EXIT
  for _ in $(seq 40); do [[ -S "$XDG_RUNTIME_DIR/genos-knock" ]] && break; sleep 0.25; done
  [[ -S "$XDG_RUNTIME_DIR/genos-knock" ]] || { echo "fill knockout: weston did not start" >&2; cat "$root/genos-debug/fill-knockout-weston.log" >&2; exit 1; }
  export WAYLAND_DISPLAY=genos-knock
fi
export VK_ICD_FILENAMES=${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/lvp_icd.json}
export VK_DRIVER_FILES=${VK_DRIVER_FILES:-$VK_ICD_FILENAMES}

echo "fill knockout: guards on"
cargo test --locked -p genos-stress --test fill_knockout -- --test-threads=1 --nocapture

echo "fill knockout: cache_fill removed"
log="$root/genos-debug/fill-knockout-red.log"
set +e
GENOS_KNOCKOUT=cache_fill cargo test --locked -p genos-stress --test fill_knockout --features knockout -- --test-threads=1 --nocapture 2>&1 | tee "$log"
code=${PIPESTATUS[0]}
set -e
if [[ $code -eq 0 ]]; then
  echo "fill knockout: cache_fill removed and the presented frame still passed" >&2
  exit 1
fi
# The marker is the prefix of every assertion message in fill_knockout.rs.
if ! grep -q "fill gate red:" "$log"; then
  echo "fill knockout: the knocked-out run failed ($code), but not on the presented-frame assertion" >&2
  exit 1
fi
echo "fill knockout: guards on passed, cache_fill knocked out went red on the presented frame"
