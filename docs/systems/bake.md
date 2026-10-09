# Bake

## Goal

Build the import-time data GI v2 traces against, once per mesh: a sparse signed distance field (the software tracer) and surface cache cards (albedo, normal, depth and emission seen from each side). See [GI v2 design](gi-v2-design.md), "Scene representation" and "Surface cache".

## Intent

- A mesh's bake depends only on its content (triangles, materials, the texels they use) and the bake settings. It never depends on the camera or on where the mesh is placed. Instances share one bake.
- When the bake runs is a setting, `BakeMode`: `Prebuilt` (built ahead of time by `genos-bake`; our editor's default) or `OnLoad` (built at first load and cached on disk). `GENOS_BAKE=onload` switches at run time. In `Prebuilt` mode a missing bake is an error that names the file to build; it is not silently rebuilt.
- Results are cached by a 64-bit FNV-1a content key (`<key>.gbake` under `target/bake-cache`, or `GENOS_BAKE_CACHE`). Files are written to a temporary name and renamed, and a file of another format version or a cut file is ignored.
- Everything is written in this repository (ADR 0007): the BVH in `genos-mesh`, the decoders in `genos-load`.

## Code

The crate is `crates/bake`, package `genos-bake`.

- `BakeMesh` (`src/mesh.rs`): object-space `MeshTriangle`s, materials and images. `BakeMesh::from_gltf(scene, mesh)` takes a glTF mesh; `BakeMesh::plain` takes triangles with flat albedo (analytic shapes). `surface` gives the diffuse albedo (base colour x (1 - metallic), linear) and emission (emissive texture x factor x strength) at a UV.
- `sdf.rs`: voxel = largest side / 64, clamped to 2-25 cm and to at most 512 voxels per side. 8^3 bricks are stored only where the surface is within the band (4 voxels), as 8-bit distances, plus a coarse grid with one value per brick. A closed mesh is signed by ray parity (majority of three skew rays); an open mesh is unsigned and flagged `thin` (two-sided), so thin sheets cannot leak.
- `cards.rs`: six axis-aligned cards over the bounds at a fixed texel size (`card_texel`, default 6 cm, at most 512 texels per edge). Each texel takes four rays into the mesh: albedo, normal and depth average over the hits, coverage is the hit share, and emission is the mean over all four, so a card keeps the emitted power of texture-driven emission.
- `format.rs`: the `.gbake` file.
- `src/bin/genos-bake.rs`: `genos-bake [--out DIR] [--texel M] [--voxels-per-side N] [--force] file.gltf|file.glb ...` bakes every mesh of each file and skips up-to-date ones.

## Game use

Build a `BakeMesh` per mesh and call `load_or_bake(&mesh, &BakeSettings::default())`. Ship the cache directory with the game when it uses `Prebuilt`.

## Limits

- Six cards per mesh. The greedy cover for complex meshes (up to 32 cards) comes with the surface cache (phase 4).
- Analytic shapes bake through their triangles; exact shape SDFs come later.
- No LODs yet. No GPU upload yet: the renderer does not read bakes until phase 2 (SDF) and phase 4 (cards).
- `MeshSdf::distance` is a nearest-voxel lookup for tests and tools, not the GPU sampler.
- Measured on the box (8 cores, release): Khronos DamagedHelmet 0.3 s; Sponza as one 262k-triangle mesh 10 s (it is authored in centimetres, so its voxels hit the 512-per-side cap).

## Decisions

- [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md): no third-party crates for native and renderer work.
- [ADR 0012](../adr/0012-gi-v2.md): GI v2.
