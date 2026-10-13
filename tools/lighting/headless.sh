#!/usr/bin/env bash
# Run a command on lavapipe under a headless Weston (no GPU, no display needed).
#
#   tools/lighting/headless.sh                 # the lighting correctness tests
#   tools/lighting/headless.sh still           # only the still-scene test (quick)
#   tools/lighting/headless.sh -- CMD ARGS...  # any command, e.g. genos-stress --script
#
# The window is a Wayland toplevel, so a compositor must exist; Weston's headless
# backend draws nothing and needs no GPU. VK_DRIVER_FILES pins Mesa's lavapipe even
# on a box that has a GPU (GENOS_HEADLESS_ICD=system keeps the system's drivers).
# Timing on lavapipe means nothing; these runs check correctness only.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"

icd=/usr/share/vulkan/icd.d/lvp_icd.json
[ -f "$icd" ] || icd="$(ls /usr/share/vulkan/icd.d/lvp_icd*.json 2>/dev/null | head -n1 || true)"
if [ "${GENOS_HEADLESS_ICD:-lavapipe}" != system ]; then
    if [ -z "$icd" ] || [ ! -f "$icd" ]; then
        echo "lavapipe is not installed (mesa-vulkan-drivers); see .cursor/install.sh" >&2
        exit 2
    fi
    export VK_DRIVER_FILES="$icd" VK_ICD_FILENAMES="$icd"
fi
command -v weston >/dev/null || { echo "weston is not installed; see .cursor/install.sh" >&2; exit 2; }

runtime="$(mktemp -d /tmp/genos-headless.XXXXXX)"
chmod 700 "$runtime"
socket=genos-headless
weston_pid=
cleanup() {
    [ -n "$weston_pid" ] && kill "$weston_pid" 2>/dev/null || true
    wait 2>/dev/null || true
    rm -rf "$runtime"
}
trap cleanup EXIT
export XDG_RUNTIME_DIR="$runtime"
unset DISPLAY
weston --backend=headless --socket="$socket" --idle-time=0 --width=1280 --height=720 \
    >"$runtime/weston.log" 2>&1 &
weston_pid=$!
for _ in $(seq 100); do
    [ -S "$runtime/$socket" ] && break
    sleep 0.1
done
if [ ! -S "$runtime/$socket" ]; then
    cat "$runtime/weston.log" >&2
    echo "headless weston did not start" >&2
    exit 2
fi
export WAYLAND_DISPLAY="$socket"

tests=(cargo test --release -p genos-stress --test gi_headless -- --ignored --nocapture --test-threads=1)
case "${1:-}" in
    "") "${tests[@]}" ;;
    still) "${tests[@]}" gi_still ;;
    moved) "${tests[@]}" gi_moved ;;
    --) shift; "$@" ;;
    *) echo "usage: $0 [still|moved|-- CMD...]" >&2; exit 2 ;;
esac
