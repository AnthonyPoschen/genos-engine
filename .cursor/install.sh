#!/usr/bin/env bash
# Cloud agent bootstrap: build dependencies plus Mesa's lavapipe and a headless
# Weston, so the engine and its lighting tests run without a GPU
# (tools/lighting/headless.sh). Idempotent.
set -euo pipefail

packages=(
    build-essential pkg-config git
    glslc                                         # shaders (crates/render, crates/gpuref)
    libwayland-dev wayland-protocols libxkbcommon-dev  # crates/window
    libvulkan1 mesa-vulkan-drivers vulkan-tools   # lavapipe (software Vulkan)
    weston                                        # headless Wayland compositor
    python3-numpy python3-pil                     # tools/lighting/dump.py
)
missing=()
for p in "${packages[@]}"; do
    dpkg -s "$p" >/dev/null 2>&1 || missing+=("$p")
done
if [ ${#missing[@]} -gt 0 ]; then
    sudo apt-get update -qq
    sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends "${missing[@]}"
fi

# Cargo.lock needs a toolchain newer than some images ship (edition 2024 crates).
if ! command -v rustup >/dev/null; then
    curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal
    sudo ln -sf "$HOME/.cargo/bin/"{cargo,rustc,rustup} /usr/local/bin/
fi
export PATH="$HOME/.cargo/bin:$PATH"
rustup toolchain install stable --profile minimal
rustup default stable

cargo build --workspace --release --tests
