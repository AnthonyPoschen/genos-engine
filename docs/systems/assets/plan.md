# Asset plan

Written 2026-10-12 against `gi-v2` at `e61b89d`. This is a plan. It does not change Loading, Bake, or the renderer. A later commit edits [Loading](../loading.md), [Bake](../bake.md), or [Physics](../physics.md) in the same change that changes those facts.

**Goal.** A game can import a wide set of files, keep every byte the runtime does not use yet, cook the parts the GPU does use, and load the result with no window and no GPU. A headless server loads the collision subset of those same files. The client and the server name each file with one hash.

**Architecture.** Loading stays the decoder. A new import document sits in front of the decoders that already work: it stores the source bytes, the runtime projection those decoders already return, and a support note for every feature the projection drops. The cook reads that document and writes GPU-ready meshlets, texture blocks, and the existing `.gbake`. The renderer keeps the shape pool, the particle images, and the mesh SDF upload it has now.

**Bindings.** [ADR 0007](../../adr/0007-native-calls-go-to-the-system-and-vulkan.md) keeps third-party crates off the window, the input, the renderer, the audio device, and the network. [ADR 0012](../../adr/0012-gi.md) and [GI design](../gi-design.md) reject mesh shaders and Nanite-style virtual geometry, because those need features above the Pascal and Polaris floor. A cook library that never owns a draw is named below as a cook-only dependency, not a renderer dependency. Words are in `CONTEXT.md`: an asset is data a game holds after a load; a byte source supplies the bytes; Loading does not play audio and does not upload to a GPU.

## What was searched

On 2026-10-12, `git branch -r` and `git ls-remote --heads origin` listed:

