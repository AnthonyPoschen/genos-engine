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

## Amendment A: a valid package does not import the source

Status: **Accepted**. When a manifest line carries `mat=`, `World::add_cooked` loads that asset through `load_covered` and does not call `load_gltf_file`. `World::add_covered` is the same load from the asset id, so the source file can be absent. A line without `mat=` stays the package the decision above already accepts, and decision 6 still applies to that line. A line with `mat=` takes factors and the shade from the record. That is the departure from decision 6.

### Why the accepted decision still imports

Decision 6 says `.gcook` meshes and instances replace the glTF triangle decode, and material factors still come from the glTF because the cook record does not store them. PR #157 measured the cooked load slower than the source load for that reason: the load reads the glTF, then checks the manifest and decodes the record.

Skipping that call drops the factors, the texture slots, and the instance mean. `add_gltf_parts` calls `surface_mean` on the cooked path, and `surface_mean` samples source image texels. `.gcook` version 1 rejects trailing bytes, so the factors cannot be appended to an existing record. Decoding BC7 back to RGBA8 to recover a mean is the option this ADR already rejected.

### Decision, once accepted

1. The cook writes `DIR/cooked/<asset>.gmat` beside `.gcook`. The magic is `GMT1`, the version is 1, and a SHA-256 trailer covers the body. The manifest gains ` mat=<sha256>` after `gtex=` when both are present. A line without `mat=` stays the package this ADR already accepts.
2. The record stores the `PbrMaterial` list (factors, alpha, double sided, texture slots), each image's width and height and no pixels, and one mean and emission per primitive in mesh order. Those two vectors are what `surface_mean` returns. The cook computes them. The renderer does not sample source texels for them.
3. `load_covered(dir, asset_id)` checks the `cook=`, `gtex=` and `mat=` hashes, then returns meshes, instances, materials and cooked image blocks. It does not call `load_gltf`, `import_bytes` or `decode_gltf`, and it does not read the source file. `Package::verify` is still the check that also hashes the cached source objects. It checks `mat=` when the token is present. A line without the token does not look for a `.gmat`. `Package::verify` is `genos-cook --verify`. The frame load does not call it.
4. A hash mismatch or a decode error on a line that has `mat=` logs `assets fallback <source> <reason>` and that asset loads from source, which does call the importer. A missing source file cannot take that path: `add_cooked` has no id to look up, and `add_covered` adds no objects. A fallback does not fail the frame. This is decision 2. A line with no `mat=` token is not an error.
5. `World::add_cooked` calls `load_covered` when the line has `mat=`. It uses the stored shade when one entry exists per primitive. A different count logs `assets fallback <source> no stored shade` and loads from source. The shade index is the primitive's place in mesh order. A second instance of a mesh uses that mesh's entries again. `add_cooked` still reads the source bytes to find the asset id. The call that does not open the source file is `World::add_covered`.

### Prototype in this draft

`genos-cook` writes `.gmat` and `mat=`. `Package::verify` in `genos-load` and `genos-cook` checks that token. `World::add_cooked` calls `load_covered` when the token is present, and `World::add_covered` loads by id. `importer_entries` counts entries to `decode_gltf_phased` on the calling thread. Cook computes the shade with `primitive_shades`. The draw function `surface_mean` is unchanged, so the lossy-cast ceiling stays where it was.

| Test | What it checks |
|---|---|
| `material_record_matches_the_gltf_materials` | `materials.gltf` and `tri.gltf` materials, image sizes and shades round-trip. The decode does not enter the importer. |
| `a_damaged_material_record_does_not_decode` | A flipped magic or trailer does not decode. |
| `covered_package_matches_the_import_with_the_source_removed` | Source file and cached source objects removed. Importer count stays 0. Meshes, instances, materials and image sizes match the import. Cooked image sizes match. |
| `a_cooked_image_size_that_differs_is_corrupt` | A matching `mat=` hash whose stored width disagrees with the cooked image is corrupt. |
| `corrupt_material_record_falls_back_to_the_source` | A bad `mat=` hash. `Package::verify` fails with the material hash. `load_covered_or_source` still falls back to the source file, not the stomped object cache. Materials, meshes and instances match. |
| `stored_shade_is_the_renderer_mean` | Each stored mean and emission equals `surface_mean` for `tri.gltf`. |
| `covered_load_time_against_the_gltf_import` | Same-process medians. Printed. Not a pass threshold. |

### Rejected here

- Bumping `.gcook` to version 2. Records already on disk would stop loading. A sibling file matches `.gtex`.
- Pointing `World::add_cooked` at the prototype before the shade is stored. The instance colour would change.
- Decoding blocks at load to feed `surface_mean`.

### Not in this acceptance

The lavapipe picture test in `crates/render/tests/cooked_package.rs` still builds its package with `package`, which writes no `mat=`. It is not the check for this amendment, and this acceptance does not claim that picture. `debug/assets` is not in the tree. The probe for a bad record is `genos-cook --verify` and the `assets fallback` line on `CookedReport`.

- Replicated state: none. The material record is a local file.
- Authority: the machine that has the file.
- Prediction / rollback: none.
- Determinism: the same record bytes decode to the same materials and shades.
- Headless server: geometry and materials may load from the package. Textures are not uploaded.
