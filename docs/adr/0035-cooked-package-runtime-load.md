# 0035: Runtime load of the cooked package into the #77 streamer

Status: **Accepted**. Extends [ADR 0020](0020-asset-pipeline-import-cook-stream.md) and [ADR 0014](0014-pbr-materials.md) §4. Plan: [asset plan](../systems/assets/plan.md).

## Context

ADR 0020 says the runtime loads cooked records with no transcode, and the asset-loading design (S4) says KTX2 BC7 goes "through the #77 streamer". Neither says how. The gaps are concrete:

- The #77 streamer (`crates/render/src/material_stream.rs`, `World::color_atlas` / `normal_atlas`) streams one packed **RGBA8** atlas per kind. It builds the atlas and its box-filtered `Chain` at load from decoded images. A cooked `.gtex` holds one KTX2 per image, so per-image BC blocks have no slot in that atlas.
- ADR 0014 §4 says "Normal maps stay RGBA8". The cook writes normals as **BC5** (`.gtex`). One of the two has to change.
- No step in the plan says what happens when a `.gcook`/`.gtex` hash does not match, or when the device lacks `textureCompressionBC`. The plan only says an ASTC package exists for such a device and the RGBA8 mips stay in the import document.
- Plan step 16 uploads cooked **meshlet** pages. There is no step for uploading cooked **texture** blocks.

## Decision

1. **Opt-in.** `--cooked DIR` (and `RenderSettings::cooked_dir`) points the runtime at a `genos-cook` output directory. Unset, the runtime path is unchanged. `GENOS_COOKED` is the mirror and applies only when the argument was omitted.
2. **Verify, then fall back per asset.** At load, the runtime reads `DIR/package.manifest` and, for each source, checks the asset id, the `cook=` hash against the `.gcook` SHA-256 trailer, and the `gtex=` hash against the `.gtex`. On any mismatch, missing file or decode error, it logs one `assets fallback <asset> <reason>` line and loads that asset from source as today. A fallback never fails the frame.
3. **Capability gate.** If the physical device lacks `textureCompressionBC` (BC7 and BC5), cooked textures are not used. Geometry from `.gcook` is still used. One `assets fallback textures no-bc` line.
4. **Atlas in block format.** The colour atlas becomes `BC7_SRGB` and the normal atlas `BC5_UNORM` when cooked. Each image lands in a 4-texel-aligned rect, padded to whole blocks per mip. The atlas mip chain is assembled by copying each image's cooked mip blocks into place. Nothing is decoded, filtered or encoded at load. The streamer keeps its rules (finest resident mip sampled, missing page reads the next coarser mip, factor colour if none); only the texel format and the byte count per level change. Images smaller than the mip run out at 4×4 and the atlas stops at the level where any rect would fall below one block.
5. **Normals.** ADR 0014 §4 changes to: normals are BC5 (RG, Z rebuilt in `gbuffer.frag`) on the cooked path and RGBA8 on the source path. `gbuffer.frag` rebuilds Z from RG on both paths, so one shader serves both.
6. **Geometry.** `.gcook` meshes and instances replace the glTF triangle decode and feed the existing shape pool unchanged. Meshlets and LOD stay plan steps 8 and 16. Material factors still come from the glTF, because the cook record does not store them.
7. **Proof.** A lavapipe GPU test renders one fixed view of a fixture scene through both paths and asserts mean absolute per-channel error ≤ 3/255 and a 99th-percentile error ≤ 16/255 on the final LDR image (BC7/BC5 error is expected, a wrong rect or format is not). A fixture report records load time and texture bytes for both paths. Sponza is the same flag on the owner's machine; this checkout does not smudge that asset.

## Rejected

- Decoding BC7 back to RGBA8 at load to reuse the atlas as-is. It costs the load time and VRAM the cook exists to save.
- A separate per-image texture array for cooked images. It forks the #77 sampling path and the `frame_views` logic.
- Failing the load on a bad hash. A stale cook dir in dev must not stop the engine.
- ASTC on the runtime now. No ASTC encoder exists yet; a device without BC falls back to source.

## Consequences

- `crates/render` gains a block-format atlas path, the BC capability check and the Z rebuild in `gbuffer.frag`. `crates/load` reads the package. `crates/cook` keeps meshoptimizer and `intel_tex` out of the renderer (ADR 0007).
- The plan step between 13 and 14 is "Load the cooked package into the shape pool and the BC atlas", with the test in item 7.

## Departures

- The manifest file is `package.manifest`, which is what `genos-cook` writes.
- The colour atlas is the cooked base-colour image, not the RGBA path's `bake_diffuse` product (`base × (1 − metallic)`). The fixture sets metallic to 0 and the base-colour factor to 1, so the two products are the texture. A metal still darkens on the RGBA path and not on the cooked path until the cook stores that product.
- A block atlas is copied mip-to-mip. `vkCmdBlitImage` does not accept BC formats, so a coarser resident mip is fetched with `texelFetch` at that lod instead of being scaled into mip 0.
- `debug/lighting --cooked` passes `bc = true`. `genos-stress --cooked` reads `Renderer::bc_textures`.
