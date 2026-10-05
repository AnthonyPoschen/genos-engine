# Genos Engine

Genos Engine is a Rust game runtime and a Linux editor. The runtime runs on Linux, Windows, and macOS. The editor runs on Linux. Omarchy is the first desktop the editor must fit. Engine code makes each native call to the operating system or to Vulkan.

The hosting product stays in the `genos` repository.

## Plan

- `CONTEXT.md` holds the vocabulary.
- `docs/architecture.md` holds the crate map and the first proof.
- `docs/goals/mvp-scene.md` is the execution goal for that proof.
- `docs/adr/` holds the decisions.
- `AGENTS.md` tells an agent which of those files to open.
- `docs/references/breaking-point.md` names the old engine to read for math and camera ideas.

The input behavior comes from [`input-zig`](https://github.com/AnthonyPoschen/input-zig). This repository will port that library to Rust.
