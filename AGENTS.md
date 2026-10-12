# AGENTS.md

Genos Engine is the runtime and the Linux editor. The hosting service is `../genos`. Keep those products apart.

## Domain

This repository has one context. Before you add a term or a decision, read `docs/agents/domain.md`.

## Architecture

Before you add a crate, a platform path, a window, or a renderer, read `docs/architecture.md`.

Before you change the language, the repository split, the input model, the window, or the graphics API, read `docs/adr/`.

## Native calls

Before you add a dependency, read `docs/adr/0007-native-calls-go-to-the-system-and-vulkan.md`. This covers the window, the input, the graphics, the audio, and the network.

## Scripting

Before you add a script, a mod API, or the `rhai` crate, read `docs/adr/0009-rhai-is-the-scripting-language.md`.

## Lighting

Before you add lights, shadows, or global illumination, read `docs/lighting.md` and `docs/adr/0008-lighting-uses-radiance-cascades.md`.

## Breaking Point

Before you write vector math, quaternion math, a camera, or a view-projection, read `docs/references/breaking-point.md`.

## Rust

Finish a Rust change only after `cargo build --workspace` and the tests for the crates you touched pass. The workspace denies compiler warnings. A new crate sets `[lints] workspace = true`. `InputCode` keeps the input-zig names, so that enum is the one allowed `non_camel_case_types` exception.

## Input behavior

`../input-zig` is the behavior source for the input port. Preserve the tested platform calls. The file list is in `docs/architecture.md`.

## Systems

Before you change a system, rewrite a system, or add a game feature that uses one, read `docs/systems/README.md` and the matching system file. When the goal, the intent, the game use, or a limit changes, edit that system file in the same change.

## Proof

Before you implement the first scene, read `docs/goals/mvp-scene.md`.

## Cursor Cloud specific instructions

The image Rust is too old for this lockfile. `thin-vec` 0.2.20 needs edition 2024, so the toolchain is Rust 1.99.0 (`rustup default 1.99.0`).

The build links system libraries. Install `libwayland-dev`, `wayland-protocols`, `libwayland-bin` (`wayland-scanner`), `libxkbcommon-dev`, `libasound2-dev`, `glslc`, and `mesa-vulkan-drivers`. `crates/render/build.rs` runs `glslc`. There is no GPU; Vulkan is lavapipe. Set `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json` before a window or a picture test.

`cargo build --workspace --locked` is the build. Picture tests need a Wayland socket. Headless Weston is the one this environment starts:

```sh
export XDG_RUNTIME_DIR=/tmp/runtime-ubuntu
export WAYLAND_DISPLAY=wayland-0
export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json
```

The socket name stays `wayland-0`. MCP discovery writes `$XDG_RUNTIME_DIR/genos/mcp`. A socket named `genos` in that directory makes the camera fail with `Not a directory`.

`cargo test --workspace --locked` is the suite. Skip `the_first_click_locks_the_pointer_and_escape_releases_it` here: it drives Hyprland with `hyprctl` and `ydotool`, and it runs whenever `WAYLAND_DISPLAY` is set and `target/debug/genos-camera` exists. Lavapipe disagrees with these picture checks, so skip them on this machine: `a_camera_spin_does_not_recolor_the_wall`, `the_outside_lamp_and_the_fire_stop_at_the_corridor_wall`, `walking_does_not_flip_the_light_between_states`, `the_live_picture_tracks_the_ray_trace_within_100ms` (a frame is slower than 100 ms), and `the_small_stress_picture_matches_the_ray_trace` (the room A bounce).

A visible window on the X11 desktop uses Weston `--backend=x11` on the same socket. `genos-camera --proof --profile off --frames N --size WxH --readback FILE` draws that many frames and writes a PNG.