| Branch | What it adds for this plan |
|---|---|
| `gi-v2` (`e61b89d`) | The base. glTF, OBJ triangles, PNG, JPEG, WAV, fonts, bake, SDF upload. |
| `master` (`751bee7`) | Same loaders. `crates/render/src/shadow.rs` is the old floor-polygon shadows. The FBX commit was reverted here on purpose. |
| `mesh-loading` (`6ec390c`) | OBJ+MTL scenes, binary and ASCII FBX 7, `load_model_file`. Tests in `crates/load/tests/models.rs`. |
| `gi-v2-prefill` | Lighting only. |
| `cursor/frame-output-reuse-6556` | [PR #5](https://github.com/AnthonyPoschen/genos-engine/pull/5). Frame reuse design. Not merged. |
| `cursor/dynamic-perf-plan-e94c` | Lighting performance plan. Not this work. |
| `cursor/gi-prefill-spare-budget-3544`, `cursor/gi-headless-stuck-patch-2bf4`, `cursor/wrc-need-ray-ceilings-3c49` | Lighting. |
| `cursor/cloud-agent-notes-4faa`, `cursor/cloud-env-notes-d5a7` | Environment notes. |
| `cursor/physics-architecture-plan-a077` (`cfec5bd`) | [Physics plan](https://github.com/AnthonyPoschen/genos-engine/blob/cursor/physics-architecture-plan-a077/docs/systems/physics/plan.md). `.gfract`, `BondBreak`, `break_baseline`, and the content hash on a destruction spawn. Not merged. |
| `cursor/agent-rubric-b06d` (`e2f451b`) | Debug binaries under `debug/<system>/`, the observability list, and the five networking lines. Not merged. |

`docs/systems/core-networking/plan.md` is not on `gi-v2` or on any other remote head listed above, as of this search. The physics plan is the contract that names that file. It assigns the session, the UDP socket, `NetId`, `InputFrame`, the snapshot ring, interest, bandwidth, and the session debug binary to that file. This plan uses those names. When that file lands, a name there wins, and this plan changes in the same commit.

The cook stores the chunks and the bond graph. The step that breaks a bond stays in the physics plan. The live physics record on `gi-v2` remains [Physics](../physics.md).

## Principles

These bind every step.

- GPU-resident data. The frame loop uploads a new block or a new page. It does not read a buffer back to decide the next draw. A completed frame's feedback may be read after its fence, the same way `poll_gpu_times` reads timestamps. The current submit does not wait on that read.
- Work is gated by need. A cook, an upload, or a stream runs when a key or a page is missing. A still resident set does not dispatch a streaming pass.
- Verify cheaply. Each frame the CPU checks the resident set against the relevancy set, the camera, and the hashes it already holds. A skip without that check is not allowed.
- Follow a published design, and write the deviation in the same commit.
- New triangles, terrain, foliage, water surfaces, and decals that should bounce light claim surface-cache texels the way a mesh does today. The running cache is the world texel field (12.5 cm, [S57](../lighting/experiments/S57-no-camera-distance-sizes.md)), not a second atlas. Import cards in `.gbake` stay the bake product. See [Bake](../bake.md).
- A cooked change dirties the scene key in [PR #5](https://github.com/AnthonyPoschen/genos-engine/pull/5) (`docs/frame-reuse.md` on `cursor/frame-output-reuse-6556`). Frame reuse itself is that plan. This plan only names the keys a cooked asset adds: shape-pool generation, image generation, and the mesh-field key. Those generations already exist as the pool and `upload_image`.

## Do not rebuild

| Piece | Status | Leave it here |
|---|---|---|
| Byte sources | done | `crates/load/src/source.rs` (`FileSource`, `MemorySource`) |
| PNG and JPEG to RGBA8 | done | `crates/load/src/png.rs`, `jpeg.rs`, `inflate.rs` |
| glTF 2.0 triangles, materials, images, node matrices | done | `crates/load/src/gltf.rs`, `crates/load/tests/gltf.rs`, `crates/load/examples/gltf_info.rs` |
| Strict OBJ triangles | done | `crates/load/src/mesh.rs`. `load_mesh` stays strict. The scene OBJ parser is the other function. |
| Text material and texture-map text | done | `crates/load/src/material.rs`, `texture.rs` |
| Text transform keys and `Quat::slerp` | done | `crates/load/src/animation.rs`. Sampling stays. Driving a mesh is still open. |
| WAV PCM and the mixer | done | `crates/load/src/wav.rs`, `crates/audio`. The device stays in `crates/audio/src/device.rs`. |
| Simple TrueType outlines | done | `crates/load/src/ttf.rs` |
| In-repo BVH | done | `crates/mesh` |
| SDF, six cards, FNV-1a `.gbake` | done | `crates/bake`. `BakeMesh::content_key` in `crates/bake/src/mesh.rs`. `genos-bake`. |
| SDF upload | done | `World::add_gltf_traced` in `crates/render/src/gltf_draw.rs`, `crates/render/src/mesh_field.rs` |
| Shape pool | done | `crates/render/src/pool.rs`. Identical shapes share one block. |
| FBX and OBJ+MTL | done on `mesh-loading` | `crates/load/src/fbx.rs`, `obj_scene.rs`, `crates/load/tests/models.rs` at `6ec390c`. Copy those files. Do not write a second parser. |

`load_gltf` stays strict. A file that requires an extension it cannot draw returns `LoadError` from `load_gltf`, as `crates/load/src/gltf.rs` does now (`SUPPORTED_EXTENSIONS` is only `KHR_materials_emissive_strength`). The new import path is the one that keeps the payload. Callers that draw today keep calling `load_gltf`.

## Inventory

`done` is on `gi-v2` and used. `partial` is decoded or stored, and the runtime drops or ignores a part. `elsewhere` is on `origin/mesh-loading` and not on `gi-v2`. `missing` has no code.

| Item | Status | Where |
|---|---|---|
| glTF `.gltf` / `.glb`, triangle lists, strips, fans | done | `gltf.rs`. Points and lines return `None` from `primitive` and are dropped. |
| Sparse accessors, `byteStride`, normalized integers, `data:` URIs | done | `gltf.rs` `Reader::read`, `read_uri` |
| Metallic-roughness, textures, samplers, alpha, double sided, emissive strength | partial | Stored on `PbrMaterial`. The draw uses the mean albedo (`gltf_draw.rs` `surface_mean`). |
| Skins, morph targets, animations, cameras, lights | partial | The JSON is parsed only as far as the mesh walk. Those arrays are not copied. `loading.md` says they are ignored. |
| `TEXCOORD_0`, normals, tangents | done | `Primitive` |
| `TEXCOORD_1` and up, `COLOR_0`, `JOINTS_0`, `WEIGHTS_0` | missing | `primitive` reads only `POSITION`, `NORMAL`, `TEXCOORD_0`, `TANGENT`. |
| Required extension other than emissive strength | missing | `decode_gltf` returns `LoadError::Unrecognized`. The file is not kept. |
| Draco, `EXT_meshopt_compression`, `KHR_texture_basisu`, `KHR_texture_transform` | missing | Refused when required. Named in the `gltf.rs` module note. |
| OBJ `v` `vt` `vn` `f` | done | `mesh.rs`. Unknown tags are errors. A polygon becomes a triangle fan. |
| OBJ+MTL scene (`usemtl`, `map_Kd`, `map_Ke`, bump) | elsewhere | `obj_scene.rs` on `6ec390c`. Other tags are skipped there, not stored. |
| FBX 7 binary and ASCII meshes, materials, embedded images, axes | elsewhere | `fbx.rs` on `6ec390c`. The file header lists skins, blend shapes, animation, cameras, lights, and NURBS as not read. |
| USD, USDA, USDC, USDZ | missing | |
| PNG 8-bit, JPEG baseline and progressive | done | Limits in `loading.md`: edge ≤ 8192, no GIF, WebP, BMP, DDS, KTX, or block compression. |
| WAV 16-bit mono or stereo | done | `wav.rs`. OGG, MP3, and FLAC are named as not decoded in `loading.md` and `audio.md`. |
| Video | missing | |
| Composite glyphs, kerning, hinting | missing | `ttf.rs` returns an error for a composite glyph. |
| Content-addressed cache for source files | done | `crates/load/src/cache.rs` `store_object`, `store_import`: SHA-256 (`sha256.rs`) at `<dir>/objects/<hex>`, temp file then rename. The bake cache stays FNV-1a. |
| Import document | done | `crates/load/src/import.rs` `ImportDoc`, `import_file`. Unused glTF arrays are kept as JSON with a `Note`. `examples/import_report.rs`. |
| Offline cook to `.gcook` | partial | `crates/cook` (`genos-cook`). Geometry, instances and the bake key, with input hashes and a SHA-256 trailer. Images cook to a separate `.gtex` (next row). Materials, meshlets and LOD are not cooked yet. The renderer does not read `.gcook`. |
| Meshopt, meshlets, LOD | missing | GI design names up to 4 LODs. None are built. |
| BC7, ASTC, KTX2, Basis | partial | `crates/cook/src/ktx2.rs` writes and reads KTX2 (BC7 unorm / sRGB, BC5, ASTC 4×4 formats named). `crates/cook/src/texture.rs` cooks every glTF image with `genos_load::mip_chain` in the space from `image_spaces`: colour BC7 sRGB, data BC7 unorm, normals BC5. Encoder `intel_tex_2` (ISPC), a cook-only dependency in place of the step 12 Basis vendor. Record `<asset>.gtex`, same input hashes, settings hash and SHA-256 trailer as `.gcook`; `genos-cook --textures`. No ASTC encode yet. The runtime does not read `.gtex`. |
| Virtual textures, mesh streaming, GPU-driven scene draws | missing | Lighting uses `vkCmdDispatchIndirect` (`lighting_gpu.rs`). Scene draws use `vkCmdDraw`. |
| Hot reload of a cooked file | missing | Shaders are compiled by `crates/render/build.rs` at build time. |
| Package manifest | done | `crates/cook/src/package.rs`. One line per asset; `genos-cook --verify DIR` checks every hash. |
| Pre-fractured chunks | missing | `crates/physics/src/mesh.rs` builds at most 6 boxes or one 12-vertex hull. That is a collider, not a fracture. |
| Bake cards on the GPU | partial | `cards.rs` writes them into `.gbake`. `add_gltf_traced` uploads the SDF only. |

## Support matrix

Two layers. **Import** is the document this plan adds. **Runtime** is what a draw, a mix, or a bake may use. `used` means the runtime reads it. `loaded, unsupported` means the import kept the bytes and wrote a support note, and the runtime projection omitted them. `error` means the bytes are not a file of that kind (bad magic, truncated). A truncated file stays an error. An unsupported feature inside a readable file does not.

The note is one line: the path in the file, the feature name, and `used` or `loaded, unsupported`. `import_report` prints those lines. A later runtime commit moves a row from `loaded, unsupported` to `used` without reading the source file again.

### Containers

| Feature | Import target | Runtime today | Runtime target until a later commit |
|---|---|---|---|
| glTF 2.0 JSON and GLB | Keep the source bytes and the decoded scene | `used` for meshes, materials, images, instances | Same. `load_gltf` stays the projection. |
| glTF required extension the decoder does not apply | Keep the extension JSON and the buffers it points at | `error` from `load_gltf` | Import: `loaded, unsupported`. `load_gltf` still returns `LoadError`. |
| `KHR_draco_mesh_compression`, `EXT_meshopt_compression` | Keep the compressed buffer | `error` when required | `loaded, unsupported` until the cook decoder lands. Triangles stay absent, so nothing draws a wrong mesh. |
| Wavefront OBJ triangles | Keep the text | `used` by `load_mesh` | Same. |
| OBJ+MTL | Keep the OBJ, the MTL, and the decoded scene from `obj_scene.rs` | `elsewhere` | `used` for the scene projection after the `mesh-loading` files are copied. Groups, smoothing, and curves: `loaded, unsupported`. |
| FBX 7 binary and ASCII | Keep the file and the scene `fbx.rs` already builds | `elsewhere` | `used` for meshes, materials, textures, transforms. Skins, blend shapes, animation stacks, cameras, lights, NURBS: copy the node payload and mark `loaded, unsupported`. |
| USDA text | Parse `def Mesh` and `def Material`. Keep the rest of the text | missing | Meshes and materials `used` when the small parser lands. Other statements `loaded, unsupported`. |
| USDC and USDZ | Keep the bytes | missing | `loaded, unsupported` until a crate-file reader exists. Do not link OpenUSD. |
| PNG, JPEG | Keep the encoded bytes and the RGBA8 `Image` | `used` | Same. The encoded bytes are what a later cook compresses, so a 16-bit or a third format can be added without a new import. |
| GIF, WebP, BMP, DDS, EXR | Keep the bytes when the magic matches | missing | `loaded, unsupported`. |
| KTX2, BC7, ASTC, Basis UASTC / ETC1S | Keep the file. A KTX2 header the reader understands records the vkFormat | missing | Cook output is `used` by the GPU upload once that step lands. A KTX2 the reader cannot transcode is `loaded, unsupported` and the original image bytes remain. |
| WAV PCM 16-bit | Keep the RIFF and the `Pcm` | `used` by `genos-audio` | Same. |
| Float WAV, ADPCM, Ogg, Opus, MP3, FLAC | Keep the bytes. Record channels and rate when the header is readable | missing | `loaded, unsupported`. The mixer stays 16-bit interleaved. |
| MP4, WebM, image-sequence folders | Keep the bytes. Record duration when the header says it | missing | `loaded, unsupported`. No decoder in the frame. |
| TrueType | Keep the file and the simple glyph outlines | `used` for simple glyphs | Composites, kerning, and hinting: `loaded, unsupported`. |
| Text material, texture map, text animation | Keep the text and the decoded value | `used` | Same. The animation still does not move a mesh until a later runtime commit. |

### glTF features

| Feature | Import target | Runtime today |
|---|---|---|
| Node tree, TRS or matrix, one scene | `used` | `used` (`MeshInstance::transform`) |
| Indexed triangles, strips, fans | `used` | `used` |
| Points and lines | Keep the accessor | Dropped. Target: `loaded, unsupported` |
| Missing normals | `used` (flat normals, vertices unwelded) | `used` |
| `TEXCOORD_0`, tangents | `used` | Stored. The G-buffer discards a vertex whose UV x is ≥ 0 (`gbuffer.frag`), and packed meshes write UV `(-1, -1)` (`pack.rs`). So the UV is not sampled. |
| Further UV sets, vertex color | Keep the accessors | `loaded, unsupported` |
| Metallic-roughness factors and textures | `used` in the document | Mean albedo and mean emission on the draw (`gltf_draw.rs`). Factors are `used` by that mean. Textures are `used` by the bake sampler (`BakeMesh::surface`) and `loaded, unsupported` by the raster. |
| Normal, occlusion | Stored on `PbrMaterial` | `loaded, unsupported` by the raster. Bake does not sample them. |
| `KHR_materials_emissive_strength` | `used` | `used` by the mean emission and by `BakeMesh::surface` |
| Alpha mask and blend, double sided | Stored | Mask and blend are not a separate raster pass. Double sided sets the bake `thin` path. Mark mask and blend `loaded, unsupported` for drawing until a pass exists. |
| `KHR_materials_unlit`, transmission, volume, IOR, specular, clearcoat, sheen, iridescence, anisotropy, dispersion, variants | Keep the extension object | `loaded, unsupported` |
| `KHR_texture_transform` | Keep the transform | `loaded, unsupported`. `load_gltf` still refuses it when it is required. |
| `KHR_lights_punctual` | Keep the light nodes | `loaded, unsupported`. Scene lamps stay the Rhai `light` call. |
| `KHR_materials_emissive_strength` | `used` | `used` |
| Skins (`JOINTS_0`, `WEIGHTS_0`, inverse bind) | Keep the accessors and the skin JSON | `loaded, unsupported`. GI design says a skinned mesh uses a proxy, not an exact SDF. |
| Morph targets and weights | Keep the deltas | `loaded, unsupported` |
| Animation clips (translation, rotation, scale, weights) | Keep the samplers | `loaded, unsupported`. The text `Animation` type stays the one the game samples today. |
| Cameras | Keep the camera JSON | `loaded, unsupported`. The game camera stays `genos-scene`. |
| `EXT_mesh_gpu_instancing` | Keep the instance attributes | `loaded, unsupported` until the instance buffer consumes them. The node instances already drawn are `used`. |
| `KHR_mesh_quantization` | Keep the quantized accessors | Already dequantized by `component` when the accessor is normalized. Mark the extension `used` once a fixture proves a quantized file. |
| Sparse accessors | `used` | `used` (`tri.gltf` test) |
| Images: PNG, JPEG, buffer view, `data:` URI | `used`, and the encoded bytes are kept | `used` as RGBA8 |
| Basis / WebP images inside glTF | Keep the image bytes | `loaded, unsupported` |

### FBX and OBJ rows that the copy must report

These are the gaps in `6ec390c`, from the file headers. Copying the parsers does not close them. The import document records them.

| Feature | After the copy |
|---|---|
| FBX geometry, materials, embedded `Video`, axis conversion to Y-up metres | `used` |
| FBX skins, blend shapes, animation, cameras, lights, NURBS, property-template defaults | `loaded, unsupported` |
| OBJ `v` `vt` `vn` `f` `usemtl` `mtllib` | `used` |
| OBJ groups, smoothing, curves, `vp` | `loaded, unsupported` |
| MTL `Kd` `Ke` `Ns` `d` `Tr` `map_Kd` `map_Ke` `map_Bump` | `used` when the image is PNG or JPEG |
| An MTL map that is not PNG or JPEG | `loaded, unsupported`, factor kept |

## Recommended architecture

### Import document

One type, `ImportDocument`, in `crates/load`. It holds:

- `source`: the exact bytes, and the path when there was one.
- `kind`: the magic or the extension that selected the decoder.
- `scene`: `Option<GltfScene>`, filled by `load_gltf`, by `load_obj_scene`, or by `load_fbx`. OBJ triangles from `load_mesh` become one primitive in that scene so the cook has one mesh shape.
- `images_encoded`: the encoded image bytes beside each decoded `Image`.
- `pcm`: `Option<Pcm>` when the file is WAV.
- `kept`: named blobs (`skin`, `morph`, `animation`, `extension`, `usd-rest`, `audio`, `video`) with their bytes.
- `notes`: `Vec<SupportNote> { path, feature, state }` where `state` is `Used` or `LoadedUnsupported`.

`import_file` and `import_bytes` build this. They do not open a window, a device, or a GPU. `load_gltf` and the other `load_*` functions stay the projections. `crates/load/examples/gltf_info.rs` stays. A new example `import_report` prints the notes.

A file that is not a known magic and has no known extension is still `LoadError::Unrecognized`. That is an error, not an unsupported feature.

### Cook

A new crate `crates/cook`, package `genos-cook`, `[lints] workspace = true`. It depends on `genos-load`, `genos-mesh`, and `genos-bake`. It does not depend on `genos-render` or `genos-window`.

The cook writes a `.gcook` next to the import. The mesh section is meshlets and LOD levels. The texture section is mip blocks. The GI section is the existing `.gbake` key from `load_or_bake`. The cook does not reimplement the SDF or the cards.

Mesh work uses meshoptimizer ([meshoptimizer](https://meshoptimizer.org/), Arseny Kapoulkine). Vendor it under `crates/cook/third_party/meshoptimizer` and call it from the cook binary only. Do not add it to `genos-render`. The algorithms to call, in order, are the ones the library documents: vertex-cache optimization, overdraw optimization, `meshopt_buildMeshlets`, then `meshopt_simplify` for the LOD. Cluster limits for the indirect-draw path are 64 vertices and 64 triangles, which is inside the usual 64 / 124 mesh-shader budget and does not require `VK_EXT_mesh_shader`.

LOD follows the cluster hierarchy in Karis, "A Deep Dive into Nanite Virtualized Geometry", SIGGRAPH 2021 Advances in Real-Time Rendering ([slides](https://www.advances.realtimerendering.com/s2021/Karis_Nanite_SIGGRAPH_Advances_2021_final.pdf)). Groups of clusters simplify together with a locked boundary, and each cluster stores the parent's error so a later GPU cull can pick a level from one sphere. **Deviation from ADR 0012.** Nanite then software-rasterizes the visible clusters into a visibility buffer with mesh shaders or a compute rasterizer. This cook stops at the clusters, the error, and the bytes. The draw stays `vkCmdDrawIndexedIndirect` on the hardware rasterizer. Mesh shaders and the software rasterizer stay out, because the floor is the GTX 1060 and the RX 580 (`docs/systems/gi-design.md`, non-goals and the feature table).

Texture blocks follow the KTX 2.0 specification ([registry](https://registry.khronos.org/KTX/specs/2.0/ktxspec.v2.html)) and Basis Universal UASTC ([Khronos 3D formats note](https://github.com/KhronosGroup/3D-Formats-Guidelines/blob/main/KTXDeveloperGuide.md)). UASTC transcodes to ASTC 4×4 losslessly and to BC7 nearly so. **Deviation.** The runtime does not transcode. The cook writes BC7 (`VK_FORMAT_BC7_UNORM_BLOCK` / `_SRGB_BLOCK`) into the desktop package and ASTC 4×4 into a package for a device without `textureCompressionBC`. The import still holds the encoded source image, so another package can be cooked later. The Basis transcoder, if vendored, links only into `genos-cook`. Until that vendor lands, the cook writes an RGBA8 mip chain and records `loaded, unsupported` for block compression.

The bake stays the GI cook. Card texel size stays a bake setting (`BakeSettings::card_texel`), never a camera distance ([S57](../lighting/experiments/S57-no-camera-distance-sizes.md)).

### Cache and package

Two keys, on purpose.

- Source and import: SHA-256 of the source bytes. Path `<cache>/objects/<hex>`. The import document is stored beside it. A second import of the same bytes hits the object.
- Bake: the existing FNV-1a `content_key`. Do not rehash `.gbake` with SHA-256. Old caches stay valid.

Writes use a temporary name and `rename`, as `genos-bake` does.

A package is a manifest of those hex names plus the cook records a platform needs. The game ships the manifest and the objects. There is no second archive format until a measurement says the directory is too slow to open.

### Streaming

Virtual textures follow Mittring, "Advanced Virtual Texture Topics", SIGGRAPH 2008 ([notes](https://advances.realtimerendering.com/s2008/SIGGRAPH%202008%20-%20Advanced%20virtual%20texture%20topics.pdf)): a mip chain, a page table, a physical atlas of fixed pages, and a border so linear filtering does not read a neighbour page. **Deviation.** The page table is a buffer the shader indexes. Hardware sparse images are not a required feature on the floor, so they are not the path.

Mesh streaming follows the same page idea as Nanite's cluster pages (Karis 2021): the hierarchy stays resident, the cluster bytes stream. The cull that fills the indirect draw follows Haar and Aaltonen, "GPU-Driven Rendering Pipelines", SIGGRAPH 2015 ([slides](https://advances.realtimerendering.com/s2015/aaltonenhaar_siggraph2015_combined_final_footer_220dpi.pdf)): instance cull, cluster cull, compact, `multiDrawIndirect`. Occlusion for that cull is the rendering plan.

The candidate pages are the assets in the relevancy set, defined in the Networking section. The CPU orders those pages from the camera and the resident set it already has. The GPU may write a request list. The CPU reads that list only after the frame's fence, and the current frame does not wait. A cheap CPU check still runs every frame so a missed read cannot hide a missing page. A headless server does not build this table.

### Hot reload

When a source file's SHA-256 changes, the import and the cook for that object are rebuilt. The new bytes are a new `AssetId`. The previous object stays in the cache under the old id. Relevancy names the id the scene references after the reload. The renderer swaps a shape-pool block or an image when the cook key changes, which is the same generation the pool and `upload_image` already bump. That generation is part of the frame-reuse scene key. A reload does not read the picture back. The pool swap is step 16. The file watch is step 17.

### Pre-fractured destruction

The cook, not the physics step, builds the chunks. The pattern is a Voronoi decomposition of the mesh bounds, clipped to the mesh, as in Müller, Chentanez, and Kim, "Real Time Dynamic Fracture with Volumetric Approximate Convex Decompositions", ACM SIGGRAPH 2013 ([paper](https://matthias-research.github.io/pages/publications/fractureSG2013.pdf)). Each chunk is a mesh in the import document, with its own bake key so GI cards and the SDF exist per chunk.

The bond graph is the cook's adjacency: one node per chunk, one edge per shared face, with a rest area and a strength. **Deviation.** The paper also fractures at runtime from an impact and rebuilds islands. This cook stores one precomputed pattern. Runtime fracture, island detection, and the solver belong to the physics plan. Today's collider (`Shape::from_triangles`, at most 6 boxes or 12 hull vertices, `crates/physics/src/mesh.rs`) stays the contact shape until that plan says a chunk is a body. The asset step does not call `step`.

The chunk collision and the bond graph are the sim subset. Their bytes are the `.gfract` version 1 payload from the physics plan (`write_gfract`, little-endian `f32`, parent, `first_child`, `child_count`, bond area, centroid, normal, health, and a world bond of `u32::MAX`). The SHA-256 of that payload is the content hash that plan sends on spawn when the client already has the file. SDF cards and texture blocks stay on the render subset, so a BC7 cook and an ASTC cook share one physics hash. `BondBreak`, `break_baseline`, and the snapshot codec stay in the physics plan and in the core-networking plan. When `crates/fracture` lands, the cook calls `write_gfract` and hashes those bytes. It does not grow a second fracture file.

## Networking

[core-networking/plan.md](../core-networking/plan.md) owns the session, the UDP socket, `NetId`, `InputFrame`, the snapshot ring, interest, bandwidth, and the session debug binary (one server world and N client worlds in one process, with latency, loss, and jitter). This section is the asset contract that file calls. Loading and the cook store `AssetId` and the subset hashes. They do not open a socket, resend a packet, or define the codec.

That file is absent on every remote head searched on 2026-10-12. The names here are the ones the [physics plan](https://github.com/AnthonyPoschen/genos-engine/blob/cursor/physics-architecture-plan-a077/docs/systems/physics/plan.md) already assigns to it. A later name in `core-networking/plan.md` wins.

Gameplay networking still waits ([Architecture](../../architecture.md)). The loopback agent endpoint stays the live-scene path. These lines are the contract for the server that file will add.

### Identifiers

`AssetId` is the SHA-256 of the source bytes, the object key from the cache section. The server and the client compute the same id from the same bytes. A path is a byte source on one machine. `NetId` is the server-assigned id of a replicated body. A body keeps its `NetId`. The asset that body uses is named by `AssetId`.

The bake key stays the FNV-1a `content_key` of `.gbake`. Old bake caches stay valid. The wire names the source `AssetId` and the subset hashes below.

A subset hash is the SHA-256 of that subset's canonical bytes. The canonical sim bytes are little-endian, version byte `1`, then bounds, collision positions, and indices, and for a breakable the `.gfract` payload above. The render bytes are the meshlets, then the KTX2 block buffer for one platform, then the bake key stored as bytes. The audio bytes are the WAV PCM when the file has any. Two cooks of the same source and the same cook settings write the same sim bytes and the same sim hash. A BC7 record and an ASTC record share that sim hash and differ in the render hash. If the physics plan or the core-networking plan names a different function for the `.gfract` content hash, that function wins and this paragraph changes with it.

`missing_subsets(asset_id, profile)` returns the subset names whose hash is absent locally. That list is what the session sends, under the bandwidth budget the core-networking plan owns. A snapshot names `AssetId` and the subset hashes for assets referenced by `NetId`s in interest. Once the peer has stored a hash, a later snapshot names the hash and leaves the bytes out.

### Load profiles

| Profile | Who | What it loads |
|---|---|---|
| `Sim` | Dedicated server, and any headless sim | Collision the physics record already builds (`Shape::from_triangles`, at most 6 boxes or 12 hull vertices), chunk bounds, the bond graph, asset bounds |
| `Render` | A client | The sim subset, plus meshlets, the mip blocks for this platform, and the bake key |
| `Listen` | A server that also draws and plays | The sim subset, the render subset, and the PCM |

`Sim` leaves texture blocks, meshlets, SDF cards, and PCM on disk. It creates no Vulkan device and opens no audio device. `Render` leaves the other platform's blocks on disk. The server loads the sim subset for every asset the sim references, including a body outside one client's interest.

### Relevancy

Interest is the set of `NetId`s a connection may see. The core-networking plan owns that set. Relevancy for assets is the `AssetId`s those `NetId`s reference, plus the view body's asset. The session maps `NetId` to `AssetId` and passes the ids. The cook stores no `NetId`.

The client page table requests pages only from that set. The camera orders the requests inside the set, which is the streaming section. An `AssetId` that leaves interest stays for `relevancy_hold` frames and then becomes an eviction candidate. A page the camera can see, whose asset is outside the set, is not requested.

The server does not upload texture pages and does not build a page table.

### Five lines

- **Replicated state.** `AssetId` and the subset hashes for assets referenced by `NetId`s in interest. Subset bytes travel when `missing_subsets` names them. Texture pages and meshlet pages stay on the client.
- **Authority.** The server lists the `AssetId`s in a connection's interest set. The client loads those ids. A local sim hash that differs from the hash the snapshot names is a fault `asset_metrics` counts. The sim keeps the server's bytes.
- **Prediction / rollback.** An asset has no predicted state and no rollback. Until the render subset arrives, the draw is the collision mesh when the sim subset is present, and the asset bounds when it is absent. The view-body rollback in the physics plan uses the sim subset already loaded. A rollback leaves resident pages in place.
- **Determinism.** The sim hash is stable across processes that share the source bytes and the cook settings. The comparison on the wire is that hash, plus the quantized poses the physics plan already compares. Render blocks may differ by platform and stay outside the sim hash.
- **Headless server.** `LoadProfile::Sim`. Collision, chunk bounds, and the bond graph are loaded. Texture pixels stay on disk. There is no window, no Vulkan device, and no audio device.

## Assets debug binary

The binary is `debug/assets`, package `genos-assets-view`, `[lints] workspace = true`. The folder is the one the agent rubric names for this system (`debug/assets` on `cursor/agent-rubric-b06d`). `genos-stress` stays the lighting host. This binary is the observability host for import, cook, cache, residency, and relevancy.

It uses `genos-debug` and the MCP command queue in [Debugging](../debugging.md). MCP `tools/call`, `POST /cmd/<name>`, and Rhai share one JSON form. New probes are added to `crates/debug/src/commands.rs`. The binary does not mount `reference` or `compare`, and it does not open the session window. The session binary in the core-networking plan is the multi-client view. This binary publishes `asset_checksum` (the sim hash of each relevant `AssetId`, in `AssetId` order) and the resident-set byte count on `asset_metrics`, so that view can read them.

Scenes are loaded through `genos-scene`, the same files `genos-stress --scene` already accepts. Today those files are `examples/stress/scenes/` and `examples/camera/scene.rhai`. When `debug/scenes/` exists, the files move once and this binary loads that directory. It does not keep a second copy of `room.rhai`.

The scaffolding:

- A scene picker at launch, and a hotkey that switches scenes.
- Checkpoints `room_hall`, `room_far` (the camera far enough that page priority drops the near pages), and `fracture`.
- `reset` and `key_r` restore the start: an empty resident set, the scene's starting relevancy, and the default knobs. `key_r` is bound on a debug action map in this binary. It is not added to `character_controller()`.
- A script runs from frame 0 on a fixed step. The metrics JSON is a function of the scene, the profile, and the frame count.
- `capture` and `--shot PATH` write a PNG through `genos_debug::png::write_png`. `--metrics PATH` writes the JSON.
- Live knobs: `cache_budget` (page bytes), `profile` (`sim`, `render`, `listen`), `force_evict` (one `AssetId`, or the oldest page), `relevancy_hold` (frames).
- The overlay (`Tools::overlay`) lists the failure modes and the probe that exposes each one.
- Headless drive over the CLI and over MCP.

Commands this binary adds, beside the catalogue's `status`, `time`, `step`, `wait`, `knobs`, `view`, `capture`, and `quit`:

| Command | What it returns |
|---|---|
| `scene` | Load one scene file. Returns the `AssetId`s it references. |
| `reset` | The start state above. |
| `asset_metrics` | Cache objects, sim subsets loaded, render subsets loaded, pages resident, pages missing, bytes uploaded since the last frame, evictions, `evicted_relevant`, hash mismatches, unsupported notes, `asset_checksum`, resident bytes. |
| `asset_dump` | One `AssetId`: source hash, subset hashes, profile, byte sizes, resident pages, notes, and `error` when the load failed. |
| `asset_notes` | `SupportNote` rows (`Used` or `LoadedUnsupported`) for one asset or the whole set. |
| `resident_set` | Page ids and the byte total. |
| `relevancy` | `AssetId`s in interest, and the hold list. |
| `asset_snapshot` | The resident set and the relevancy set, as JSON. |
| `asset_restore` | Put that JSON back, so a failure can be repeated. |

View modes, selected with `view`: `meshlet` (meshlet id), `residency` (resident and missing pages), `placeholder` (bounds or the collision mesh where the render subset is absent). `albedo` and `depth` stay the modes `debug_view.rs` already has. `--headless` selects `LoadProfile::Sim` and does not create a device.

| Failure | Probe |
|---|---|
| A subset the snapshot named is absent locally | `asset_metrics` field `missing`. `asset_dump` names the subset. |
| The local sim hash differs from the snapshot | `asset_metrics` field `hash_mismatch`. `asset_dump` shows both hashes. |
| A truncated file | `asset_dump` state `error` with `LoadError::Truncated`. |
| A feature the draw does not use | `asset_notes` state `LoadedUnsupported`. |
| A page evicted while its `AssetId` is still relevant | `asset_metrics` field `evicted_relevant`. `resident_set` and `relevancy` together show the page and the set. |

Headless:

```sh
genos-assets-view --headless --scene examples/stress/scenes/room.rhai --frames 8 --metrics target/assets-metrics.json
genos-assets-view --script debug/assets/scripts/relevancy.rhai --report target/debug-reports/assets-relevancy
```

Exit code is 1 when a check fails. Script shorts match [Debugging](../debugging.md): `step(n)`, `shot(name)`, `knob(name, value)`, `check`, `require`.

## Shared scenes

There is no scene-library crate. The shared set is the stress host:

- `genos-stress --scene FILE` loads one Rhai scene and moves nothing (`examples/stress/src/main.rs`).
- `genos-stress --script FILE` runs a debug script ([Debugging](../debugging.md)).
- `genos-stress --gltf FILE` places a model. After the FBX copy, `--model` is the name on `mesh-loading`; keep `--gltf` working.
- `examples/stress/scenes/room.rhai` is the one shared scene file today.
- `examples/camera/scene.rhai` and `examples/camera/room.rhai` are the camera proof.

New scenes go in `examples/stress/scenes/` until `debug/scenes/` exists. New checks go in `examples/stress/scripts/` or in `debug/assets/scripts/`. Feature checks share the stress host and the assets binary. They do not add a binary per feature. Headless proofs stay `cargo test -p genos-load`, `cargo test -p genos-cook`, and `genos-assets-view --headless`. The owner's GPU proof on the stress host is:

```sh
genos-stress --proof --no-panel --size 1280x720 \
  --scene examples/stress/scenes/room.rhai \
  --gltf <file> \
  --script examples/stress/scripts/assets-place.rhai \
  --report target/debug-reports/assets-place
```

`import_report` does not need a GPU. `genos-bake` stays the bake binary. `genos-cook` is the cook binary, added with the cook crate.

| Area | Scene | Script or test | What the owner looks at |
|---|---|---|---|
| Import notes | none | `cargo test -p genos-load import_report` | The note list. |
| glTF, FBX, OBJ placed | `scenes/room.rhai` plus `--gltf` | `scripts/assets-place.rhai` | Albedo and depth views of the model in the room. |
| Texture cook | `scenes/room.rhai` | `scripts/assets-texture.rhai` | A checker that survives BC7 on the albedo view. |
| Meshlets and LOD | `scenes/room.rhai` | `scripts/assets-lod.rhai` | A meshlet-id colouring, then the far LOD. |
| Streaming | `scenes/room.rhai` | `scripts/assets-stream.rhai` | Pages fill in as the camera walks. No stall line in `GPU_MS`. |
| Hot reload | `scenes/room.rhai` | `scripts/assets-reload.rhai` | Replace the file on disk. The next settled frame shows it. |
| Fracture chunks | `scenes/fracture.rhai` | `scripts/fracture.rhai` | Chunks drawn as meshes. Bonds are a debug overlay. The physics step is unchanged. |
| Relevancy and the server profile | `scenes/room.rhai` | `debug/assets/scripts/relevancy.rhai` | `genos-assets-view` in `placeholder`, then `residency`. Headless metrics show zero texture bytes. |

## Priority

1. Import document and the support notes, wrapped around `load_gltf`. No new formats.
2. Copy FBX and OBJ+MTL from `mesh-loading`, and record the features those parsers skip.
3. SHA-256 object cache. Leave `.gbake` keys alone.
4. Keep encoded images, extra glTF accessors, and extension blobs.
5. Identify audio and video containers. Decode only WAV, as now.
6. USDA meshes into the same document. USDC stays bytes.
7. Cook crate that passes triangles through and calls `load_or_bake`.
8. meshoptimizer meshlets and LOD in the cook.
9. RGBA8 mips, then BC7 and ASTC in KTX2 from the cook only.
10. Package manifest.
11. Page table and cluster streaming on the CPU, then the GPU upload.
12. Hot reload through the existing pool generation.
13. Voronoi chunks and the bond graph.
14. Sim and render subset hashes. The sim hash is the `.gfract` content hash.
15. Relevancy gates the page request list. A headless server stays on `LoadProfile::Sim`.
16. A placeholder while the render subset is absent.
17. `debug/assets`, with the probes in the debug-binary section.

Streaming, fracture, the profiles, and the debug binary wait until the document and the cook exist. They have nothing to hash or show before that. Relevancy waits on the page table.

## Steps

Each step is one commit. Headless tests do not open a window. Visual scripts are for the owner's GPU. Do not run the stress host in an agent session.

### Step 1: Import document around glTF

Files: `crates/load/src/import.rs`, `crates/load/src/lib.rs`, `crates/load/tests/import.rs`, `crates/load/examples/import_report.rs`.

`import_bytes` on a `.glb` or `.gltf` calls `decode_gltf`. On success it stores the source bytes, the `GltfScene`, and a `Used` note per mesh, material, and image. It walks the JSON for `skins`, `animations`, `cameras`, `meshes[].primitives[].targets`, and any `extensionsRequired` entry outside `SUPPORTED_EXTENSIONS`. Each of those becomes `LoadedUnsupported` and the raw JSON is stored in `kept`. `load_gltf` is not changed. A fixture that `load_gltf` already accepts (`crates/load/fixtures/cube.gltf`) imports with zero `LoadedUnsupported` notes.

Test: `cargo test -p genos-load --test import`. A hand-written glTF in the test has a skin and a morph target. `import_bytes` returns the triangles and two `LoadedUnsupported` notes. `load_gltf` on a file that requires `KHR_texture_transform` still returns `LoadError`.

Commit: `load: import document keeps glTF features the draw does not use`.

### Step 2: Copy the FBX and OBJ scene parsers

Files: copy `crates/load/src/fbx.rs`, `obj_scene.rs`, `crates/load/tests/models.rs`, and `crates/load/examples/model_info.rs` from `6ec390c`. Wire them in `lib.rs` the way that commit does. Extend `import_bytes` so FBX and OBJ+MTL fill `scene` and add `LoadedUnsupported` notes for the skips named in those file headers (the note text can be a fixed list until step 4 stores the raw nodes).

Do not edit the parser logic in the copy. `load_mesh` stays the strict OBJ path.

Test: `cargo test -p genos-load --test models --test import`. The tests already on that branch (`obj_scene_groups_by_material_and_reads_mtl`, `binary_fbx_7400_and_7500`, `ascii_fbx_matches_binary`, `model_file_dispatch`) pass unchanged.

Commit: `load: take the FBX and OBJ scene parsers from mesh-loading`.

### Step 3: SHA-256 object cache

Files: `crates/load/src/cache.rs`, `crates/load/tests/import.rs`.

`store_import(dir, doc)` writes `<dir>/objects/<sha256>` by temp file and rename. A second store of the same bytes does not change the file. A one-byte change makes a second object. The bake cache directory and `content_key` stay.

Test: `cargo test -p genos-load --test import cache`.

Commit: `load: content-address source bytes beside the bake cache`.

### Step 4: Keep accessors and encoded images

Files: `crates/load/src/gltf.rs` (a new function that copies unused accessors; `decode_gltf` stays), `import.rs`, `tests/import.rs`.

Unused attribute streams (`TEXCOORD_n` for n > 0, `COLOR_0`, `JOINTS_0`, `WEIGHTS_0`, morph deltas) are copied into `kept` as f32 arrays. Each image's encoded bytes are stored next to the decoded `Image`. PNG and JPEG tests in `load_assets.rs` stay green.

Test: a glTF with `TEXCOORD_1` and a PNG image. The note list contains `TEXCOORD_1`. The encoded PNG bytes in the document equal the bytes in the buffer view.

Commit: `load: keep unused glTF accessors and encoded images`.

### Step 5: Container magics for audio and video

Files: `crates/load/src/container.rs`, `tests/import.rs`.

Identify RIFF/WAVE (already decoded), Ogg, FLAC (`fLaC`), an MP3 frame sync, MP4 `ftyp`, and EBML. Store the bytes and one note. WAV still returns `Pcm` through `load_pcm`. No new playback.

Test: one byte string per magic. WAV notes `used`. The others note `loaded, unsupported`. A truncated RIFF is still `LoadError::Truncated`.

Commit: `load: identify audio and video containers without decoding them`.

### Step 6: USDA meshes

Files: `crates/load/src/usda.rs`, `tests/import.rs`.

Parse a USDA `def Mesh` with `points`, `faceVertexCounts`, and `faceVertexIndices`, and a `def Material` with a diffuse color, into `GltfScene`. Any other statement is stored as text with `LoadedUnsupported`. A `usda` crate magic that is not `#usda` is the byte-keep path (USDC).

Test: a 2-triangle USDA string imports as 2 triangles and one material. A following `def Camera` is one note.

Commit: `load: USDA meshes enter the import document`.

### Step 7: Cook crate that calls the bake

Files: `crates/cook/Cargo.toml`, `crates/cook/src/lib.rs`, `crates/cook/src/bin/genos-cook.rs`, `crates/cook/tests/cook.rs`. `[lints] workspace = true`. Add the member to the workspace `Cargo.toml`.

`cook(doc)` copies the scene triangles into a cook record and calls `genos_bake::load_or_bake` per mesh. The record stores the bake key. It does not build an SDF of its own.

Test: `cargo test -p genos-cook --test cook`. The cube fixture's triangle count matches, and the bake key equals `BakeMesh::from_gltf(...).content_key` for the same settings.

Commit: `cook: record triangles and the existing bake key`.

### Step 8: Meshlets

Files: `crates/cook/third_party/meshoptimizer` (vendored, cook binary only), `crates/cook/src/meshlets.rs`, `tests/cook.rs`.

Run vertex-cache optimization and `meshopt_buildMeshlets` at 64 vertices and 64 triangles. Every input triangle is in one meshlet. The meshlet bounds contain its positions. The import document's source bytes are unchanged.

Test: `cargo test -p genos-cook --test cook meshlets`.

Commit: `cook: meshoptimizer meshlets, renderer unchanged`.

### Step 9: Cluster LOD

Files: `crates/cook/src/lod.rs`, `tests/cook.rs`.

Build one coarser level by simplifying cluster groups with a locked boundary, and store the parent error and the sphere from the Nanite hierarchy. LOD 0 stays bit-identical to step 8. A two-triangle mesh may have a single level. That is a valid result, and the test allows it.

Test: a dense grid (built in the test, not a shipped asset) has fewer triangles at LOD 1, a finite error, and the same boundary vertices.

Commit: `cook: one cluster LOD with a locked boundary`.

### Step 10: RGBA8 mips

Files: `crates/cook/src/mips.rs`, `tests/cook.rs`.

Box-filter mip chain from the decoded image. A constant image is constant at every level. A 2×2 image has one mip of one texel. Encoded source bytes stay in the import document. Block compression is not in this commit.

Test: `cargo test -p genos-cook --test cook mips`.

Commit: `cook: RGBA8 mip chain beside the source image`.

### Step 11: KTX2 writer for BC7 and ASTC blocks

Files: `crates/cook/src/ktx2.rs`, `tests/cook.rs`.

Write a KTX2 file from a block buffer the test supplies. The desktop record names `VK_FORMAT_BC7_SRGB_BLOCK` for colour and `VK_FORMAT_BC7_UNORM_BLOCK` for data. The other record names ASTC 4×4 LDR. This commit does not vendor a transcoder and does not link Basis into any crate. The RGBA8 mips from step 10 stay in the document so a device without the format can upload them. The runtime upload is still RGBA8.

Test: `cargo test -p genos-cook --test cook ktx2`. A 4×4 constant image's one-block buffer writes a KTX2 whose header names the format and whose level length is 16 bytes.

Commit: `cook: KTX2 writer for BC7 and ASTC blocks`.

### Step 12: Basis transcode in the cook binary

Files: `crates/cook/third_party/basis_universal` (vendored, cook binary only), `crates/cook/src/basis.rs`, `tests/cook.rs`.

Transcode one RGBA8 mip to UASTC and then to BC7 and to ASTC 4×4, and write both through the step 11 writer. `genos-render` does not link this code. A colour image uses the sRGB format. A normal or a data image uses the unorm format.

Test: `cargo test -p genos-cook --test cook basis`. A constant 4×4 colour transcodes to one BC7 block and one ASTC block, and both KTX2 headers name the formats from step 11.

Commit: `cook: Basis transcode to BC7 and ASTC, cook binary only`.

### Step 13: Package manifest

Files: `crates/cook/src/package.rs`, `tests/cook.rs`.

A manifest lists the SHA-256 of each imported object and the cook record for one platform. Two cooks of the same bytes write the same manifest. One changed image byte changes one line.

Test: `cargo test -p genos-cook --test cook package`.

Commit: `cook: package manifest of content hashes`.

### Step 14: CPU page table

Files: `crates/cook/src/pages.rs`, `tests/cook.rs`.

A page table over the mip chain. A request marks a page missing. A fill marks it resident. Eviction drops the least recently requested page when the byte budget would overflow. No Vulkan.

Test: a budget of two pages, three fills, the first page is the one evicted.

Commit: `cook: virtual-texture page table on the CPU`.

### Step 15: Cluster residency

Files: `crates/cook/src/residency.rs`, `tests/cook.rs`.

The same table over meshlet pages. The resident set for a camera is the clusters whose parent error projects above one pixel and whose sphere meets the frustum. A still camera returns the same set and requests no upload. This is the cheap check. No GPU.

Test: a cluster behind the camera is absent. A cluster in front at LOD 0 is present. A second call with the same camera requests nothing.

Commit: `cook: CPU cluster residency from the LOD error`.

### Step 16: Upload a cooked page into the shape pool

Files: `crates/render/src/pool.rs`. The test is `#[cfg(test)]` in that file. It does not open a device.

The frame does not read back. The upload runs when step 15 requested a page. A still set does not write the buffer.

Test: two identical meshlet payloads share one block, as the pool already does for shapes. A changed payload bumps the generation.

Commit: `render: stream a cooked meshlet page into the shape pool`.

GPU proof, owner: `scripts/assets-stream.rhai` walks the camera in `scenes/room.rhai` with a cooked model. The script captures albedo at the start and after the walk. It checks that the second capture is non-black where the model sits. It logs that `GPU_MS` was printed and does not assert a frame time.

### Step 17: Hot reload

Files: `crates/load/src/cache.rs`. The draw reads the generation step 16 bumps. This commit does not add a Vulkan call.

`reload_key(path)` recomputes the SHA-256. When it differs, it reimports and recooks. The draw swaps the pool block from step 16. The scene key gains that generation, as PR #5 already hashes `shape_gen` and `image_gen`.

Test: `cargo test -p genos-load --test import reload` writes a file, caches it, overwrites one byte, and sees a new key. The old object remains in the cache.

GPU proof, owner: `scripts/assets-reload.rhai` captures, the owner replaces the glTF between two `wait` calls (the script documents the pause), then captures again. The report shows both images.

Commit: `load: recook when the source hash changes`.

### Step 18: Voronoi chunks and the bond graph

Files: `crates/cook/src/fracture.rs`, `tests/cook.rs`, `examples/stress/scenes/fracture.rhai`, `examples/stress/scripts/fracture.rhai`.

Given seeds inside the mesh bounds, cut the triangles into one mesh per Voronoi cell. Write a bond for every pair of chunks that share a boundary edge longer than a tolerance, with the rest area and a strength of 1. Store that strength as the bond health in the `.gfract` version 1 payload from the pre-fractured section. Each chunk is a mesh in the cook record and gets its own bake key through `load_or_bake`. Those bake keys stay on the render subset. Do not call `genos_physics::step`. Do not raise `MAX_MESH_PIECES`. The chunk collision and the bond graph are the sim-subset bytes step 19 hashes.

Test: a unit cube and 8 seeds at the octant centers produce 8 chunks. The bond graph connects face-adjacent octants and does not connect opposite octants. The sum of the chunk volumes is the cube volume within 1e-3. The source mesh is still in the import document.

GPU proof, owner: `scenes/fracture.rhai` places the chunks in `room.rhai`'s hall. `scripts/fracture.rhai` captures albedo and depth. The chunks are separate objects. Lighting uses their SDFs because each chunk was baked.

Commit: `cook: pre-fractured Voronoi chunks and a bond graph`.

### Step 19: Load profiles and subset hashes

Files: `crates/cook/src/profile.rs`, `tests/cook.rs`.

`AssetId` is the source SHA-256 from step 3. `subset_hash(doc, Sim)` hashes the canonical sim bytes (version `1`, little-endian bounds, collision, and the `.gfract` payload from step 18 when the cook has chunks). `subset_hash(doc, Render)` hashes meshlets, the platform block buffer, and the bake key. `missing_subsets(id, profile)` lists the names whose hash is absent from a local map the test fills. `LoadProfile::Sim` reports a texture byte length of 0. Two cooks of the same source share the sim hash. A BC7 record and an ASTC record share the sim hash and differ in the render hash.

Test: `cargo test -p genos-cook --test cook profiles`. The cube fixture's sim hash is stable across two calls. After the test inserts the sim hash into the local map, `missing_subsets` for `Sim` is empty and for `Render` still names the render subset.

Commit: `cook: sim and render subsets share one source hash`.

### Step 20: Relevancy gates residency

Files: `crates/cook/src/residency.rs`, `tests/cook.rs`.

`set_relevant(asset_ids)` is the interest projection the session passes in. The page request list is the step 15 camera order, restricted to those ids. A page outside the set is omitted while the camera can see its bounds. An id that leaves the set stays for `relevancy_hold` frames and then is an eviction candidate. The function stores no `NetId` and opens no socket. A `Sim` call returns the sim-subset request and an empty page list.

Test: `cargo test -p genos-cook --test cook relevancy`. Two assets in front of the camera, one id in the set. Only that asset's pages are requested. After the hold expires, a second call drops the id that left. The same camera and the same set request nothing on the next call.

Commit: `cook: residency follows the relevancy set`.

### Step 21: Placeholder while the render subset is missing

Files: `crates/cook/src/profile.rs`, `tests/cook.rs`.

`visible_lod(asset)` returns `Placeholder` when the sim subset is present and the render subset is absent, and `Meshlets` when the render hash is local. The placeholder draw is the collision mesh. With neither subset local, the draw is the bounds. No device is opened and nothing is read back.

Test: `cargo test -p genos-cook --test cook placeholder`.

GPU proof, owner: `debug/assets/scripts/relevancy.rhai` starts with an empty render subset, captures `placeholder`, marks the render hash resident, and captures `residency`. The first image is the collision shape. The second shows pages. The script records both. An agent does not run it.

Commit: `cook: a missing render subset stays a placeholder`.

### Step 22: Assets debug binary

Files: `debug/assets/Cargo.toml`, `debug/assets/src/main.rs`, `debug/assets/src/lib.rs`, `debug/assets/scripts/relevancy.rhai`, `crates/debug/src/commands.rs`. Add the member to the workspace `Cargo.toml`. Document the binary in `docs/systems/debugging.md` in this commit.

The commands, knobs, views, and failure probes are the debug-binary section. `--headless` loads `LoadProfile::Sim` and writes metrics JSON. It does not open a window. The windowed path depends on `genos-window`, `genos-render`, and `genos-ui`. The headless path depends on `genos-load`, `genos-cook`, `genos-debug`, and `genos-scene`.

Test: `cargo test -p genos-assets-view`. `metrics_for(LoadProfile::Sim, cube fixture)` reports one asset, texture bytes 0, and the same sim hash on a second call. A relevant page marked evicted sets `evicted_relevant`. A truncated byte source sets `error` to truncated. A skin note from step 1 is `LoadedUnsupported`.

GPU proof, owner:

```sh
genos-assets-view --scene examples/stress/scenes/room.rhai \
  --script debug/assets/scripts/relevancy.rhai \
  --report target/debug-reports/assets-relevancy
```

Commit: `debug: assets binary reports hashes, residency, and relevancy`.

## Later runtime work this plan does not do

These stay `loaded, unsupported` until a runtime commit moves them. They are listed so a cook commit does not pretend to finish them.

- Skinning and morph playback. The text `Animation` sampler stays the playback that exists.
- A second UV set on the raster. The G-buffer UV discard has to change first, and that change is in the rendering plan.
- Decoding Opus, MP3, FLAC, or video into the mixer.
- A USDC crate reader.
- Drawing the bond graph as a break. That is the physics plan, reading this cook record.
- Sending subset bytes on a socket. `missing_subsets` is the list. The session in the core-networking plan sends it.

## References

Published sources named above. A departure in this file is a **Deviation.** sentence next to that source.

- Arseny Kapoulkine, [meshoptimizer](https://meshoptimizer.org/).
- Brian Karis, "A Deep Dive into Nanite Virtualized Geometry", SIGGRAPH 2021 Advances in Real-Time Rendering. [Slides](https://www.advances.realtimerendering.com/s2021/Karis_Nanite_SIGGRAPH_Advances_2021_final.pdf).
- Khronos, [KTX 2.0](https://registry.khronos.org/KTX/specs/2.0/ktxspec.v2.html).
- Khronos, [KTX developer guide](https://github.com/KhronosGroup/3D-Formats-Guidelines/blob/main/KTXDeveloperGuide.md) (Basis Universal UASTC).
- Martin Mittring, "Advanced Virtual Texture Topics", SIGGRAPH 2008. [Notes](https://advances.realtimerendering.com/s2008/SIGGRAPH%202008%20-%20Advanced%20virtual%20texture%20topics.pdf).
- Ulrich Haar and Sebastian Aaltonen, "GPU-Driven Rendering Pipelines", SIGGRAPH 2015. [Slides](https://advances.realtimerendering.com/s2015/aaltonenhaar_siggraph2015_combined_final_footer_220dpi.pdf).
- Matthias Müller, Nuttapong Chentanez, and Tae-Yong Kim, "Real Time Dynamic Fracture with Volumetric Approximate Convex Decompositions", ACM SIGGRAPH 2013. [Paper](https://matthias-research.github.io/pages/publications/fractureSG2013.pdf).
- Card texel size stays a bake setting ([S57](../lighting/experiments/S57-no-camera-distance-sizes.md)).
- The session contract is [core-networking/plan.md](../core-networking/plan.md). `make lint-docs` allowlists that path until the file lands.
- Runtime fracture reads the cook record from the [physics plan](../physics/plan.md).
