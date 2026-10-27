# Geometry LOD and GPU-driven culling (design)

Status: proposed, 27 Oct 2026. Owner: Engine. Binds to [ADR 0012](../adr/0012-gi.md), [GI design](gi-design.md), [asset plan](assets/plan.md) steps 8+, [rendering extras plan](rendering/extras-plan.md) "LOD and culling", and texture streaming (#77, `crates/load/src/stream.rs`, `crates/render/src/material_stream.rs`).

## 1. Goal
Draw scenes with far more triangles than the GPU can raster by picking, per object or per cluster, the coarsest geometry whose screen-space error is under ~1 pixel, with no cracks and no visible pops, culled on the GPU, and streamed like texture pages.

## 2. Where it sits today
| Piece | State | Code |
|---|---|---|
| glTF mesh load, BVH | done | `crates/load/src/gltf.rs`, `crates/mesh/src/bvh.rs` |
| Draw | `vkCmdDraw` per draw, CPU frustum cull `in_view` | `crates/render/src/world.rs`, `gltf_draw.rs` |
| LOD | missing (GI design names up to 4 LODs) | – |
| Meshlet cook | planned, not built (asset plan step 8, meshoptimizer, cook-only) | `crates/cook` (not yet created) |
| Page streaming | textures done (#77) | `stream.rs`, `pages.rs`, `vram.rs` |

## 3. Prior art
- **Discrete LOD chains**: Hoppe, "Progressive Meshes", SIGGRAPH 1996; Garland & Heckbert, "Surface Simplification Using Quadric Error Metrics", SIGGRAPH 1997. Production: Unity/Unreal LOD groups, `meshopt_simplify` (Kapoulkine, meshoptimizer).
- **Cluster DAG LOD**: Karis, "A Deep Dive into Nanite Virtualized Geometry", SIGGRAPH 2021 Advances; Cignoni et al., "Batched Multi Triangulation", IEEE Vis 2005 (the locked-boundary group simplification Nanite builds on). Open implementations: meshoptimizer `clusterlod` demo (2024–25), Bevy 0.14+ "virtual geometry", zeux/niagara.
- **GPU-driven culling**: Haar & Aaltonen, "GPU-Driven Rendering Pipelines", SIGGRAPH 2015; Wihlidal, "Optimizing the Graphics Pipeline with Compute", GDC 2016; Greene, "Hierarchical Z-Buffer Visibility", SIGGRAPH 1993 (two-pass HiZ occlusion).
- **Transitions**: alpha-to-coverage/dither cross-fade (Unreal "dithered LOD transition"); geomorphing (Hoppe 1996). Cluster DAGs are crack-free by construction (locked shared boundaries, monotonic parent error).
- **Mesh shaders**: `VK_EXT_mesh_shader` (Vulkan 2022). Not on GTX 1060 (Pascal) or RX 580.

## 4. Options
| | A. Baked discrete chain | B. Cluster DAG, compute cull + `vkCmdDrawIndexedIndirect` | C. B + mesh-shader path | D. B + software raster (Nanite) |
|---|---|---|---|---|
| Pop risk | per-object switch; needs dither fade | per-cluster, ~sub-pixel error, no fade | same as B | same |
| Cracks | none (whole mesh switches) | none by construction | none | none |
| Huge single meshes (terrain, cliffs) | poor: whole mesh one level | good | good | best |
| GTX 1060 floor | yes | yes | no (fallback = B) | yes but costly |
| Cost to build | S | L | L + M | XL |
| GI/BLAS fit | one BLAS per level (GI design) | BLAS per a fixed coarse level; raster uses DAG | same | same |
| ADR fit | yes | yes (asset plan already plans this) | **deviation** from ADR 0012 non-goal | **deviation** |

## 5. Recommendation: staged
1. **Stage 1 – discrete chain + GPU cull.** Cook 4 LODs with `meshopt_simplify` (target 50/25/12.5%, error in object-space metres, store it). Select by projected error `e * H / (2 d tan(fov/2)) < τ` (τ = 1 px High, 2 px Low). Dithered cross-fade over 0.25 s on a switch. Compute instance cull (frustum + two-pass HiZ) fills `vkCmdDrawIndexedIndirectCount` (core in Vulkan 1.2). Ships value soon and is the BLAS source for GI.
2. **Stage 2 – cluster DAG.** Same cook adds 64v/64t meshlets (`meshopt_buildMeshlets`), group-of-~8 locked-boundary simplify, parent error spheres (Karis 2021). Compute cluster cull and LOD cut, indexed indirect on the hardware rasterizer. Cluster pages stream through the #77 page/VRAM budget; hierarchy stays resident.
3. **Stage 3 – optional `VK_EXT_mesh_shader` path** only if Stage 2 timings on the 4070 show the index-expansion compute is >15% of geometry ms. Needs an ADR amendment. Compute stays the default and the Low path.
Software raster (D) stays out.

**Deviation flags.** Stage 3 contradicts ADR 0012 non-goals and needs Anthony's ruling. Dither fade is not in Karis 2021 (Nanite does not fade); it is only for Stage 1.

## 6. Data and settings
`settings.geometry`: `lod_error_px` (Low 2.0 / Med 1.25 / High 1.0), `lod_bias`, `fade_seconds`, `hiz` (on), `cluster_lod` (off until Stage 2), `max_resident_cluster_mb` (Low 128, Med 256, High 512), `mesh_shader` (auto/off). Runtime-settable, no restart. Cook settings in `.gcook` header, so cook bytes are deterministic (asset plan hash rule).

## 7. Per-frame stats
`geom.cull_ms`, `geom.hiz_ms`, `geom.draw_ms`, `geom.tris_drawn`, `geom.instances_{tested,visible}`, `geom.clusters_{tested,visible}`, `geom.lod_hist[0..4]`, `geom.resident_mb`, `geom.page_misses`, `geom.fading`. VRAM through the shared budget helper (`budget.rs`).

## 8. Low preset, GTX 1060 6GB, 1080p
Budget: geometry ≤ 3.0 ms, cull+HiZ ≤ 0.4 ms, resident clusters ≤ 128 MB, `lod_error_px` 2. Vulkan 1.3 core only (draw-indirect-count, descriptor indexing, sync2). No mesh shaders.

## 9. Verification harness and mutation tests
Harness `crates/render/tests/geometry_lod.rs` + `scripts/assets-lod.rhai` (asset plan row). Oracles are independent of the selector:
- **Error oracle**: CPU reprojects each drawn level against the LOD0 mesh, max screen deviation ≤ τ + 0.5 px.
- **Pop test**: fly-through capture, per-frame luminance delta vs LOD0-forced render; max ≤ 16/255 (same limit as the lamp-pop test).
- **Crack test**: render against a contrasting background, count background pixels inside the LOD0 silhouette = 0.
- **Cull completeness**: GPU visible set ⊇ CPU brute-force set (0 false negatives over a camera cut).
- **Determinism**: two cooks → identical bytes/hash.
Planted mutations (each must fail, thresholds ≥ 2/255): drop the distance term (always LOD0 → tri count fails), flip error comparison, unlock group boundaries (crack test fails), skip HiZ second pass (cut test fails), zero fade time (pop test fails), off-by-one LOD index, unseeded simplify.

## 10. Agent control and verification
- CLI: `--lod-error-px`, `--lod-force N`, `--hiz off`, `--cluster-lod on`.
- MCP: `geometry.set {..}`, `geometry.stats`, `view lod|meshlet|residency|hiz` (asset plan view modes), `geometry.pick` returns instance id, LOD level, cluster id under the cursor.
- An agent verifies by running the harness, reading `geom.*` stats, and grading the `lod` view screenshot; no GUI.

## 11. Networking
Render-only. Server loads the Sim subset (collision mesh), never LODs or clusters. LOD choice is never replicated.

## 12. Decisions for Anthony
1. Accept the staged path (discrete chain first, cluster DAG second)?
2. Allow a later `VK_EXT_mesh_shader` path (ADR 0012 amendment) or keep it out permanently?
3. Pop limit 16/255 and τ = 1 px High / 2 px Low OK?
