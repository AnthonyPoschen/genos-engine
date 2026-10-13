# AGENTS.md

Genos Engine is the runtime and the Linux editor. The hosting service is `../genos`. Keep those products apart.

## Rubric

Every coding agent and every planning agent follows `docs/agent-rubric.md`. A reviewer scores the change on that checklist. The pull request includes the self-scored table (`pass`, `fail`, or `n/a`, with evidence). The first scored item is a dedicated debug binary for every engine system, under `debug/<system>/`. The second is observability: every failure mode and every tunable has a probe. Lighting is the reference for both. The inventory is `docs/systems/lighting/debug-standard.md`.

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

## Assets

Art assets live under `assets/` and are tracked with Git LFS. Read `docs/engine/assets.md` before adding an asset, a texture, or a mesh. `assets/source/<name>/` holds the untouched originals and a credits file. `assets/built/<name>/` holds the engine-ready files when conversion is not cheap enough to repeat on every build.

## Tools

A new tool signals intent with command-line arguments. An environment variable is only a secondary mirror of an argument: the argument wins when both are set, and the variable applies only when the argument was omitted. Document every argument in `--help` and in the system record.

The lighting debug binary (`debug/lighting`) writes screenshots, dumps, and logs under `./genos-debug/<kind>/` in the current working directory. `--out <dir>` changes that directory. It does not write those files under `$HOME`.

## Input behavior

`../input-zig` is the behavior source for the input port. Preserve the tested platform calls. The file list is in `docs/architecture.md`.

## Settings

Every engine-feature setting follows `docs/engine/settings-principles.md`. It is a public serializable value any system can read and write, it is on the MCP command catalogue, and a person can edit it from the debug menu or UI. Lighting is the reference. A game may map its own player settings onto that value, or show the raw value.

## Arguments

A tool signals intent with command-line arguments. An environment variable is only a secondary mirror of an argument: the argument wins when both are set, and the variable applies only when the argument was omitted. Document every argument in `--help` and in the system record.

## Output

An output-writing command behaves as a Unix tool. It writes into the current working directory by default, under `./genos-debug/<kind>/`. It takes `--out <dir>`, and that argument overrides the default. It does not write under `$HOME` unless that path was passed in. An environment variable is a mirror of the same engine setting, not a second location. The directory is an `EngineSettings` value, on the MCP catalogue, and in the debug menu, as in `docs/engine/settings-principles.md`. Count, size, and age caps stay on that directory.

## Systems

Before you change a system, rewrite a system, or add a game feature that uses one, read `docs/systems/README.md` and the matching system file. When the goal, the intent, the game use, or a limit changes, edit that system file in the same change.

Every new system is designed with networking in mind. Before writing that system's plan, read `docs/systems/core-networking/plan.md` and the networking row in `docs/agent-rubric.md`. The plan states replicated state, authority, prediction or rollback, determinism, and headless server behavior. The implementing commit copies those five lines into the system record.

## Proof

Before you implement the first scene, read `docs/goals/mvp-scene.md`.
