# PBR materials and glTF 2.0 import

Status: **accepted**. ADR: [0014](../adr/0014-pbr-materials.md). The Engine bot accepted the design. Implementation follows the phased rollout below. The measurement gate is `crates/bake/tests/materials_gate.rs`.

Terms used here are defined in [the lighting glossary](../lighting/GLOSSARY.md). New entries from this design live in that file under Materials. One term has one meaning.

## Goal

A glTF 2.0 material looks the same to the G-buffer, the surface light cache, and the path tracer. Metallic-roughness, normal, occlusion, and emissive maps survive the cook. An emissive texel lights the GI. The picture of a dielectric, scored against the path tracer, stays inside the lighting reference gate. A metal is not recoloured to hide a missing specular lobe.

## Non-goals

- A specular, clearcoat, sheen, transmission, or subsurface lobe in the real-time picture. Roughness and metallic are stored. [ADR 0012](../adr/0012-gi.md) already defers the lobe. This design does not schedule it.
- Meshlets, LOD, USD, fracture, audio, or video. Those stay in the [asset plan](assets/plan.md).
- Copying or editing `origin/mesh-loading` (`6ec390c`). That commit already maps OBJ+MTL and FBX onto `PbrMaterial`. A later commit whose only job is the copy brings those files unchanged.
- Radiance cascades, or any GI v1 structure. Removed 2026-10-12.
- A second material language. The text material in `crates/load/src/material.rs` stays a flat colour plus an optional file name. Custom SPIR-V stays unloaded.
- Hardware ray tracing as a requirement. `VK_KHR_ray_query` is optional. macOS uses the software tracer. The shade does not care which tracer hit the surface.
- Sparse images, mesh shaders, and a runtime Basis transcode. The floor is Vulkan 1.3 on a GTX 1060 6 GB and an RX 580, with 4 GB as the minimum VRAM.

## What is already in the tree

| Piece | Where | What it does |
|---|---|---|
| `PbrMaterial` | `crates/load/src/gltf.rs` | Factors, metallic-roughness, normal, occlusion, emissive, `KHR_materials_emissive_strength`, alpha mode, double sided. `SUPPORTED_EXTENSIONS` is only the emissive-strength extension. |
| `BakeMesh::surface` | `crates/bake/src/mesh.rs` | Diffuse albedo and emission at a UV. sRGB on base colour and emissive. Metallic-roughness is linear, metallic in B. Occlusion and the normal map are not read. Wrap is repeat. The sampler's wrap and filter are ignored. |
| Colour and normal atlases | `crates/render/src/gltf_draw.rs`, `gbuffer.frag` | `bake_diffuse` stores sRGB of `linear(base) × (1 − metallic)`. The G-buffer multiplies by the base-colour factor. The normal uses a screen-space derivative tangent. Clamp and mirrored repeat are not in the shader. |
| `surface_mean` | `gltf_draw.rs` | Area-weighted base colour and emission. Metallic does not darken it. That mean is the untextured draw colour and the SDF field's albedo. |
| Path tracer | `crates/debug/src/reference.rs`, `crates/gpuref` | Flat albedo. The reference file says textures are not traced. |
| `.gbake` | `crates/bake` | SDF and six cards. `content_key` is FNV-1a. Cards are not the runtime cache. |
| Surface light cache | ADR 0012, ADR 0013 | World-space irradiance, budgeted refresh, presets Potato, Low, Medium, High. Stored light does not depend on the camera. |
| Asset plan | `docs/systems/assets/plan.md` | Import document, SHA-256 object cache, cook crate, BC7 and ASTC packages, page table, hot reload, `debug/assets`. Not built. |

`load_mesh` stays the strict OBJ triangle reader. `load_gltf` stays the strict projection: a required extension it does not apply is `LoadError`.

## Fit to the renderer and to GI

The frame already rasterizes a G-buffer, traces direct light, and gathers indirect light from the surface light cache under ADR 0013's work caps. Materials join that frame. They do not add a second lighting pass and they do not raise those caps.

The cache stores irradiance. A shade sample turns irradiance into outgoing light:

```text
diffuse albedo = base colour × (1 − metallic)
outgoing       = diffuse albedo × irradiance × occlusion + emissive
```

Occlusion is 1 for direct light. Emissive is `emissiveTexture × emissiveFactor × emissiveStrength`, strength 1 when the extension is absent. Alpha below the mask cutoff contributes nothing.

