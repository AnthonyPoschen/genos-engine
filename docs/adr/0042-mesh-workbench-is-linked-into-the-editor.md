# 0042: mesh-workbench is linked into the editor

Status: **Accepted** (Anthony, 22 Nov 2026). Touches [ADR 0007](0007-native-calls-go-to-the-system-and-vulkan.md), [ADR 0019](0019-agent-operable.md) and [ADR 0020](0020-asset-pipeline-import-cook-stream.md). Record: [mesh workbench](../systems/mesh-workbench.md). Running list: [editor checklist](../systems/editor-checklist.md).

## Context

Meshes are made and edited with [mesh-workbench](https://github.com/NathanChambers/mesh-workbench) (crate `gpt-mesh`, MIT), a Rust library with a JSON command API. The first integration started its `mesh-server` as a child process and posted commands over loopback HTTP. Anthony decided the tool is an SDK dependency, not a second process.

ADR 0007 says a third-party crate does not own the window, the input, the renderer, the audio, or the network, and that other helpers live in this repository. `gpt-mesh` owns none of those five. It is a helper that does not live in this repository, and it brings `serde`, `serde_json`, `png` and `ufbx`.

## Decision

1. `gpt-mesh` is a git dependency pinned to a revision. One crate, `genos-meshbench`, depends on it. No other engine crate names `gpt-mesh`, `serde`, or `serde_json`.
2. `genos-meshbench` speaks `genos_json::Value`. It converts to and from `serde_json` through JSON text.
3. Only the editor binary (`editor/`, `genos-editor`) links `genos-meshbench`. It hands the scene program a function that opens a session (`Program::mesh_session`), so the shared frame loop (`genos-scene-program`) and `genos-lighting` do not link mesh-workbench or `serde_json`. The runtime that ships a game does not. A finished mesh still enters a game as a GLB through import, cook and load (ADR 0020).
4. `deny.toml` allows that one git source. The five bans of ADR 0007 are unchanged.
5. The pinned revision moves on purpose, in its own commit, after the bridge tests pass against it. mesh-workbench says its command API and workspace format may change.
6. If the crate cannot be linked as it stands, it is forked under Anthony's GitHub and this ADR names the fork.

## Consequences

- An edit is a function call on the frame thread. A long command stalls the frame until it returns. Moving the session to a worker is later work.
- The session writes `project.json` and its recordings in the workspace, as the server did. The viewer still follows that file, so a workspace another process edits is still shown.
- The engine's build now fetches a git repository and compiles `ufbx` (C, through its crate).

## Rejected

- **A child process and loopback HTTP.** Built first. It kept the dependency out of the tree, and it needed the tool installed and a port.
- **Vendoring a copy of the source.** It would drift from upstream with no record of the revision.
