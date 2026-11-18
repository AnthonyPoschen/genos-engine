#!/usr/bin/env bash
# Geometry coverage + buffer poison + Vulkan validation for GPU integrity.
# Gates the PR against Instance-stride holes, uninitialised buffer reads, and
# validation-layer API misuse. Needs Wayland + lavapipe in the env.
#
# Bound for shared house runners (16 GiB cgroup): uncapped lavapipe + High
# default + debug builds OOMKilled the pod (exit 137). Caps below keep peak
# RSS well under ~8 GiB. Test assertions are unchanged: the validation gate
# grades VUIDs only (not picture pass/fail); stride and coverage still run.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

# Mesa lavapipe defaults LP_NUM_THREADS to nproc; each thread holds compile
# and dispatch state. Cargo defaults -j to nproc. Cap both hard.
export LP_NUM_THREADS="${LP_NUM_THREADS:-1}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-1}"
export RUST_TEST_THREADS="${RUST_TEST_THREADS:-1}"
export RAYON_NUM_THREADS="${RAYON_NUM_THREADS:-1}"

# Launch lock: LightingSettings::default() is High (million-slot cache).
# gpu_frame scenes also default High; GENOS_LIGHTING_PRESET wins every frame
# (crates/scene lighting apply_env). Potato skips that table; lavapipe keeps
# freed High pages until vkDestroyDevice. Validation still grades API misuse.
export GENOS_LIGHTING_PRESET="${GENOS_LIGHTING_PRESET:-potato}"

# Release profile: same checks, less text RSS than debug+lavapipe+validation.
# vk-validation-gate.py reads this and adds --release to its cargo commands.
export GENOS_GPU_INTEGRITY_RELEASE=1

export GENOS_BUFFER_POISON=1
export VK_INSTANCE_LAYERS="${VK_INSTANCE_LAYERS:+$VK_INSTANCE_LAYERS:}VK_LAYER_KHRONOS_validation"

log_mem() {
  local tag=$1
  local cgroup=""
  if [[ -r /sys/fs/cgroup/memory.current ]]; then
    cgroup=" cgroup_bytes=$(cat /sys/fs/cgroup/memory.current)"
  elif [[ -r /sys/fs/cgroup/memory/memory.usage_in_bytes ]]; then
    cgroup=" cgroup_bytes=$(cat /sys/fs/cgroup/memory/memory.usage_in_bytes)"
  fi
  echo "==> mem[$tag]: rss_kb=$(awk '/VmRSS/{print $2}' /proc/self/status)${cgroup}"
}

echo "==> bounds: LP_NUM_THREADS=$LP_NUM_THREADS CARGO_BUILD_JOBS=$CARGO_BUILD_JOBS RUST_TEST_THREADS=$RUST_TEST_THREADS RAYON_NUM_THREADS=$RAYON_NUM_THREADS preset=$GENOS_LIGHTING_PRESET release=1"
log_mem start

echo "==> instance stride (source gate)"
cargo test --locked --release -p genos-render --test instance_stride -- --test-threads=1
log_mem after_stride

echo "==> geometry coverage under buffer poison"
cargo test --locked --release -p genos-stress --test geometry_coverage -- --test-threads=1 --nocapture
log_mem after_geometry

echo "==> Vulkan validation gate (fails on any non-allowlisted VUID)"
python3 scripts/vk-validation-gate.py
log_mem after_validation

echo "ci-gpu-integrity: ok"
