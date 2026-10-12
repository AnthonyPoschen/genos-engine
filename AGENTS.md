# AGENTS.md

Genos Engine is the runtime and the Linux editor. The hosting service is `../genos`. Keep those products apart.

## Rubric

Every coding agent and every planning agent follows `docs/agent-rubric.md`. A reviewer scores the change on that checklist. The pull request includes the self-scored table (`pass`, `fail`, or `n/a`, with evidence).

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

Before you add lights, shadows, or global illumination, read `docs/systems/lighting/README.md` and `docs/adr/0012-gi-v2.md`. GI v1 (radiance cascades, ADR 0008) was removed; do not bring it back.

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
