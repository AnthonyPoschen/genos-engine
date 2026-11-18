#!/usr/bin/env bash
# Geometry coverage + buffer poison + Vulkan validation for GPU integrity.
# Gates the PR against Instance-stride holes, uninitialised buffer reads, and
# validation-layer API misuse. Needs Wayland + lavapipe in the env.
#
# Bound for shared house runners (16 GiB cgroup): uncapped lavapipe + debug
# builds OOMKilled the pod (exit 137). Caps below keep peak RSS well under
# ~8 GiB. Test assertions are unchanged.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

# Mesa lavapipe defaults LP_NUM_THREADS to nproc; each thread holds compile
# and dispatch state. Cargo defaults -j to nproc. Cap both.
export LP_NUM_THREADS="${LP_NUM_THREADS:-2}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
export RUST_TEST_THREADS="${RUST_TEST_THREADS:-1}"

# Release profile: same checks, far less RSS than debug+lavapipe+validation.
# vk-validation-gate.py reads this and adds --release to its cargo commands.
export GENOS_GPU_INTEGRITY_RELEASE=1

export GENOS_BUFFER_POISON=1
export VK_INSTANCE_LAYERS="${VK_INSTANCE_LAYERS:+$VK_INSTANCE_LAYERS:}VK_LAYER_KHRONOS_validation"

echo "==> bounds: LP_NUM_THREADS=$LP_NUM_THREADS CARGO_BUILD_JOBS=$CARGO_BUILD_JOBS RUST_TEST_THREADS=$RUST_TEST_THREADS release=1"

echo "==> instance stride (source gate)"
cargo test --locked --release -p genos-render --test instance_stride -- --test-threads=1

echo "==> geometry coverage under buffer poison"
cargo test --locked --release -p genos-stress --test geometry_coverage -- --test-threads=1 --nocapture

echo "==> Vulkan validation gate (fails on any non-allowlisted VUID)"
python3 scripts/vk-validation-gate.py

echo "ci-gpu-integrity: ok"
