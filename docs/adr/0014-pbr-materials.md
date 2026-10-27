# 0014: One glTF shade for the picture, the cache, and the path tracer

Status: **Accepted**. Design: [`docs/systems/pbr-materials-design.md`](../systems/pbr-materials-design.md). Anthony delegated design approval to the Engine bot, which accepted this decision.

## Context

The picture, the bake, and the path tracer do not read one material.

- `BakeMesh::surface` (`crates/bake/src/mesh.rs`) returns diffuse albedo `base colour × (1 − metallic)` and emission `emissive texture × factor × KHR_materials_emissive_strength`, with sRGB decoded on colour and emissive images.
- `bake_diffuse` in `crates/render/src/gltf_draw.rs` stores that same diffuse product in the colour atlas. The G-buffer multiplies it by the base-colour factor.
- `surface_mean` in the same file keeps the base colour and does not multiply by `(1 − metallic)`. The comment records why: without a specular lobe, that product turned DamagedHelmet black.
- The G-buffer samples the colour atlas and the normal atlas. The normal is a screen-space derivative tangent (`gbuffer.frag`). Metallic-roughness, occlusion, and the emissive texture are not sampled. Emission in the G-buffer is one value per primitive.
- The CPU path tracer and `genos-gpuref` shade a flat albedo. `crates/debug/src/reference.rs` states that textures are not traced.
- `load_gltf` accepts `KHR_materials_emissive_strength` and refuses every other required extension. There is no BC7 or ASTC cook, no texture page table, and no material hot reload.
- OBJ+MTL and FBX already become `PbrMaterial` on `origin/mesh-loading` at `6ec390c`. Those files are not on this branch.

GI stores irradiance in the world-space surface light cache and refreshes it under per-preset work caps ([ADR 0013](0013-lighting-invalidation-is-local.md)). [ADR 0012](0012-gi.md) already requires emissive surfaces, including a texture on part of a mesh, to light the scene, and it stores roughness and metallic for a later specular lobe. This materials decision does not schedule that lobe. [ADR 0019](0019-reflections.md) (accepted 2026-10-26) is the specular path.

## Decision

1. One **material record** (`PbrMaterial`) and one **shade sample** feed the G-buffer, the surface light cache, and the path tracer. Diffuse albedo is `base colour × (1 − metallic)`. The real-time picture stays Lambert. The path tracer's target is the glTF 2.0 Appendix B BRDF. Metals stay dark in the Lambert picture until that lobe exists. Nothing recolours a metal with its base colour to hide the gap.
2. The cache keeps storing irradiance. Emission is added by the shade at the pixel and at a ray hit. It is not baked into the stored cache value. A material edit is a change record under ADR 0013. It is not a geometry rebuild and it is not a full-frame GI flush.
3. Small or bright emissive regions are still extracted at import into the light tree (ADR 0012). A ray that hits a promoted region for indirect light reads only the emission that was not promoted.
4. Textures are cooked to BC7 on desktop and ASTC 4×4 where BC7 is absent, inside the cook the [asset plan](../systems/assets/plan.md) already names. The runtime does not transcode. Normal maps stay RGBA8. A missing page reads the next coarser resident mip, or the factor colour when no mip is resident. The sample is never black and never a half-written page.
5. The `mesh-loading` parsers are not copied and not edited by this decision. A later commit whose only job is that copy brings them unchanged. They already write `PbrMaterial`.
6. Which mips are resident is decided every frame from the view, not once at load from the image's index in the file. The player cap is `texture_quality` (Ultra is the full mip, High one level coarser, Medium two, Low three). It does not shrink because the card is small. The byte target is the device-local `VK_EXT_memory_budget` after the engine's other allocations and a headroom of `max(5% of heapBudget, 256 MB)`. There is no fixed pool and no per-preset megabyte default. Loads prefer the nearest, most visible textures. Eviction runs only under budget pressure, lowest screen importance then farthest, and a mip that just arrived stays for a dwell unless the heap is over budget. Uploads go to a transfer queue, bounded by the measured disk, decode, and transfer rates, and a page flips only after that submit finishes. The frame does not wait on it.

## Departures

- **glTF 2.0 Appendix B and Filament.** The real-time shade omits the specular lobe. Appendix B's diffuse term is what GI and the Lambert picture use. The path tracer is the place the lobe lands.
- **gi-design, "Emissive surfaces", path 1.** A card texel there stores outgoing radiance. Here the surface light cache stores irradiance, and the shade adds emission. Cards in `.gbake` stay the bake product. They are not the runtime cache (ADR 0013).
- **Current `surface_mean`.** It keeps base colour for metals. That stops being the shade. `metal_diffuse` fails a reader that still does it.
- **glTF normal texture.** The spec requires a MikkTSpace tangent. The G-buffer uses a screen-space derivative. The cook stores the glTF tangent, or a Mikk tangent when the file has none. The derivative path is not the reference.
- **glTF alpha mode `BLEND`.** The raster keeps today's cutoff of 0.5 until the transparent pass in the rendering extras plan. Reference pictures use opaque and mask materials.
- **Basis Universal.** UASTC transcodes to ASTC 4×4 without loss and to BC7 nearly so. The runtime does not transcode. The cook writes the GPU format. `KHR_texture_basisu` stays unloaded until that cook accepts it.
- **Mittring's virtual texture (SIGGRAPH 2008).** The page table is a buffer. Hardware sparse images are not required. This is the asset plan's departure, kept. Eviction is not Mittring's least-recently-used list: while the budget has headroom the camera turning away does not drop a mip, and under pressure the order is screen importance then distance.
- **Vulkan image LOD and `VK_EXT_sampler_feedback`.** The wanted mip is the specification's λ = log₂(ρ) from the draw's screen-space UV derivatives. The CPU computes ρ. Sampler feedback is not a required device feature on the GTX 1060, Polaris, or lavapipe.
- **`VK_EXT_memory_budget`.** `heapBudget` and `heapUsage` on the device-local heaps are the budget. A device without the extension uses half of the device-local heap size. `VK_ERROR_OUT_OF_DEVICE_MEMORY` on a texture allocation evicts and retries.
- **OBJ bump and FBX `Bump` on `6ec390c`.** Those parsers store the image as a tangent-space normal. This decision does not add a height-map reconstruction.

## Rejected

- **OpenPBR 1.1 as the runtime model.** Coat, fuzz, subsurface, and transmission would be stored and ignored. The floor is a GTX 1060, and GI is diffuse.
- **A split shade.** Lumen's cache albedo and a separate path-tracer material graph are the split this engine already has. Two formulas cannot match one reference.
- **Copying `mesh-loading` in the materials work.** The parsers stay on that branch until their own commit.
- **A fixed `residency_mb` or per-preset texture pool.** A 256 MB cap, and later fixed 1–4 GB presets, drop the wrong mips on a card that has room and still overflow a card that does not. The budget is the heap the device reports. `texture_quality` stays a player choice on a small card with a fast bus.

## Consequences

Implementation commits edit [Loading](../systems/loading.md), [Bake](../systems/bake.md), and [Renderer](../systems/renderer.md) when a limit changes. This ADR does not. The P0 gate is `cargo test -p genos-bake --test materials_gate`. It loads `panel.gltf` through `genos-load` and scores `BakeMesh::surface` against hand constants from the IEC curve and the glTF product. It prints `NOT-IMPLEMENTED` for every path that is not in the tree. A later phase replaces a `NOT-IMPLEMENTED` line with a measurement. It does not retune the threshold to make a red picture pass.
