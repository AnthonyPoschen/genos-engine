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

## Input behavior

`../input-zig` is the behavior source for the input port. Preserve the tested platform calls. The file list is in `docs/architecture.md`.

## Proof

Before you implement the first scene, read `docs/goals/mvp-scene.md`.
