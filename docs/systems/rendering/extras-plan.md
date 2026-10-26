# Rendering extras plan

Written 2026-10-12 against `gi-v2` at `e61b89d`. This is a plan. It does not change the renderer. A later commit edits [Renderer](../renderer.md) in the same change that changes a game use or a limit.

**Goal.** Materials, shadows, particles, post, terrain, foliage, water, sky, decals, and culling land on the draw that already exists, and each one feeds GI and frame reuse.

**Architecture.** The G-buffer, the traced shadows, the surface-cache texels, the shape pool, the particle cards, FXAA, and the display curve stay. A new pass is a compute or a raster that runs only when its inputs changed, writes GPU memory, and does not read it back in the frame. New surfaces claim the same world texels a mesh claims. They do not grow a second light cache.

**Bindings.** [ADR 0003](../../adr/0003-one-vulkan-renderer-moltenvk-on-macos.md) is one Vulkan renderer. [ADR 0007](../../adr/0007-native-calls-go-to-the-system-and-vulkan.md) keeps a third-party crate off that renderer. [ADR 0012](../../adr/0012-gi.md) and [GI design](../gi-design.md) are the lighting design. The non-goals there include mesh shaders and Nanite-style virtual geometry. [PR #5](https://github.com/AnthonyPoschen/genos-engine/pull/5) (`docs/frame-reuse.md` on `cursor/frame-output-reuse-6556`, not merged) is the frame-reuse design. The asset cook is [the asset plan](../assets/plan.md). This plan consumes its meshlets, pages, and chunks. It does not cook them.

## What was searched

The branch list is in the asset plan. For this plan the extra findings are:

| Location | Rendering fact |
|---|---|
| `gi-v2` | G-buffer, traced lighting, FXAA, particle cards, flat sky, CPU frustum cull, instance buffer, shape pool. No `shadow.rs`. |
| `master` | `crates/render/src/shadow.rs` projects analytic solids onto the floor. That file is absent on `gi-v2`. Do not bring it back. Lamp shadows on `gi-v2` are traced rays. |
| `mesh-loading` | Loaders only. The draw is unchanged. |
| PR #5 | Scene key, held scene picture, UI damage. The renderer steps in `docs/frame-reuse-plan.md` are not built. |
| `docs/systems/lighting/dynamic-perf-plan.md` on `cursor/dynamic-perf-plan-e94c` | Lighting cost of moving lamps. It is not a material or post plan. |

## Principles

- GPU-resident. Shape blocks, particle images, the mesh-field buffer, and the light caches stay on the device. The CPU uploads a delta. `Renderer::draw` does not wait on a frame fence. `GENOS_LIGHTING_DEBUG` may read counters. A feature's default path may not.
- No fixed per-frame floor. A pass records its dispatch only when a CPU check says something changed, or when a rotating check is due. The check itself is the cheap part and always runs. This is the same rule as the lighting notes: do not skip on an assumption ([lighting README](../lighting/README.md), S74A).
- A new surface that should bounce writes a G-buffer record and is claimed as world texels at 12.5 cm ([S57](../lighting/experiments/S57-no-camera-distance-sizes.md)). Import cards in `.gbake` stay the bake product ([Bake](../bake.md)). The running cache is the texel field, which is the deviation already recorded: pixels read surface-cache texels directly. Do not add a second card atlas for terrain or foliage.
- Particles, fog cards, and unlit quads discard in `gbuffer.frag` because `v_uv.x >= 0.0` or the shade flag says so. That discard stays until a step gives mesh UVs a different signal. A particle does not become a GI surface by accident.
- Frame reuse keys the scene from bytes the CPU already has: the scene block, mesh-field key, draw list, dynamic vertices, shape generation, image generation, the picture filter, wireframe, sky, and size (PR #5). A new pass adds its inputs to that key. A reused frame runs no post pass and no particle step. TAA history is valid only while the key matches. When the key changes, history is dropped for one frame. There is no readback to decide that.
- Published designs are cited on the step that implements them. A deviation is a sentence in that commit and a line in this file.

## Do not rebuild

| Piece | Status | Where |
|---|---|---|
| G-buffer and the depth prepass | done | `shaders/gbuffer.vert`, `gbuffer.frag`, `lighting_gpu.rs` |
| Traced direct light and traced shadows | done | `shaders/lighting_direct.comp`, `lighting_cache.glsl`. Hit shadows are cached per 12.5 cm cell ([S74A](../lighting/experiments/S74A-shadow-static-lamp-shadow-reuse.md)). Per-pixel reuse was reverted ([S77B](../lighting/experiments/S77B-pixel-shadow-reuse.md)). |
| World texel cache, screen probes, compose | done | `lighting_cache.comp`, `lighting_gather.comp`, `lighting_compose.comp` |
| Display curve and dither | done | `tone_channel` in `lighting_compose.comp`, `mesh::tone` in `crates/render/src/mesh.rs`, `shaders/dither.glsl`. The CPU reference in `crates/render/src/trace.rs` inverts the same curve. |
| FXAA | done | `crates/render/src/aa.rs`, `aa_gpu.rs`, `shaders/fxaa.comp`, `crates/render/tests/aa_picture.rs` |
| Flat sky radiance | done | `Scene::sky` in `crates/scene/src/types.rs`. A miss brings `sky.color`. `crates/render/tests/sky.rs`. The stress sun cycle is `examples/stress/src/building.rs`. |
| CPU frustum cull | done | `in_view` in `crates/render/src/world.rs`, applied in `pack.rs` |
| Occluder grid for rays | done | `crates/render/src/occ_grid.rs`, `shaders/scene_rays.glsl`. This accelerates trace queries. It is not raster occlusion culling. `crates/render/tests/occlusion.rs` checks that a wall blocks a lamp. Leave both. |
| Instance buffer | done | `gpu.rs` writes one instance record per mesh draw, and `gbuffer.vert` reads `gl_InstanceIndex`. |
| Shape pool and byte budget | done | `pool.rs`, `budget.rs` |
| Particle cards, resident images, one fire term | done | `crates/render/src/particles.rs`, `pack.rs` atlas, `crates/render/tests/particles.rs`, `particle_frame.rs`, `particle_resident.rs`. Assets `crates/render/assets/ember.png`, `smoke.png`. |
| Analytic mesh draw and glTF placement | done | `DrawKind::Mesh`, `World::add_gltf`, `add_gltf_traced` |
| Displacement grid | partial | `DrawKind::Shader` with `Displacement` builds a grid (`world.rs`). A screen-space shader is recorded and not drawn (`renderer.md`). |
| Indirect lighting dispatch | done | `cmd_dispatch_indirect` in `lighting_gpu.rs`. Do not retarget it. Scene draws stay `vkCmdDraw` until the meshlet step. |
| Debug views | done | `crates/render/src/debug_view.rs`: `full`, `direct`, `bounce`, `albedo`, `normal`, `depth`, `light`, `texels`. |
| GPU path-traced reference | done | `crates/gpuref`. It is the quality gate, not a scene draw. |

## Inventory

| Area | Status | Gap |
|---|---|---|
| Material factors | partial | `PbrMaterial` holds metallic, roughness, normal, occlusion, alpha (`gltf.rs`). The draw stores a mean albedo and a mean emission (`gltf_draw.rs`). |
| Per-texel colour | missing | Mesh vertices are packed with UV `(-1, -1)`. Real UVs would be discarded by `gbuffer.frag`. |
| Shader programs | partial | No custom SPIR-V. `DrawKind::Shader` does not load a file. |
| Lamp shadows outside the GI rays | missing | There is no shadow map. The GI rays are the lamp shadows for anything in the G-buffer. `shadow.rs` on `master` is the removed floor projection. |
| Cascades, virtual shadow maps | missing | Named below. Not started, on purpose, until a caster cannot be traced. |
| GPU particle simulation | missing | The step is CPU. Cards are rebuilt on the CPU and the dynamic vertex range is uploaded. |
| TAA, upscaling, bloom, exposure, DOF, motion blur | missing | Picture modes are `off` and FXAA (`aa.rs`). SSAA left with GI v1. |
| Terrain | missing | The displacement grid is the closest hook. It has no LOD and no texels of its own beyond the triangles it emits. |
| Foliage and instancing | partial | One instance per draw. No instance buffer of many copies. No wind. |
| Water | missing | |
| Atmosphere and weather | missing | The sky is one radiance in every direction, including below the horizon (`scene/src/types.rs`). No sun disc, no aerial perspective. The stress sky colour follows the sun in `building.rs`. That is a game-side colour, not a scattering model. |
| Decals | missing | |
| Mesh LOD | missing | Designed as up to 4 LODs in `gi-design.md`. The cook plan builds the clusters. The draw does not select them. |
| GPU occlusion culling of draws | missing | Frustum cull is on the CPU. |

## Architecture

### Materials

The raster samples `baseColorTexture`, the metallic-roughness texture, and the normal texture into the G-buffer. Albedo in the G-buffer becomes the texture's diffuse albedo (base colour times one minus metallic), which is what `BakeMesh::surface` already computes on the CPU. Metallic and roughness go in a new G-buffer channel when a specular step exists. Until that step they are stored and the lighting BRDF stays the Lambert term in `lighting_compose.comp` (`LAMBERT`). [ADR 0019](../../adr/0019-reflections.md) (accepted) is the specular path. Roughness and metallic stay stored for it. The Lambert picture stays until that path draws the lobe.

**Deviation.** glTF PBR is the Khronos metallic-roughness model ([glTF 2.0](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html)). The picture stays Lambert until a specular commit. The support note for the specular extensions stays `loaded, unsupported` in the asset plan.

The UV signal changes in one place. Packed mesh vertices may carry a real UV. Particles keep a UV with a shade value that still discards. The test that particles are absent from the G-buffer stays.

A material file is not a new language. The glTF material and the text `Material` in `crates/load/src/material.rs` are the two descriptions. Custom SPIR-V stays unloaded. A screen-space shader, when it is drawn, is one of the passes in this plan (post), not an author-supplied program.

### Shadows

Lamps and the sun that hit a G-buffer surface keep the traced ray and the cached visibility they have. GI design already records the deviation from Lumen: Lumen uses shadow maps, and this renderer traces a ray per light through the same tracer as the bounce.

Cascaded shadow maps are the older directional-light method (parallel-split cascades). Virtual shadow maps are the paged shadow atlas in Unreal, described for Fortnite Chapter 4 ([Virtual Shadow Maps in Fortnite](https://www.unrealengine.com/tech-blog/virtual-shadow-maps-in-fortnite-battle-royale-chapter-4)): 128×128 pages, cached until a caster's bounds invalidate them, marked with the same cluster cull as Nanite. **Deviation, and it stays.** Virtual shadow maps want the cluster rasterizer ADR 0012 does not require, and they add a per-frame page-marking cost. Cascades add a second shadow of the same lamp the ray already computes. Neither replaces the traced shadow.

A later map is allowed only for a caster that is not in the tracer. The first candidate is a foliage card or a particle that must cast and that was refused a SDF. That map is one cached sun cascade, invalidated by the caster bounds the CPU already has, and it does not run when no such caster is in view. It is not a project-wide switch. Point lamps stay traced.

### Particles

The CPU emitter, the card builder, the resident image, and the single fire term stay (`particles.rs`). A GPU particle step is a compute dispatch that writes the positions and ages the dynamic vertex buffer already uploads. The dispatch runs when an emitter is enabled and its count is non-zero. A disabled emitter does not dispatch. The CPU keeps the emitter record so the scene key can hash it without a readback. Ages advance on the GPU. The CPU does not read the positions back.

Cards stay out of the G-buffer. An emitter that must bounce uses a proxy mesh (a baked SDF and texels), which is the GI rule for anything that is not a stable surface. The fire term stays one lamp, as `renderer.md` says. One lamp per ember is not added.

The published reference for the GPU step is a persistent particle buffer updated in compute, the same structure as the crowd and particle sections of the SIGGRAPH 2015 GPU-driven pipelines talk (Haar and Aaltonen). **Deviation.** That talk also expands particles into the GPU-driven index buffer. Here the existing card builder remains the expander until the meshlet draw exists. The simulation moves first. The expansion moves with the indirect-draw step.

### Post

The order, when a pass is enabled:

1. Lit HDR colour, before `tone_channel`. Today compose applies the curve. The first post commit splits that: compose writes linear colour into a scene image, and a display pass applies the curve. FXAA moves to the display image, where it is now (it runs on the tonemapped picture).
2. TAA on the linear image, history rejected by velocity and by the scene key.
3. Bloom on the linear image, added before the curve.
4. Exposure, then the existing `tone_channel`. The curve stays the one the reference inverts. A second film curve is a later measurement, not this plan's default.
5. DOF and motion blur after TAA, before the curve, and only when the camera or a velocity buffer says the effect is non-zero.

FXAA stays the mode the panel selects today (`Antialias::Fxaa`). TAA is a third picture mode. It does not remove FXAA.

Sources:

- TAA: Karis, "High Quality Temporal Supersampling", SIGGRAPH 2014 Advances in Real-Time Rendering. Jitter, clip the history to the neighbourhood, and a luminance weight so HDR fireflies do not stick. **Deviation.** Karis accumulates every frame. A reused frame (PR #5) does not run TAA and does not advance history. The check is the scene key, which is cheap and exact.
- Upscaling: a later mode may render the G-buffer at a smaller size and display with the same TAA history. There is no second upscaler until TAA's history is measured. FSR and DLSS are vendor libraries and stay out under ADR 0007.
- Bloom: Jimenez, "Next Generation Post Processing in Call of Duty: Advanced Warfare", SIGGRAPH 2014. A dual-filter pyramid, thresholdless, a wide kernel, and the first downsample weighted so one bright pixel cannot flicker. The pyramid dispatch runs when the scene key changes. A reused frame keeps the last bloom image on the GPU.
- Exposure: a CPU value on the camera, plus an optional average of the histogram the GPU writes. The histogram is read after the fence, not in the submit. The default exposure is the one that keeps `tone_channel` looking as it does now (exposure 1). Auto exposure does not move the picture in a still scene once the histogram has been applied.
- DOF and motion blur: Jimenez's circle-of-confusion and tile-dilated velocity in that same 2014 course. Both are off when the aperture is zero and when no object and no camera moved. The velocity buffer is written by the raster from the previous model matrix the CPU already uploaded. No readback.

The overlay stays after the display pass, as FXAA is before the overlay today (`renderer.md`).

### Terrain

A terrain is a displacement grid, which `DrawKind::Shader` already builds from a height image, plus a fixed texel size so the surface cache claims it like a mesh. LOD is a coarser grid in the cook, not a camera-dependent vertex density. Clipmap terrain (Losasso and Hoppe) recenters rings on the camera. **Deviation.** A camera-centered ring would change which triangles exist as the camera moves, and the lighting rule is that stored light does not depend on the camera. The terrain mesh is baked in world space. The camera only decides which pages are resident, which is the asset plan's residency check.

The height image is an asset. Its encoded bytes stay in the import document. The grid the draw uses is the cooked mesh.

### Foliage and instancing

Instances are the instance buffer that `gbuffer.vert` already reads, filled with many records that share one shape-pool block. The asset plan's `EXT_mesh_gpu_instancing` note becomes `used` in that commit. Wind is a vertex function of time. Time is a CPU uniform. A still time does not change the scene key.

Foliage cards that are not in the tracer do not bounce until they have a proxy, same as particles. A card that is a baked triangle mesh does claim texels. Do not instance the analytic room solids through this path. They stay `DrawKind::Fixed`.

### Water

[ADR 0019](../../adr/0019-reflections.md) is accepted. That lifts the limit that the screen probes are the reflection and that a mirror is a GI non-goal. Water is a Gerstner mesh in world space. It writes the G-buffer, and it shades exact dielectric Fresnel, one Snell refraction, and Beer–Lambert absorption. The phases and the caps are in the [reflections design](../reflections-design.md). The old transparent pass that skipped the G-buffer is not the water path.

The displacement is the Gerstner sum on the scene clock. When time is frozen, the surface does not move and the scene key is stable.

### Sky, atmosphere, weather

The sky colour in `Scene::sky` stays the value a miss returns, so the lighting and the background keep agreeing (`types.rs`). Atmosphere is a function that fills that colour, and an aerial-perspective term on the compose pass, from Hillaire, "A Scalable and Production Ready Sky and Atmosphere Rendering Technique", EGSR 2020 ([paper](https://sebh.github.io/publications/egsr2020.pdf)). The LUTs are small and rebuilt when the atmosphere parameters change, not every frame. Bruneton and Neyret, "Precomputed Atmospheric Scattering", 2008, is the model Hillaire replaces. **Deviation.** Hillaire's LUTs can update in one frame. This renderer rebuilds them only when the parameters change, and a still sky is a stable scene key. The sun disc is a smooth function of the existing sun direction in `building.rs`, drawn as the sky miss, not as a new lamp.

Weather is a parameter block on that atmosphere (cloud cover, fog density). Fog that fills a volume is the GI non-goal for volumetrics. A height fog that only tints the miss and the aerial term is allowed, because it does not trace. A froxel volume is out of this plan.

### Decals

A decal is a small mesh projected in a box, drawn after the opaque depth, with the depth test set so it sticks to the surface. It writes albedo in the G-buffer where it lands, so the existing texel field picks up the new colour on the next claim. It does not add a light. A decal with no pixels in view is not drawn. The CPU check is the decal box against the frustum, which `in_view` already is.

### LOD and culling

LOD selection and occlusion run in compute and fill an indirect draw. The source is Haar and Aaltonen, SIGGRAPH 2015: cull instances, expand clusters, cull clusters against the frustum and a depth pyramid, compact, `multiDrawIndirect`. The depth pyramid is built from the previous frame's depth. A second pass draws what the first pass rejected and the new pyramid accepts, so a camera cut cannot leave a hole. Greene, "Hierarchical Z-Buffer Visibility", SIGGRAPH 1993, is the pyramid.

**Deviation.** The 2015 talk reprojects last frame's depth and accepts a one-frame false negative. The two-pass version in Aaltonen's follow-up is the one to copy, because a missing object for a frame is a pop the lighting tests already treat as a failure. The pyramid build and the cull dispatch run only when the camera or the instance set changed. A reused frame runs neither. The CPU frustum cull in `in_view` stays as the coarse reject before upload, so a mesh behind the camera never enters the instance buffer.

Mesh shaders are not used. The indirect draw uses the hardware rasterizer and the meshlets from the cook. The instance list is the relevancy set from the asset plan. A headless server builds no pyramid and uploads no pages.

## GI and frame reuse, per area

| Area | Surface cache | Scene key adds | When the pass is absent |
|---|---|---|---|
| Textured materials | Texel albedo becomes the shaded albedo. Claim path unchanged. | Material and image generation, already in the key once images bump `image_gen`. | A mesh with no texture keeps the mean colour. |
| Traced shadows | Unchanged. | Unchanged. | No extra pass. |
| Sun cascade for untraced casters | The cascade is not a GI result. GI rays ignore it. | Caster bounds and the sun direction. | No untraced caster in view: no cascade dispatch. |
| GPU particles | Out of the G-buffer unless a proxy mesh is baked. | Emitter record and the dynamic vertex generation. | No enabled emitter: no dispatch. |
| Post | Reads the lit image. Does not write texels. | Picture mode, exposure, aperture, shutter. | Mode `off`, or a reused frame: the display pass is skipped and the held picture is shown. |
| Terrain, foliage meshes, decals | They are triangles. They claim texels. | Their pool generation and poses. | Resident and still: no upload. |
| Water surface | Transparent pass does not claim. A proxy card does. | Time, only when the displacement is on. | Frozen time: key stable. |
| Atmosphere | Miss colour replaces `sky.color` for that frame. | Atmosphere parameters. | Parameters unchanged: LUT dispatch skipped. |
| LOD cull | Does not change stored light. | Camera and the resident cluster set. | Reused frame: no cull. A headless server: no pyramid and no page upload. |

## Networking

Bytes, `AssetId`s, subset hashes, and load profiles are the [asset plan](../assets/plan.md). Interest, `NetId`, the socket, bandwidth, and the session binary are [core-networking/plan.md](../core-networking/plan.md). This plan does not open a socket. That networking file is absent on the remote heads searched on 2026-10-12. The names here are the ones the asset plan and the physics plan already use for it. A later name in that file wins.

The client culls and orders pages for the intersection of the camera residency and the relevant `AssetId`s. A page outside that set is not uploaded. A missing render subset draws the placeholder from the asset plan (the collision mesh, or the bounds).

- **Replicated state.** The G-buffer, the depth pyramid, and the page table stay on the client. A snapshot names the `AssetId`s the asset plan already replicates. This plan adds no picture and no page id to that snapshot.
- **Authority.** The server lists interest. It does not shade. A client keeps its own page table.
- **Prediction / rollback.** Post and the cull are not replayed. TAA history drops when the resident set changes, which is already a scene-key change. A view-body rollback leaves resident pages in place.
- **Determinism.** The picture is local. Two clients with the same relevancy set and the same camera can hold different pages when their bandwidth differed. The sim hash in the asset plan is the shared checksum.
- **Headless server.** `LoadProfile::Sim`. No depth pyramid, no texture upload, no Vulkan device. `in_view` and the cluster cull run on a client.

`genos-stress` stays the lighting host. The rubric moves that coverage to `debug/lighting`. This plan does not add a second lighting binary. `meshlet`, `residency`, and `placeholder` are view modes on `debug/assets`.

## Shared scenes

The lighting host is `genos-stress`. Asset hashes, residency, and relevancy are `genos-assets-view` in the asset plan. Scenes live in `examples/stress/scenes/`. Scripts live in `examples/stress/scripts/`. The room the lighting tests already use is `scenes/room.rhai`. New scenes include that room when they need walls and one lamp, so gi-check still has a meaning.

The owner's GPU command:

```sh
genos-stress --proof --no-panel --size 1280x720 \
  --scene examples/stress/scenes/<scene>.rhai \
  --script examples/stress/scripts/<script>.rhai \
  --report target/debug-reports/<script>
```

Headless tests are `cargo test -p genos-render` for CPU helpers, and the existing picture tests when a change touches the G-buffer. An agent does not start a long stress run. The picture tests that already open a window (`sky.rs`, `aa_picture.rs`, `occlusion.rs`, `particle_frame.rs`) stay the regression for those features. Do not duplicate them.

| Area | Scene | Script | Views |
|---|---|---|---|
| Materials | `scenes/materials.rhai` (the room plus one textured quad) | `scripts/materials.rhai` | `albedo`, `normal` |
| Shadows | `scenes/room.rhai` | the existing `light` view | `light:0` |
| Untraced caster | `scenes/foliage.rhai` | `scripts/foliage-shadow.rhai` | `direct` |
| Particles | `scenes/particles.rhai` | `scripts/particles.rhai` | `full` |
| Post | `scenes/room.rhai` | `scripts/post.rhai` | `full` at exposure 1, which must match today's curve |
| Terrain | `scenes/terrain.rhai` | `scripts/terrain.rhai` | `albedo`, `texels` |
| Foliage | `scenes/foliage.rhai` | `scripts/foliage.rhai` | `albedo` |
| Water | `scenes/water.rhai` | `scripts/water.rhai` | `full` |
| Atmosphere | `scenes/sky.rhai` (open floor, no ceiling) | `scripts/sky.rhai` | `full` against `sky.rs`'s flat colour when the atmosphere is off |
| Decals | `scenes/decals.rhai` | `scripts/decals.rhai` | `albedo` |
| LOD and occlusion | `scenes/lod.rhai` (many copies of one mesh) | `scripts/lod.rhai` | a meshlet-id colouring added to `debug_view.rs` in that step |

`genos-camera` stays the first proof. It does not grow a scene per feature. `genos-gpuref` stays the lighting reference. A material change that alters Room A has to pass the existing gi-check scripts under `examples/stress/scripts/gi_check/` before it is kept. That is a lighting regression, and the step that changes albedo says so.

## Priority

1. Split the display curve out of compose without changing pixels. Everything else in post hangs on a linear image.
2. Mesh UVs into the G-buffer, particles still discarded. Per-texel albedo.
3. Store metallic and roughness. Do not change the BRDF.
4. GPU particle positions, CPU cards unchanged until the numbers move.
5. TAA as a picture mode that frame reuse can skip.
6. Bloom and exposure that leave exposure 1 identical to today's curve.
7. Many instances in the existing instance buffer.
8. Indirect cluster draw and the depth-pyramid cull, after the cook has meshlets.
9. Decals that write albedo.
10. Terrain from the displacement grid, world-space LOD.
11. Atmosphere LUTs that fill `Scene::sky`.
12. Water as a transparent pass plus an optional proxy card.
13. DOF and motion blur.
14. One sun cascade, only for casters that are not traced.

A step that changes the lit picture of `scenes/room.rhai` at the default exposure is a lighting change. It needs the gi-check scripts, not only the new script.

## Steps

Each step is one commit. The headless command is the test to run in the agent. The script is what the owner runs on a GPU. Do not start `genos-stress` here.

### Step 1: Linear scene colour, same curve

Files: `crates/render/shaders/lighting_compose.comp`, `crates/render/src/mesh.rs`, a CPU test next to `mesh::tone`.

Compose writes the linear lit colour into the scene image. A display pass, one compute dispatch, applies `tone_channel` and the dither. FXAA still reads the displayed image. At exposure 1 the displayed bytes match the previous curve. The function `tone` in `mesh.rs` stays the reference. `trace.rs` still inverts it.

Test: `cargo test -p genos-render tone`. A table of linear values (0, 0.64, 1, 4) matches `tone_channel`'s formula in a Rust copy used by the test. The GPU picture test is the owner's:

`scripts/post.rhai` captures `full` on `scenes/room.rhai` and checks the mean luminance against a committed baseline image in the report. The first run records the baseline. A later run fails the script if the mean moves by more than 1 code.

Commit: `render: display curve in its own pass, pixels unchanged`.

### Step 2: Mesh UVs in the G-buffer

Files: `crates/render/src/pack.rs`, `shaders/gbuffer.frag`, `crates/render/src/gltf_draw.rs`.

A mesh vertex may store its `TEXCOORD_0`. The discard that keeps particles out of the G-buffer becomes a shade-flag test, not a UV test. Particles still discard. A mesh with no UV keeps `(-1, -1)` and the mean albedo.

Test: add `#[cfg(test)]` in `pack.rs`. One mesh vertex keeps the UV it was given. One particle card still receives the atlas UV from `atlas_uv`. `cargo test -p genos-render pack_uv`. The existing particle picture tests stay the GPU regression. Do not add a second window test.

GPU proof: `scenes/materials.rhai` places a quad with a checker. `scripts/materials.rhai` captures `albedo`. The checker is visible. `normal` still shows the face normal.

Commit: `render: mesh UVs reach the G-buffer, particles still discard`.

### Step 3: Per-texel diffuse albedo

Files: `gbuffer.frag`, the image atlas upload that particle images already use (`pack.rs`), `gltf_draw.rs`.

The fragment samples the base-colour image from that atlas and writes `base * texture * (1 - metallic)` into the albedo channel. Metallic comes from the factor times the blue channel of the metallic-roughness texture when that texture is present. The lighting BRDF is unchanged. `BakeMesh::surface` is the CPU twin. A test feeds the same texel to both and expects the same albedo within 1/255.

Test: `cargo test -p genos-bake --test bake` stays green. Add `cargo test -p genos-render albedo_matches_bake` as a CPU test that calls the same formula, not a GPU readback.

GPU proof: the materials script. A texel that is black in the texture is black in the albedo view. The mean-colour path remains for a mesh with no texture, and `scripts/materials.rhai` captures that quad too.

Commit: `render: G-buffer albedo samples the glTF base colour`.

### Step 4: Store metallic and roughness, Lambert stays

Files: the G-buffer record in `gbuffer.frag` and `lighting_pack.glsl`.

Add one word to the G-buffer record. Pack metallic in the low 16 bits and roughness in the high 16 bits, both unorm. Compose does not read the word. Albedo, normal, distance, and emission keep the bits they have now.

Test: `#[cfg(test)]` next to the pack helpers. A round trip of metallic 0.25 and roughness 0.5 unpacks within 1/65535. `cargo test -p genos-render pack_material`.

Commit: `render: store metallic and roughness, lighting unchanged`.

### Step 5: GPU particle integrate

Files: `crates/render/shaders/particles.comp`, `crates/render/src/particles.rs`.

The shader reads the emitter record (origin, life, rate) from a uniform the CPU fills and writes the card vertices into the dynamic vertex buffer. The CPU does not read those vertices back. `object_cards` stays the path for an emitter that is not marked `gpu`, including the camera flame. A `gpu` emitter skips `object_cards`. `particle_resident.rs` still proves the image is not reuploaded.

Test: `cargo test -p genos-render --test particles` (CPU path unchanged). A new CPU test simulates one particle for one step in Rust and checks the GPU shader's formula matches, by compiling the formula in Rust next to the shader comment. The owner's `scripts/particles.rhai` on `scenes/particles.rhai` captures frame 0 and frame 60. The ember has moved. The image atlas generation in the log is unchanged.

Commit: `render: GPU particle integrate, resident images unchanged`.

### Step 6: TAA picture mode

Files: `crates/render/src/aa.rs`, `aa_gpu.rs`, `shaders/taa.comp`.

`Antialias::Taa` is a third mode. Jitter is a Halton offset in the projection, stored on the CPU so the scene key changes while the jitter runs. History is a device image. The resolve clips history to the 3×3 neighbourhood and weights by luminance (Karis 2014). When PR #5 says the scene key matched and the frame is reused, TAA does not run and the jitter does not advance. When the key changes, history is cleared.

Test: `cargo test -p genos-render --test aa_picture` still passes for FXAA. A CPU test checks the neighbourhood clip: a history sample outside the neighbourhood is rejected.

GPU proof: `scripts/post.rhai` sets the picture mode, waits 30 frames, captures `full`. A second capture with the camera still matches the first within the script's tolerance. The script records both images for the owner.

Commit: `render: TAA mode, skipped when the scene key matches`.

### Step 7: Bloom and exposure

Files: `shaders/bloom.comp`, `shaders/display.comp` (the step 1 pass), `aa.rs` or a new `post.rs`.

Bloom is the dual-filter pyramid (Jimenez 2014). Exposure is a CPU float, default 1. The display pass computes `tone(exposure * (linear + bloom))`. At exposure 1 and bloom intensity 0, the bytes match step 1. The pyramid runs only when the scene key changes.

Test: `cargo test -p genos-render tone` with exposure 1 and a zero bloom buffer. A CPU test of the first downsample weight: a single bright pixel among zeros is finite and stable.

GPU proof: `scripts/post.rhai` captures exposure 1 and exposure 2. The second is brighter. Exposure 1 stays on the baseline from step 1.

Commit: `render: bloom and exposure in the display pass`.

### Step 8: Many instances, one mesh block

Files: `crates/render/src/gpu.rs`, `world.rs`.

`DrawKind::Mesh` may carry an instance range. The shape pool holds the vertices once. The instance buffer holds N records. `in_view` drops an instance whose bounds miss the frustum before it is written. A still, in-view instance is not rewritten.

Test: `#[cfg(test)]` in `world.rs`: 100 instances behind the camera produce an empty upload list. 100 in front produce 100 records and one shape key.

GPU proof: `scenes/foliage.rhai` with a few hundred copies of one quad. `scripts/foliage.rhai` captures albedo. The copies are visible. `GPU_MS` is in the report and is not asserted.

Commit: `render: many instances share one shape block`.

### Step 9: Indirect cluster draw and occlusion

Files: `shaders/cull.comp`, `gpu.rs`, `debug_view.rs` (meshlet id view).

This step consumes cook meshlets from the asset plan. The instance list is the relevancy set from that plan. The cull shader frustum-culls clusters, tests them against a depth pyramid built from last frame's depth, and fills an indirect draw. A second dispatch draws the clusters the first pass rejected and the updated pyramid accepts. No mesh shaders. The dispatch is skipped when the scene key matches. An empty relevancy set builds no pyramid and uploads no pages.

Test: a CPU replica of the frustum half of the cull, in `crates/render/src/cull.rs`, with one cluster inside and one outside. A third cluster inside the frustum, whose `AssetId` is outside the relevancy set, is absent from the upload. `cargo test -p genos-render cull`. The pyramid test uses a depth buffer of one near pixel and expects the far cluster rejected.

GPU proof: `scenes/lod.rhai`, `scripts/lod.rhai`, meshlet-id view. The owner checks that a camera cut still shows every in-frustum cluster on the second frame (the two-pass guarantee).

Commit: `render: cluster cull and indirect draw, two-pass occlusion`.

### Step 10: Decals

Files: `world.rs` (`DrawKind::Decal`), `gbuffer.frag`.

A decal is a box and a texture. It draws with depth equal to the opaque surface and writes albedo. `in_view` on the box gates the draw. Lighting claims the texels it overwrote through the existing dirty path, because the albedo changed. Do not add a decal light.

Test: CPU bounds test. GPU proof: `scenes/decals.rhai`, `scripts/decals.rhai`, albedo view shows the decal on the floor and not on a surface outside the box.

Commit: `render: decals write albedo into the G-buffer`.

### Step 11: Terrain

Files: the displacement path in `pack.rs`, a cook hook documented in the asset plan (the height image is already an image asset).

The grid is baked in world space at more than one density. The draw selects a density with the cluster LOD from step 9 when meshlets exist, otherwise with a CPU distance test that does not change the baked triangles, only which range is drawn. Stored lighting stays on the finest grid's texels. Coarse levels are a draw LOD, not a second bake.

Test: `cargo test -p genos-render terrain_lod` checks that two camera positions can select different ranges and that both ranges reference the same baked vertex buffer.

GPU proof: `scenes/terrain.rhai`, `scripts/terrain.rhai`, `texels` view. Texel size does not change when the camera dollies (S57).

Commit: `render: world-space terrain grids, camera picks residency only`.

### Step 12: Atmosphere into the sky colour

Files: `shaders/sky_lut.comp`, `crates/scene/src/types.rs` (parameters beside `Sky`, the miss colour stays `Sky::color`).

Rebuild the transmittance and sky LUTs when the parameters change. Compose's miss reads the LUT through the existing sky colour upload, so a lighting shader that already loads `sky` does not grow a second miss path. Aerial perspective is a compose multiply by the transmittance of the pixel's distance. Off, the uploaded colour is the flat `Sky` and `sky.rs` still passes.

Test: `cargo test -p genos-render --test sky` is the owner's window test and is not run here. A CPU test checks that identical parameters produce the same LUT hash and that a second call requests no rebuild.

GPU proof: `scenes/sky.rhai`, `scripts/sky.rhai`. One capture with atmosphere off matches the flat sky. One with it on shows a brighter horizon than zenith. The owner looks at both.

Commit: `render: Hillaire sky LUTs fill Scene::sky`.

### Step 13: Water

Files: `DrawKind::Water` in `world.rs`, a pass after the opaque compose.

The pass samples the lit image, tints it, and does not write the G-buffer. An optional proxy is a normal baked mesh from the asset plan, attached by the caller with `add_gltf_traced`. Time displacement is a uniform. Frozen time does not change the scene key.

Test: CPU: a water object with `affects_light == false` is absent from `World::fields`. GPU proof: `scenes/water.rhai`, `scripts/water.rhai`. The floor colour tints the water. The albedo view of the floor under the water is unchanged.

Commit: `render: water samples the lit image and does not claim texels`.

### Step 14: Motion blur and depth of field

Files: `shaders/motion.comp`, `shaders/dof.comp`, the velocity written by `gbuffer.vert` from the previous matrix.

Both run after TAA and before the curve, and only when shutter or aperture is non-zero and the scene key changed. A reused frame skips them. Velocity comes from the two model matrices the CPU uploaded. The shader does not read positions back.

Test: CPU clip of a tile with zero velocity: the blur kernel is one pixel. `cargo test -p genos-render motion_still`.

GPU proof: `scripts/post.rhai` with a moving solid and a non-zero shutter, one capture. A still capture with shutter zero matches the step 7 baseline.

Commit: `render: motion blur and depth of field, off when still`.

### Step 15: Sun cascade for untraced casters

Files: `shaders/sun_cascade.comp`, a path in the direct light that samples it only for a pixel the tracer did not shade.

This commit does not replace traced shadows. A pixel with a G-buffer hit keeps the ray. The cascade runs when the CPU list of untraced casters is non-empty. Invalidation is the caster bounds against the cascade frustum. No per-frame redraw when that list is still.

Test: `cargo test -p genos-render cascade_gate` with an empty caster list expects no cascade region in the pass list the CPU builds.

GPU proof: `scripts/foliage-shadow.rhai` on a card that is not in `World::fields`. The `direct` view shows a shadow. A traced box in the same scene still matches `light:0` from before this commit, within the script tolerance. If it does not, the cascade is sampling traced pixels and the commit is wrong.

Commit: `render: one sun cascade for casters the tracer does not see`.

## Deviations to keep visible

| Topic | Published | This plan |
|---|---|---|
| Shadows | Lumen and Fortnite use shadow maps and virtual shadow maps. | Traced rays stay the lamp shadow. One cascade is allowed for untraced casters only. |
| Virtual geometry | Nanite software-rasterizes clusters. | Indirect hardware draws of meshlets. ADR 0012. |
| Page priority | Nanite orders clusters from the camera. | The candidate set is the replicated relevancy set. The camera orders inside that set. |
| TAA | Karis advances history every frame. | A reused frame does not advance history. |
| Atmosphere | Hillaire can update LUTs every frame. | LUTs update when parameters change. |
| Terrain | Clipmaps follow the camera. | Pages follow the camera. Triangles stay in world space. |
| Specular | glTF PBR evaluates it. | Stored, not shaded, until a lighting commit. GI non-goal. |
| Water reflections | A screen-space pass. | Screen probes. A mirror stays out. |
| Particles in GI | Some engines inject them into the light cache. | Out of the G-buffer unless the caller bakes a proxy. |
