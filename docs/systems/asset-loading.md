# Asset loading (design)

Status: design. Decision: [ADR 0020](../adr/0020-asset-pipeline-import-cook-stream.md). Detailed step list and support matrix: [asset plan](assets/plan.md). This file is the system-level contract that ties that plan to the [job system](job-system.md), the [scene ECS](ecs-scene.md), [geometry LOD](geometry-lod.md), and [ADR 0019](../adr/0019-agent-operable.md). Where this file and the plan disagree on a name, the plan wins and this file changes in the same commit.

## 1. Goal

A game loads glTF 2.0 models (`.gltf`, `.glb`) with PBR metallic-roughness materials, normal maps, occlusion, emissive (with `KHR_materials_emissive_strength`), and places them in the scene without a frame hitch. The same model loads byte-identically on a client, on a headless server (collision subset only), and in a test. Every step is driven and checked through text: data files, the CLI, MCP, and metrics. There is no editor GUI.

## 2. Where it sits today (master)

| Piece | State | Where |
|---|---|---|
| glTF 2.0 parse: meshes, `TEXCOORD_0..7`, tangents, node matrices, images | done | `crates/load/src/gltf.rs`, `examples/gltf_info.rs` |
| PNG / JPEG decode, mips | done | `png.rs`, `jpeg.rs`, `mips.rs` |
| `PbrMaterial` and one shade (ADR 0014) | done (#71) | `crates/load/src/shade.rs` |
| Texture page streaming under `VK_EXT_memory_budget` | done (#77) | `crates/load/src/stream.rs`, `pages.rs`, `vram.rs`; tests `crates/render/tests/material_stream.rs` |
| SDF + six cards `.gbake` | done | `crates/bake` |
| Scene-file read off the frame thread | done | `run_pumped`, thread `genos-scene-load` |
| Import document, cook to engine format, package, mesh streaming | missing | this design |
| Placing a glTF node tree as scene objects | missing | this design, §6 |

## 3. Pipeline: import → cook → runtime load

Three stages, each a pure function of its inputs plus a settings header, so each output has one content hash.

1. **Import** (`genos-asset import file.glb`). Runs the existing decoders. Writes the import document `.gimport`: the source bytes' SHA-256, the runtime projection (meshes, `PbrMaterial`, node tree, images still encoded), and a support note per dropped feature (e.g. `KHR_materials_transmission dropped`, `skin kept-not-used`). Required extensions the engine cannot honour fail the import with a `LoadError` that names them. Never on the frame thread.
2. **Cook** (`genos-asset cook`). Offline, deterministic, multithreaded on the job system's Slack pool or as a standalone CLI. Reads `.gimport`, writes `.gcook`:
   - vertex streams quantised (position f32 or snorm16 in bounds, normal oct16, tangent oct16 + sign, UV half or unorm16), indices `u16` when ≤ 65 535 vertices;
   - mesh LOD chain (geometry-LOD hook, §8) and later meshlets;
   - textures as KTX2 BC7 (desktop) / ASTC 4×4, normal maps RGBA8 (ADR 0014 §4), full mip chain, pages aligned to the #77 streamer;
   - the `.gbake` SDF and cards via `crates/bake`;
   - collision subset (hull or triangle mesh) for the server profile;
   - a header with cook version and every cook setting; the cook hash is SHA-256 over source hash + header.
3. **Runtime load** (`AssetServer` in `genos-load`). Opens a package (`.gpak`: manifest + aligned blobs, memory-mapped) or loose `.gcook` files. Resolves an `AssetId` (cook hash) to a handle at once; bytes arrive later. Uploads go through the transfer queue path #77 already uses. No transcode, no simplify, no tangent generation at runtime.

Dev loop: a `.glb` passed straight to the runtime is imported and cooked on demand into `~/.cache/genos/cook/` (or `--cook-cache`), keyed by hash, then loaded the same way. Shipping uses `Prebuilt` packages only. Hot reload: a changed source hash recooks and swaps the handle's generation; it emits ADR 0013 change records (material edit ≠ geometry rebuild).

## 4. Async streaming

States per asset: `Requested → Reading → Decoding → Uploading → Resident`, plus `Failed(reason)` and `Evicted`. A handle is drawable from the moment it exists:

- Missing geometry draws nothing in the main view and keeps its bounds for culling; an optional placeholder box draws in `view residency` only. Coarsest LOD loads first, finer LODs on demand.
- Missing texture mips read the next coarser resident mip, or the factor colour (ADR 0014 §4). Never black, never a half-written page.
- Priority: screen importance (projected size), then distance; scripted `asset.preload` requests outrank both.
- Budget: the single device-local budget from `vram.rs` (`heapBudget` minus other allocations minus `max(5%, 256 MB)` headroom). Geometry and textures share it; geometry takes at most `geometry_share` of it.
- Per-frame caps: `upload_mb_per_frame` and `decode_ms_per_frame`, so a load never stalls the frame. A page or LOD flips visible only after its transfer submit's fence.

## 5. Job-system hook

Maps onto the four classes in [job-system §9](job-system.md):

| Work | Class | Notes |
|---|---|---|
| File read (mmap / `pread`) | Pumped thread `genos-asset-io` | Like `genos-scene-load`; one thread, bounded queue, no blocking in a frame scope |
| Decode, decompress, CPU-side validation | Slack | One task per blob; result goes to a lock-free completion queue |
| Choose what to request/evict this frame | Must (small) | Pure function of camera + resident set + budget; index-ordered, so replays match |
| Record the upload copies | Soon | Bounded by `upload_mb_per_frame`; leftover moves to next frame, `jobs.deferred` counts it |
| Import / cook (CLI or dev on-demand) | Slack, or workers of the CLI process | Reduction in input index order so the cook output is identical for any worker count |

Determinism rule: what is resident never changes gameplay. The server loads only the collision subset and loads it synchronously at session start, so job timing cannot change a server hash.

## 6. ECS scene binding

From [ecs-scene §5–7](ecs-scene.md):

- A model is a prefab. `asset.spawn("models/helmet.glb", pos)` (Rhai and MCP) creates one scene object per glTF node with a mesh, linked by `Parent` handles in node order, root first. The root gets `PrefabSource { asset: AssetId, path }`; children get it plus their glTF node index for overrides.
- Components: `MeshRef(AssetId, mesh index)`, `MaterialRef(AssetId, material index)` (overridable per instance), local TRS from the node (non-uniform scale lands here, which ecs-scene §7 left room for), `Bounds` from the cook header so culling works before bytes arrive.
- `.gscene` stores the `AssetId` and the path, never the bytes; a reload with the same cook hash gives the same handles. A changed hash keeps handles and bumps the asset generation.
- Residency is not a component the server sees. The client keeps it in a side table keyed by handle.
- Skins and animation channels are kept by import, bound later by the [animation design](animation.md).

## 7. Formats and settings

`settings.assets` (`EngineSettings` text form, MCP `get_asset_settings` / `set_asset_settings`, CLI `--assets-*`, env `GENOS_ASSETS_*` only when the flag is absent):

| Key | Low (GTX 1060, 4 GB floor) | High | Meaning |
|---|---|---|---|
| `texture_quality` | Low (−3 mips) | Ultra | Existing ADR 0014 cap |
| `geometry_share` | 0.35 | 0.5 | Max share of the VRAM budget for vertex/index/cluster data |
| `upload_mb_per_frame` | 8 | 32 | Transfer cap |
| `decode_ms_per_frame` | 2.0 | 4.0 | Slack decode wall cap |
| `lod_bias` | +1 | 0 | Passed to geometry LOD |
| `cook_on_demand` | on in dev, off in ship | | |

**VRAM floor.** The engine must run with 4 GB device-local memory. With the Low preset at 1080p, the target is: render targets + GI + other fixed allocations ≤ 1.5 GB, leaving ≥ 2 GB budget for assets after headroom. The Sponza + 20 DamagedHelmet test scene must be fully resident at Low under a forced 4 GB budget (`--vram-budget-mb 4096`) with `materials refused=0`, and must degrade (coarser mips/LODs, no failures) under a forced 2 GB budget.

## 8. Geometry LOD bake hook

The cook is the only place [geometry-lod Stage 1](geometry-lod.md) simplification runs: 4 levels, `meshopt_simplify` targets 50/25/12.5 %, the object-space error stored per level in `.gcook`. Stage 2 adds meshlets and the cluster DAG to the same file, in a new section, without changing Stage 1 bytes. meshoptimizer is a cook-only dependency (never linked into the renderer, ADR 0007 as read by the asset plan). The runtime streams LOD levels coarsest-first as separate blobs; the coarsest level of every placed mesh is pinned resident.

## 9. Agent control and verification

Per ADR 0019.

- **CLI.** `genos-asset import|cook|pack|info|verify <file>`. `info` prints a stable text summary (meshes, prims, tris per LOD, materials, textures with format and mips, support notes, hashes). `verify` recooks and compares hashes.
- **MCP.** `asset.load {path}`, `asset.spawn {path, transform}`, `asset.unload {id}`, `asset.preload {ids}`, `asset.list` (state, bytes resident, LOD, mip per asset), `asset.info {id}`, `get/set_asset_settings`, `view residency|lod|mip|material_channel`.
- **Rhai.** `asset_spawn`, `asset_preload`, `await_resident(id, timeout_s)` for scripted tests.
- **Per-frame metrics** (one compact line, like `materials …`): `assets requested=N resident=N failed=N load_ms_p50=… load_ms_p99=… decode_ms=… upload_mb=… vram_tex_mb=… vram_geo_mb=… budget_mb=… evictions=N holes=N`. `load_ms` is request-to-resident wall time. Frame time impact is read from the existing `jobs.*` and frame stats.
- **Deterministic test scenes** in `debug/scenes/assets/`: `helmet.rhai` (one DamagedHelmet, fixed camera), `samples_grid.rhai` (§10 set on a grid), `sponza_helmets.rhai` (Sponza + 20 helmets, scripted camera path), `pressure.rhai` (the same with a forced budget). `debug/assets` binary on the shared [debugging](debugging.md) scaffold, `--check` headless, output `./genos-debug/assets/`.

## 10. Harness-first plan

Built before the feature, each step lands with its gate red first.

**Test assets** (Khronos glTF-Sample-Assets, fetched by a script like `scripts/fetch-sponza.py`, pinned by commit and SHA-256, never committed as binaries): Box, BoxTextured, BoxTexturedNonPowerOfTwo, DamagedHelmet, MetalRoughSpheres, NormalTangentTest, NormalTangentMirrorTest, TextureCoordinateTest, TextureSettingsTest, EmissiveStrengthTest, AlphaBlendModeTest, OrientationTest, CesiumMilkTruck (hierarchy), Sponza (scale), plus malformed fixtures (truncated `.glb`, bad accessor bounds, LFS pointer, unknown required extension).

**Gates.**

| Gate | Check | Planted bug it must catch |
|---|---|---|
| A1 import | `info` text golden for every sample; malformed fixtures return the named `LoadError` | drop tangents silently; accept unknown required extension |
| A2 cook determinism | cook twice, 1 and N workers → identical SHA-256 | unordered reduction |
| A3 round trip | decoded `.gcook` vertices within quantisation error of the source; index sets equal | oct-encode sign flip on tangent `w` |
| A4 LOD | tris per level within 10 % of 50/25/12.5 %; stored error monotonic | error stored in screen space |
| A5 picture | lavapipe screenshot vs reference with the CPU path tracer (`genos-gpuref`) on DamagedHelmet, MetalRoughSpheres, NormalTangentTest; channel views via `view material_channel` | normal map Y flipped; emissive not sRGB-decoded |
| A6 async | `sponza_helmets` scripted path: no frame > 1.5× median during loads; no hole frame; `failed=0` | upload on frame thread; flip before fence |
| A7 budget | forced 4 GB → all resident at Low; forced 2 GB → degrades with `refused` only on the lowest-importance items | geometry ignores `geometry_share` |
| A8 ECS | spawn → save `.gscene` → reload → same handles, parents, transforms; CesiumMilkTruck wheel parents correct | renumber on reload |
| A9 server | headless profile loads only the collision subset; no Vulkan; hash matches client's subset hash | server pulling textures |
| A10 timing (4070, morning window) | Low preset: `load_ms_p99`, frame-time spike, VRAM; numbers recorded in the PR | — |

**Steps** (each one PR, harness first): S1 `info` + fixtures + A1 → S2 `.gimport` → S3 cook vertices/indices + A2/A3 → S4 KTX2 BC7/ASTC textures through the #77 streamer → S5 LOD chain + A4 (unblocks geometry-lod Stage 1) → S6 `AssetServer` async states + A6/A7 → S7 ECS spawn + A8 → S8 package `.gpak` + server profile A9 → S9 hot reload → S10 4070 timings A10.

## 11. Failure modes and probes

| Failure | Probe |
|---|---|
| Hitch during load | A6, `assets decode_ms`, `jobs.over_budget` |
| Black or wrong material | A5, `view material_channel` |
| Asset never arrives | `asset.list` state + `failed=` reason |
| VRAM over budget / OOM | `materials refused`, `assets evictions`, the #77 OOM retry |
| Nondeterministic cook | A2, `genos-asset verify` |

## 12. Deviations flagged

- glTF `BLEND` alpha: kept as cutoff 0.5 until the transparent pass (ADR 0014).
- Missing tangents: generated at cook with MikkTSpace, not at runtime (ADR 0014).
- `KHR_texture_basisu`, `KHR_draco_mesh_compression`, `EXT_meshopt_compression`: not loaded in the first cut; import reports them.
- Placeholder geometry is only drawn in `view residency`; the main view shows nothing until the coarsest LOD is resident (it is pinned, so this is the first load only).

## 13. Decisions for Anthony

1. Package format `.gpak` (mmap manifest + blobs) instead of zip — OK?
2. Low-preset `geometry_share` 0.35 of the asset budget — OK?
3. Dev on-demand cook from raw `.glb` — keep on in dev builds?

## References

- Khronos glTF 2.0 specification. <https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html>
- Khronos glTF-Sample-Assets. <https://github.com/KhronosGroup/glTF-Sample-Assets>
- KTX 2.0 specification. <https://registry.khronos.org/KTX/specs/2.0/ktxspec.v2.html>
- meshoptimizer. <https://github.com/zeux/meshoptimizer>
- [Asset plan](assets/plan.md), [ADR 0014](../adr/0014-pbr-materials.md), [ADR 0019](../adr/0019-agent-operable.md)
