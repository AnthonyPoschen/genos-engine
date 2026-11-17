#!/usr/bin/env bash
# Geometry coverage + buffer poison + Vulkan validation for GPU integrity.
# Gates the PR against Instance-stride holes, uninitialised buffer reads, and
# validation-layer API misuse. Needs Wayland + lavapipe in the env.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

export GENOS_BUFFER_POISON=1
export VK_INSTANCE_LAYERS="${VK_INSTANCE_LAYERS:+$VK_INSTANCE_LAYERS:}VK_LAYER_KHRONOS_validation"

echo "==> instance stride (source gate)"
cargo test --locked -p genos-render --test instance_stride -- --test-threads=1

echo "==> geometry coverage under buffer poison"
cargo test --locked -p genos-stress --test geometry_coverage -- --test-threads=1 --nocapture

echo "==> Vulkan validation gate (fails on any non-allowlisted VUID)"
python3 scripts/vk-validation-gate.py

echo "ci-gpu-integrity: ok"
