# 0020: Assets go import → offline cook → async runtime load

Status: **Accepted** by the Engine bot under Anthony's delegated design authority. Design: [`docs/systems/asset-loading.md`](../systems/asset-loading.md).

## Context

glTF parsing, `PbrMaterial` (ADR 0014) and texture page streaming (#77) exist, but the runtime still decodes source files, nothing is cooked to a GPU format, meshes are not streamed, and a model cannot be placed as scene objects. Geometry LOD, animation and the server all need one hashed, deterministic asset form.

## Decision

1. Three stages: **import** (source → `.gimport`, keeps source bytes, notes every dropped feature), **cook** (offline, deterministic → `.gcook`: quantised vertices, LOD chain, KTX2 BC7/ASTC textures, `.gbake`, collision subset), **runtime load** (`AssetServer`, async, no transcode or simplify at runtime). Identity is the SHA-256 cook hash.
2. Loads are asynchronous and never stall a frame: IO on a pumped thread, decode as Slack jobs, upload recorded under a per-frame cap and flipped after its fence. A handle is drawable immediately with fallbacks (coarser mip/LOD, factor colour).
3. Textures and geometry share the one `VK_EXT_memory_budget` budget from ADR 0014. The engine must run at Low on a 4 GB GTX 1060.
4. A model spawns as a prefab: one ECS object per glTF mesh node, linked by `Parent`, with `PrefabSource`, `MeshRef`, `MaterialRef`.
5. Everything is operable by text (ADR 0019): `genos-asset` CLI, `asset.*` MCP, Rhai, one `assets …` metrics line, deterministic test scenes. No asset browser GUI.
6. Harness first: Khronos sample models and malformed fixtures, gates A1–A10 in the design.

## Rejected

- Loading raw glTF at runtime in shipping builds (runtime transcode, tangent gen and simplify cost frame time and break determinism).
- Separate texture and geometry VRAM pools (fixed pools fail the 4 GB floor).
- An editor asset browser (ADR 0019).

## Departures

- meshoptimizer is a cook-only dependency; the renderer stays free of it (ADR 0007).
- glTF `BLEND` stays alpha-cutoff until the transparent pass (ADR 0014).
