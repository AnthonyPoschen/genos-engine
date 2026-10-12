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

## Cursor Cloud

Proof windows run on lavapipe under a headless Weston compositor. Desktop login starts `/usr/local/bin/genos-weston`. Wait until `pgrep -x weston` shows that process. The socket is `/tmp/genos-runtime/wayland-1`.

Source `/etc/profile.d/genos-cloud.sh` before `make run`, `make verify`, or a test that opens a window. Login shells and interactive bash load that file. It sets `XDG_RUNTIME_DIR` (`/tmp/genos-runtime`), `WAYLAND_DISPLAY` (`wayland-1`), `VK_ICD_FILENAMES`, `VK_DRIVER_FILES`, and `MESA_SHADER_CACHE_DIR`.

`make verify` runs the smoke script `examples/stress/scripts/smoke.rhai`. On lavapipe, `wait_settled` can hit its frame cap while the script checks still pass.

`cargo test --workspace` uses the same compositor. The pointer-lock test `the_first_click_locks_the_pointer_and_escape_releases_it` needs Hyprland. These GPU checks fail on lavapipe: `a_camera_spin_does_not_recolor_the_wall`, `the_outside_lamp_and_the_fire_stop_at_the_corridor_wall`, and `walking_does_not_flip_the_light_between_states` in `crates/render/tests/gpu_frame.rs`, and both tests in `examples/stress/tests/picture.rs`.

The toolchain is Rust 1.99.0. `Cargo.lock` uses edition 2024.