Because albedo and emission are applied at the shade, a factor edit or a finished texture swap changes the pixel on the next G-buffer. The stored irradiance stays valid until the bounce of that change is due. The change is a change record (ADR 0013): magnitude from the albedo delta or the emission delta, times the bounce reach. Near-camera error keeps the highest weight. Work beyond the cap waits. There is no full-frame flush and no screen-tile fallback.

Potato has no GI. The pixel still adds emission. The room does not bounce it. Low, Medium, and High bounce it through the cache and, for a promoted emitter, through the light tree.

A steady scene does not upload textures and does not rebuild an atlas. The upload cap is a ceiling. A still resident set dispatches nothing. The basic scene stays above 500 fps on the RTX 4070 at 1440p (frame under 2 ms). The stress scene stays near 100 fps. Material work has to fit inside the G-buffer ceilings below. It does not get a new multi-millisecond pass.

Presets are the lighting presets. `day-cycle` and `gi-1bounce` use the Low material budget. They are lighting modes, not texture tiers.

## Alternatives

### A. One glTF metallic-roughness shade

The Khronos glTF 2.0 specification, Appendix B (the BRDF), together with the Filament material model (Romain Guy and Mathias Agopian, *Physically Based Rendering in Filament*, Google, <https://google.github.io/filament/Filament.html>). The diffuse term is Burley's diffuse from *Physically-Based Shading at Disney* (Brent Burley, SIGGRAPH 2012) as Karis specialised it for real time in *Real Shading in Unreal Engine 4* (Brian Karis, SIGGRAPH 2013). The specular term, when it exists, is GGX (Walter et al., *Microfacet Models for Refraction through Rough Surfaces*, EGSR 2007) with the height-correlated masking of Heitz (*Understanding the Masking-Shadowing Function in Microfacet-Based BRDFs*, JCGT 2014). Filament, the glTF Sample Viewer, and Godot 4 ship this model against glTF files.

**This is the pick.** The file is already a glTF material. `BakeMesh::surface` already implements the diffuse and emissive lines. One function can be called from the G-buffer, a cache hit, and the path tracer, which is what a reference match needs.

**Departures from Appendix B and from Filament.** The real-time picture evaluates the diffuse term only. Filament evaluates the specular lobe in the real-time shader. We store metallic and roughness and leave the lobe to a later lighting commit. Until that commit, a metal's Lambert picture is darker than the path tracer, and `picture_metal` stays `NOT-IMPLEMENTED`. A pass that paints the metal with its base colour fails `metal_diffuse`. F0 stays 0.04. `KHR_materials_ior` and `KHR_materials_specular` are kept on the import document and are not shaded.

### B. OpenPBR as the runtime model

*OpenPBR Surface Specification* 1.1 (Andersson et al., Academy Software Foundation, 2024, <https://academysoftwarefoundation.github.io/OpenPBR/>). MaterialX, Arnold, and current Blender author this model. glTF's `KHR_materials_*` stack is a partial encoding of it (coat, fuzz, subsurface, emission, transmission).

**Rejected for this phase.** The extra lobes would be stored and ignored. GI is the diffuse cache. The floor GPU is a GTX 1060. A mapping from OpenPBR into a Lambert picture is a second shade, which is alternative C.

### C. Split G-buffer and path-tracer materials

Unreal Engine 5's Lumen surface cache stores a simplified albedo; the path tracer evaluates the material graph (Wright et al., *Lumen*, SIGGRAPH 2022). The engine already splits: `surface_mean` keeps base colour, `BakeMesh::surface` darkens by metallic, and the path tracer samples neither texture.

**Rejected.** Anthony's picture gate is the path tracer. Two formulas cannot meet it. The current split is the gap this design closes, not the model to keep.

### Texture streaming, as part of the pick

The cook and the page table follow the asset plan: KTX 2.0 (<https://registry.khronos.org/KTX/specs/2.0/ktxspec.v2.html>), Basis Universal UASTC (Khronos 3D Formats Guidelines), and Mittring, *Advanced Virtual Texture Topics*, SIGGRAPH 2008. BC7 is the desktop block format. ASTC 4×4 is Nystad et al., *Adaptive Scalable Texture Compression*, HPG 2012, and it is what UASTC transcodes to without loss.

**Departure from Basis.** The runtime does not transcode. The cook writes BC7 or ASTC. **Departure from Mittring, kept from the asset plan.** The page table is a buffer the shader indexes. `VK_EXT` sparse images are not a required feature on Pascal or Polaris. **Departure from "compress every map."** Normal maps stay RGBA8 on the BC7 path so the path tracer and the GPU read the same normal. BC7 on normals is a poor fit, and a bad normal is a near-camera error.

## Extensions

| Extension or map | Shade | Import note until that phase |
|---|---|---|
| Base colour, metallic-roughness, normal, occlusion, emissive | Used. Phase 2 samples all five. The bake samples base, metallic, and emissive today. | — |
| `KHR_materials_emissive_strength` | Used today. | `used` |
| `KHR_texture_transform` | Used when the UV set is `TEXCOORD_0`. Offset, rotation, then scale, in UV space, as the extension specifies. | `loaded, unsupported` while `load_gltf` still refuses it as a required extension. The phase that applies it adds the name to `SUPPORTED_EXTENSIONS`. |
| `KHR_materials_unlit` | Used in phase 2. The shade returns the base colour, metallic 0, roughness 1, emission 0, and an unlit flag. Compose writes the base colour. The path tracer returns it and stops. GI does not bounce it. | `loaded, unsupported` until that phase |
| Alpha `OPAQUE` and `MASK` | Used. Mask cutoff discards in the depth prepass, the G-buffer, the bake, and the path tracer. | `used` |
| Alpha `BLEND` | The raster keeps the cutoff 0.5 it uses today. | Departure from glTF. The transparent pass stays in the rendering extras plan. Reference fixtures are opaque or mask. |
| `KHR_materials_ior`, `KHR_materials_specular` | Stored. Not shaded. F0 is 0.04. | `loaded, unsupported` |
| Transmission, volume, clearcoat, sheen, iridescence, anisotropy, dispersion, variants, specular-glossiness | Not shaded. Specular-glossiness is not converted. | `loaded, unsupported` |
| `KHR_texture_basisu` | Not decoded in the renderer. | `loaded, unsupported` until the cook accepts the image |
| `TEXCOORD_0` through `TEXCOORD_7` | The CPU shade, the bake cards, and `surface_mean` read the slot's `texCoord`. A missing set falls back to set 0, then to (0, 0). Vertex colour is not sampled. | The G-buffer atlas is still fetched with `TEXCOORD_0`. A second vertex UV needs the layout in `crates/render/src/gpu.rs`. |
| Draco, meshopt | Triangles stay absent, so nothing draws a wrong mesh. | Asset plan. Not this design's phases. |

`load_gltf` keeps returning `LoadError` for a required extension it does not apply. The import document from the asset plan is what keeps the JSON. This design does not invent a second document.

## Data structures

CPU, already the public types, extended only in later phases:

- **Material record.** `PbrMaterial` as it stands, plus the unlit flag and an optional texture transform (offset, rotation, scale) when that phase lands.
- **Shade sample.** `base: [f32; 3]`, `metallic`, `roughness`, `occlusion`, `alpha` as `f32`, `normal_ts: [f32; 3]`, `emissive: [f32; 3]`. Linear. Emission may exceed 1. The G-buffer's shared-exponent word already holds values above 1; a later commit has to keep that range. Clamping emission to 1 is a failed gate, not a feature.
- **Tangent.** The glTF `TANGENT` when the primitive has one (xyz plus the w sign). Otherwise a MikkTSpace tangent computed in the cook (Morten S. Mikkelsen, *Simulation of Wrinkled Surfaces Revisited*, 2008), which is what the glTF spec requires. The sign in w follows the spec. The normal map is not Y-flipped.

GPU, later phases:

- **Material block.** One entry per material record: factors, strength, alpha cutoff, unlit and double-sided bits, sampler word, atlas rectangle or page-table id per map.
- **Texture page.** Fixed 128×128 texels of one mip, plus a one-texel border so a linear sample does not read a neighbour page (Mittring). The page table maps `(image, mip, x, y)` to a physical slot or to a miss.
- **Miss.** The finest coarser mip that is fully resident. If none, the factor colour. Never black. Never a page the upload has not finished.
- **Promoted emitter.** Mesh-local bounds, a normal cone, flux, mean colour, and 64 sample points placed by power. The points are a fixed set. They are not rotated per frame. Instances share the table and place it with the node matrix.

The bake sampler's current repeat-and-nearest filter is a departure from the glTF sampler. The shared shade uses the sampler: repeat, mirrored repeat, or clamp, and linear or nearest as the file says. Mip selection is trilinear inside a resident mip and the miss rule across mips. Anisotropy is `samplerAnisotropy` when the device has it, capped by the preset, and 1 when the feature is missing. It is not a required Vulkan feature.

Mip maps for sRGB images (base colour, emissive) are boxed in linear light and encoded back. Linear images (metallic-roughness, occlusion) are boxed in the stored values. Normal maps are averaged as decoded vectors and renormalised, then stored RGBA8. glTF does not specify the mip filter. This is the Filament choice, recorded here so a later sRGB-space box is a visible change.

## Passes

1. **Decode.** `load_gltf` or, after the copy commit, the OBJ or FBX parser. No GPU. Same bytes, same material record.
2. **Cook.** Offline. Mip chain, BC7 or ASTC for colour, metallic-roughness, occlusion, and emissive. Normals stay RGBA8. Emissive regions are extracted here. The SDF and the cards stay `load_or_bake`. The cook does not build a second SDF. A texture-only change does not change triangle bytes, so the sim hash in the asset plan stays put.
3. **Upload.** At most the preset's byte cap per frame, after the previous frame's fence. The submitting frame does not wait on a readback. The old page stays addressable until the new page is complete, then the table flips.
4. **G-buffer.** One sample of the shade. Writes diffuse albedo, the world normal, emission, and packed metallic and roughness. Compose does not read metallic or roughness until the lobe exists. Unlit writes the base colour and skips the lighting product.
5. **Direct and cache.** A hit evaluates the same shade. Outgoing light is the formula above. Promoted emitters are lights in the light tree, with the same shadow budget as a lamp of that preset. Cache rays do not also add that promoted emission into irradiance.
6. **Compose.** `diffuse albedo × (direct + indirect × occlusion) + emissive`, then the existing tone curve. Unlit skips the product.
7. **Path tracer.** The same shade at the hit. Dielectric fixtures use the diffuse term. The metal fixture uses Appendix B once the lobe exists. Next-event estimation samples lamps and promoted emitters. AO is off in the reference. The occlusion texture is a baked approximation, not a path-traced quantity. **Departure:** the picture may apply occlusion; the reference does not. `picture_dielectric` is scored with occlusion strength 0.

## Emissive surfaces as lights

ADR 0012 and the GI design's emissive section still hold, with the departure in ADR 0014: emission is a shade, not a value baked into the cache.

- Every emissive texel lights indirect through the cache, because a hit adds emission to the irradiance it reads.
- A connected region is promoted when its area is under one L0 cache cell (0.25 m × 0.25 m) or its flux exceeds `emissive_promote_flux`. The default flux is 1% of the brightest lamp, which is the GI design's rule and is not a measured constant. At most 4 regions per mesh on Low, 8 on Medium, 16 on High. Potato promotes none.
- A dim wall stays cache-only. The cache cell is large enough for that and too coarse for a thin strip. The strip is the promoted emitter.
- Double counting: the light tree owns the promoted flux. A cache ray's direct term skips it. An indirect hit on that region reads the leftover emission. A camera ray reads the full emission. That skip is in `lighting_gpu.rs`, which the lighting branch owns. `double_count` stays `NOT-IMPLEMENTED` until that write exists. The note is `model only; waits on lighting_gpu.rs`.
- Turning a promoted emitter on or off is a light change, a multiply on its layer once layers exist, and a change record until then. It does not rebuild the SDF, the BVH, or the page table.
- The furnace is a closed room, albedo 0.8, emission `E`, occlusion off, nothing promoted. Outgoing light settles at `E / (1 − 0.8) = 5E`. `emissive_furnace` stays `NOT-IMPLEMENTED` with the same note. `furnace_wall` in `crates/load/src/emitters.rs` is the radiosity model, not the cache.

## Asset cache and hot reload

Two keys, as the asset plan already split them. This design does not merge them.

- Source bytes: SHA-256, `<cache>/objects/<hex>`. A second import of the same bytes hits the object.
- Bake: FNV-1a `content_key`. Old `.gbake` files stay valid. Do not rehash them with SHA-256.

A reload watches a path given with `--watch`. `GENOS_MATERIAL_WATCH` applies only when `--watch` was omitted. A new hash is a new object. The previous object stays under the old id. The atlas or page table keeps the old pages until the new pages are fully uploaded, then flips. `hot_reload_frames` counts frames from that flip to the G-buffer sampling the new page. Threshold 1. The upload itself may take many frames. That wait is the upload cap, not this metric. A reload does not read the picture back and does not stall the submit.

A texture-only change is a change record. A triangle change goes through the existing bake. Emission is not a geometry edit.

Settings, when the feature exists, follow `docs/engine/settings-principles.md`. `MaterialSettings` is a public `EngineSettings` value. MCP `get_material_settings` and `set_material_settings` read and write it. The debug menu shows the key and the control: `residency_mb`, `mip_bias`, `anisotropy`, `occlusion`, `emissive_promote`, `emissive_promote_flux`. The menu does not carry a description line. The argument wins when both the argument and the environment variable are set.

| Argument | Environment mirror | Preset default |
|---|---|---|
| `--material-residency-mb` | `GENOS_MATERIAL_RESIDENCY_MB` | Potato 32, Low 64, Medium 128, High 256 |
| `--material-mip-bias` | `GENOS_MATERIAL_MIP_BIAS` | Potato +2, Low +1, Medium 0, High 0 |
| `--material-anisotropy` | `GENOS_MATERIAL_ANISOTROPY` | Potato 1, Low 2, Medium 4, High 8 |
| `--material-occlusion on\|off` | `GENOS_MATERIAL_OCCLUSION` | Potato off, others on |
| `--material-emissive-promote on\|off` | `GENOS_MATERIAL_EMISSIVE_PROMOTE` | Potato off, others on |
| `--material-emissive-promote-flux` | `GENOS_MATERIAL_EMISSIVE_PROMOTE_FLUX` | 0.01 of the brightest lamp |
| `--watch PATH` | `GENOS_MATERIAL_WATCH` | unset |

## Mesh loading, proposed only

Do not copy `crates/load/src/fbx.rs` or `crates/load/src/obj_scene.rs` from `6ec390c` in a materials commit. Do not reimplement them. The asset plan's step 2 is the copy, and that commit does not change the parsers.

What that commit already does, from the files at `6ec390c`:

- OBJ `usemtl` / `mtllib` becomes one `PbrMaterial` per material. `Kd` is the base colour. `Ke` is emissive. `d` and `Tr` set alpha. `Ns` becomes `roughness = sqrt(2 / (Ns + 2))`. Metallic is 0. `map_Kd`, `map_Ke`, and `map_Bump` / `bump` / `norm` become the base, emissive, and normal slots. A missing image keeps the factor. V is flipped to glTF's convention.
- FBX Phong/Lambert colours, shininess, and diffuse, normal, and emissive textures become the same record. Shininess uses the same roughness formula. `Bump` is stored as the normal slot.

This design adds no second mapping. After the copy, those records go through the same shade as glTF. The bump-as-normal choice is that parser's. OBJ bump maps are often height. This design does not add a height reconstruction, and it does not strip the normal slot. The support note on the copy commit should say so. `load_mesh` stays strict and does not grow an MTL reader.

## Cost per preset

Ceilings, not measurements. Lighting VRAM is ADR 0013's and is unchanged. Material residency sits outside that lighting budget and inside a 4 GB card. The G-buffer milliseconds are the depth prepass plus the shaded raster on top of today's cost. The phase that adds the maps measures them. `gbuffer_ms` in the gate is the High, RTX 4070, 1440p, basic-scene ceiling. The 1060 numbers wait for a 1060. gi-design already records that the lab does not have one.

| | Potato | Low | Medium | High |
|---|---|---|---|---|
| Target | any discrete GPU, GI off | GTX 1060 6 GB, RX 580, 1080p | RTX 2060 class, 1080p | RTX 4070, 1440p |
| Max edge | 512 | 1024 | 2048 | 4096 |
| Colour, metallic-roughness, occlusion, emissive | BC7, or RGBA8 if the device lacks `textureCompressionBC` | BC7 | BC7 | BC7 |
| Normals | off | RGBA8 | RGBA8 | RGBA8 |
| Resident cap | 32 MB | 64 MB | 128 MB | 256 MB |
| Upload cap | 0.25 MB/frame | 1 MB/frame | 2 MB/frame | 4 MB/frame |
| Anisotropy | 1 | 2 | 4 | 8 |
| G-buffer maps | base colour | base, metallic-roughness, normal | plus occlusion | plus emissive |
| G-buffer ceiling, 4070, 1440p, basic | 0.40 ms | 0.45 ms | 0.50 ms | 0.60 ms |
| G-buffer ceiling, 1060, 1080p | 1.60 ms | 2.00 ms | — | — |
| Promoted emitters per mesh | 0 | 4 | 8 | 16 |
| Shade | Lambert, no normal, no occlusion | Lambert, normal, occlusion | same | same, metallic and roughness stored |

BC7 is 1 byte per texel. A mip chain adds about a third. 256 MB at High is about 192 M texels of BC7, which covers a Sponza-sized set (the loader notes 69 images) with room for a second scene. RGBA8 normals are the expensive extra. High caps the normal atlas with the same residency budget. When the budget is full, the farthest image's finest mip is the one that is not resident. The miss rule then reads the coarser mip. The frame does not stutter to make room.

A basic scene that is one room of flat materials uploads nothing after the first settle and adds no cache rays. The 0.60 ms High ceiling plus ADR 0013's lighting cap still has to leave the basic frame under 2 ms. If a measurement misses that, the feature cuts maps from the preset. It does not raise the frame budget.

## Networking

Gameplay networking still waits. These lines are the contract.

- **Replicated state.** `AssetId` (SHA-256 of the source bytes) and the material-record hash for assets referenced by bodies in interest. Texture pages stay on the client. A BC7 package and an ASTC package share the sim hash and differ in the render hash, as the asset plan says.
- **Authority.** The server names the `AssetId`. The client loads it. A local sim hash that differs from the snapshot is a fault.
- **Prediction / rollback.** None for a material. Until the render subset arrives, the draw uses the factor colour. A rollback leaves resident pages where they are.
- **Determinism.** The same source bytes and the same cook settings produce the same shade factors on every machine. Block compression may differ by `bc7_colour_rmse` and stays out of the sim hash. The picture is not a sim input.
- **Headless server.** `LoadProfile::Sim`. No texture upload, no Vulkan device, no page table. Collision from the asset plan. Factors are not drawn.

## Debug tool and tests

`debug/assets` is the binary the asset plan specifies, on the lighting debug standard (`docs/systems/lighting/debug-standard.md`): scene picker, checkpoints, reset, replay, screenshots, metrics, sliders, a guided-review panel, and a headless CLI or MCP drive. This design does not build that binary. The P0 probe is the gate below. Later phases add views to the binary and to `crates/render/src/debug_view.rs`: `albedo`, `normal`, `roughness`, `metallic`, `occlusion`, `emissive`, `mip`, `residency`. `albedo` and `normal` already exist. The menu labels are those words. They do not come with an explanation.

Rhai gates, once `debug/assets` exists, live in `debug/assets/scripts/` and run on that binary, not on `genos-stress`:

| Script | What it checks |
|---|---|
| `materials.rhai` | A dielectric quad and a metallic quad. `albedo` matches the shade. The metallic quad is dark. |
| `materials-emissive.rhai` | The furnace and a small strip. `emissive_furnace` and `double_count`. |
| `materials-stream.rhai` | A walk from coarse to fine. Resident bytes stay under the cap. No black frame. `GPU_MS` has no stall line. |
| `materials-reload.rhai` | `--watch` a file whose hash changes. `hot_reload_frames` ≤ 1 after the new page is resident. |

Picture grading uses the lighting reference check (`docs/systems/lighting/reference-check.md`). The score is the blurred relative error. The per-pixel term is `|Yc − Yr| / max(Yr, 0.25 × mean(Yr), 0.02)`. The fail line is 0.12. It is not scaled by the reference noise and it is not nudged per scene. FLIP is reported and is not the bounced-picture verdict. The path tracer and the GPU sample the same cooked texels. Scoring the GPU's BC7 against the path tracer's PNG would punish the compressor twice. The PNG-to-BC7 gap is `bc7_colour_rmse` only.

The grade runs with occlusion strength 0, mip bias 0, and a still camera, after the reference check's settle rule. Lavapipe proves the files and the exit code. It does not prove milliseconds. The owner's pictures are the RTX 4070 at 1440p.

Forbidden, and called out in review if they appear:

- Using base colour as the diffuse albedo of a metal so the Lambert picture looks lit.
- Clamping emissive strength to 1.
- Skipping the sRGB decode.
- A black page while a mip is on its way.
- A temporal blend on a mip flip. Display smoothing was tried and reverted (S71E/S71F). A finished page replaces the coarser mip on the next G-buffer. One mip is the boundary.

### Metrics

PASS when `value ≤ threshold`. `NOT-IMPLEMENTED` when the feature is not in the tree. A missing feature does not score 0. FAIL when a measured value is above the threshold. `NOT-IMPLEMENTED` does not fail the process. One FAIL does.

The gate prints one line, `name value threshold STATUS`, and writes JSON under `./genos-debug/materials/`. Numbers use six digits, so 1/255 prints as 0.003922. `--out` overrides that directory. `GENOS_MATERIALS_GATE_OUT` applies only when `--out` was omitted. `--case good` loads `crates/bake/tests/fixtures/materials/panel.gltf` with `load_gltf_file` and reads `BakeMesh::surface` and `BakeMesh::from_gltf`. `--case bad` reads a deliberate broken shade in the test (sRGB applied twice, no `(1 − metallic)`, no emissive strength). `GENOS_MATERIALS_GATE_CASE` applies only when `--case` was omitted. Expected colours are constants written out from the IEC curve and the glTF product. The test does not call the shade to build them.

```text
cargo test -p genos-bake --test materials_gate -- --case good
cargo test -p genos-bake --test materials_gate -- --case bad
cargo test -p genos-bake --test materials_gate -- --nocapture --case good
```

The binary is not a libtest harness, so it already prints. `--nocapture`, `--test-threads`, `--skip`, and `--format` are accepted and ignored, because `cargo test -- --nocapture` forwards those flags to every test binary. The good process also grades the broken shade and exits 2 if that shade would pass a shade metric, or if a missing feature comes back as PASS. File checks (`mask_alpha`, `texcoord_set`, `emissive_default`, `texel_alpha`) stay measured on the broken reader, because that reader does not replace the loader or the mask clip. `texel_alpha` is the raw texel, which the broken reader also returns. The bad process is expected to exit 1. `cargo test` without arguments runs the good case.

| Metric | Threshold | What the number is |
|---|---|---|
| `shade_albedo` | 1/255 | Max absolute error of diffuse albedo against the hand constants, across the 2×2 texels and repeat samples outside 0..1. The file's sampler is nearest and repeat, which is what the bake implements. Clamp, mirror, and linear filtering stay phase 1. |
| `shade_emissive` | 1/255 | Max absolute error of emission. The shade material has no strength extension, so the constant uses strength 1. The bright material uses strength 2. |
| `srgb_decode` | 1e-5 | Max absolute error against the once-applied IEC 61966-2-1 decode, including the linear piece (byte 10) and the power piece (byte 188). |
| `srgb_double` | 1e-5 | How far the sample sits inside a 0.05 margin of the twice-applied decode. 0 means it is outside that margin. A second decode lands on 0.05 and fails. |
| `metal_diffuse` | 1/255 | Max channel of a fully metallic sample. The hand value is 0. |
| `mr_channels` | 1/255 | Error at the texel whose metallic-roughness image is G=255, B=128. The hand value uses B (`127/255` on a white base). Reading G would be 0. |
| `mask_alpha` | 0 | Wrong keep/drop count. Alpha is base alpha 0.5 times texture alpha, cutoff 0.5. Products: 64/255 drops, 0.5 stays (the spec keeps `alpha >= cutoff`), 0 drops. |
| `texcoord_set` | 0 | Wrong-slot count. The emissive slot's `texCoord` is 1, the other slots are 0, and the primitive UVs are `TEXCOORD_0` `(0.25, 0.25)`, not set 1's `(0.75, 0.75)`. |
| `texcoord_sample` | 0 | Wrong-slot count at the shade triangle's first corner. Base comes from set 0 (once-decoded 188). Emission comes from set 1 (texel 255). Sampling set 0 for emission is a wrong slot. |
| `emissive_default` | 0 | Absolute error of the loaded strength against 1 on the material that omits `KHR_materials_emissive_strength`. |
| `roughness_from_g` | 1/255 | Absolute error of roughness against `0.5 × 10/255`. The fixture stores roughness in G. Reading B is 0 at that texel and fails. |
| `texel_alpha` | 1/255 | Absolute error of the base-colour texel alpha before the factor, at the three mask UVs (bytes 128, 255, and 0). The product with the mask factor 0.5 fails the first sample. |
| `mip_chain` | 0 | Wrong-chain count through `load_gltf`. A 2×2 flat sRGB 188 image keeps a chain down to 1×1 that is still byte 188. A 2×2 normal map of opposite X encodings averages to (0, 0, 1). A chain with no 1×1 level fails. |
| `draw_mean` | 1/255 | `surface_mean` against the same shade. The gate does not link `genos-render`. |
| `occlusion_indirect` | 1/255 | Occlusion scales indirect only. The bake does not return occlusion. |
| `normal_angle_deg` | 0.5 | Angle between the shaded normal and the tangent-space reference. |
| `picture_dielectric` | 0.12 | Blurred relative error, metallic 0. The path tracer does not sample textures. |
| `picture_metal` | 0.12 | The same score for a metal under Appendix B. Stays unimplemented until the lobe exists. |
| `emissive_furnace` | 0.02 | `NOT-IMPLEMENTED`. Model only; waits on lighting_gpu.rs. |
| `double_count` | 0.02 | `NOT-IMPLEMENTED`. Model only; waits on lighting_gpu.rs. |
| `bc7_colour_rmse` | 0.02 | `NOT-IMPLEMENTED`. Open question 1 is unchosen. RGBA8 is the cook. A stand-in encoder is not an RMSE. |
| `astc_colour_rmse` | 0 | `NOT-IMPLEMENTED`. UASTC to ASTC 4×4 would be lossless, so the threshold stays 0. No transcode is linked. |
| `residency_mb` | 256 | Resident mebibytes from `plan_residency`, the cap `load_gltf` applies. The farthest finest mip drops first. |
| `hot_reload_frames` | 1 | Frames from `HotReload`'s finished upload to the sample that sees the new page. The old page stays until that upload finishes. |
| `gbuffer_ms` | 0.60 | High, RTX 4070, 1440p, basic scene. |

## Phased rollout

Each phase is its own commit and its own measurement. A phase that misses a threshold is dropped or cut back. The threshold is not edited to match the result.

| Phase | Change | Gate that moves off `NOT-IMPLEMENTED` |
|---|---|---|
| P0 | This design and `materials_gate`. The fixture is a 2×2 glTF. | `shade_albedo`, `shade_emissive`, `srgb_decode`, `srgb_double`, `metal_diffuse`, `mr_channels`, `mask_alpha`, `texcoord_set`, `emissive_default`. |
| P1 | One shade function shared by the bake and a CPU hit test. Sampler wrap and filter included. The shade reads the slot's UV set and returns roughness from G. | The P0 shade metrics stay, now on that function. `texcoord_sample` and `roughness_from_g` move off `NOT-IMPLEMENTED`. |
| P2 | G-buffer samples metallic-roughness, normal, occlusion, and emissive. `surface_mean` uses the shade. Metallic and roughness are stored. Lambert stays. | `draw_mean`, `occlusion_indirect`, `normal_angle_deg`. |
| P3 | The CPU path tracer samples the shade. Dielectric pictures use the reference check. | `picture_dielectric`. `picture_metal` stays unimplemented. |
| P4 | Emission at cache hits, promoted emitters, furnace, double-count switch. | `emissive_furnace`, `double_count`. |
| P5 | Cook writes BC7 and ASTC mips. Normals stay RGBA8. Residency cap enforced. | `bc7_colour_rmse`, `astc_colour_rmse`, `residency_mb`. |
| P6 | Page table and the miss rule. | `materials-stream.rhai` on the owner's machine. Lavapipe checks the counters. |
| P7 | `--watch` hot reload. | `hot_reload_frames`. |
| — | `gbuffer_ms` | Measured on the 4070 when phase 2's maps exist. Not a reason to build a pass early. |

The mesh-loading copy is not a row. It is the asset plan's step 2.

## Open questions

1. Which BC7 encoder is vendored into `genos-cook` only. Not into `genos-render`. Unchosen. RGBA8 remains the honest cook until that vendor lands.
2. Low-preset milliseconds on a GTX 1060 and an RX 580. The lab does not have those cards. The 2.00 ms Low ceiling is the gi-design estimate, not a measurement.
3. When the specular lobe is scheduled. Not in the phases above. `picture_metal` waits for it. Closing that metric by changing the diffuse product is a rejected change, not an answer to the question.

## Networking note for the implementing commit

The five lines in the networking section are copied into the loading system record by the commit that first ships the shade, as the agent rubric requires. This design does not edit that record. The limits there are still true.
