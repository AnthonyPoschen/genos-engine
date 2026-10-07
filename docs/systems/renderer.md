# Renderer

The renderer owns the Vulkan draw. It turns a world and a camera into one presented frame. Lighting runs inside that draw.

## Goal

A game can submit fixed shapes today, and can submit meshes, particles, and shader draws as those features grow. Objects outside the view are not rasterized. Hidden objects are not rasterized. Both still feed the light field when `affects_light` is true.

## Intent

One Vulkan path serves Linux, Windows, and macOS. macOS uses MoltenVK. The renderer loads `libvulkan.so.1` directly. It does not use `ash`, `wgpu`, or `vulkano`.

`Renderer::draw` uploads new shape blocks and lamp parameters, then submits GPU lighting and raster. A normal submit does not wait for that GPU work. The caller does not build the field and does not pair lamps with objects. The engine keeps the field while the lights and the occluders stay the same. The engine keeps each shape block while a draw still uses it.

A level keeps one shape pool on the GPU. Identical shapes share one block and the same pointers. The draw uploads a new block once. A later frame reuses that block. A shape with no users returns to the free list. Particles and the overlay stay in a separate patch.

Particle image pixels stay in one resident buffer. The draw uploads those pixels when the image is first admitted or when the pixels change. A later frame can move particles, age them, or select another frame of the same image. That frame does not upload the pixels again. The per-frame write is the positions, the ages, or the frame selection.

Shape blocks and particle images share one byte budget. Required in-view geometry outranks unused blocks and optional particle images. A new admit that would pass the budget drops unused blocks first. It then shrinks or drops the particle image. Geometry that fits on its own stays resident.

A profiled draw writes Vulkan timestamp queries around the GPU draw. The GPU time is that timestamp span. A fence wait is not the GPU time. `Renderer::draw` does not write timestamp queries. A stage with no GPU work has GPU time 0.

`World::from_scene` makes one object for the floor, one for each wall, and one for each solid. Extra objects are meshes, particles, or shaders.

`DrawKind` is the set of draws:

- `Fixed` is the floor, a wall, or a solid from the scene.
- `Mesh` is a triangle list plus a column-major pose. Replace the pose each frame to move the mesh.
- `Particles` are cards. A card can face the camera or yaw by an angle. `lit` multiplies the base albedo by world light. `emission` adds that card to the one ground fire term. An image is a loaded texture or one texture-map frame chosen from the particle age. Density above zero is still one fog card.
- `Shader` is `ShaderSpace::Mesh` or `ShaderSpace::Screen`. A mesh-space shader can carry a `Displacement` height map. The map builds a grid. A screen-space shader is recorded and is not drawn over the scene yet.

`hidden` skips raster. `in_view` skips raster when the bounds miss the camera frustum. `affects_light` set to false also removes that object from the cascade scene. The floor stays in the cascade scene because the march runs on it.

Each box face and each cylinder side is lit from a point just outside that face. One sample at the center of a wall lights the back face. Do not return to that sample.

The GPU cascade places probes on the floor. Spacing is finer near the player than far away. Each probe casts 64 rays, and every range of a ray keeps that ray's direction. The fragment shader shades each pixel. A cast shadow on the floor follows the lamp ray through the occluder.

## Code

The crate is `crates/render`, package `genos-render`.

- `Renderer` is in `src/gpu.rs`. `draw_profiled`, `poll_gpu_times`, and `finish_gpu_times` record the draw's GPU time.
- `Antialias` is in `src/aa.rs`. The filter commands are in `src/aa_gpu.rs`. The passes are `shaders/fxaa.comp` and `shaders/ssaa.comp`.
- `World`, `Object`, `DrawKind`, and `in_view` are in `src/world.rs`.
- Triangle positions are packed in `src/pack.rs`, and shape blocks are in `src/pool.rs`.
- `FrameMemory` in `src/budget.rs` counts those blocks and the particle image against one byte budget.
- The particle step and the card builder are in `src/particles.rs`.
- GPU lighting is `shaders/light.comp` and `shaders/scene.frag`. The fragment shader samples a particle image when a card has one.
- `Lighting` in `src/lighting.rs` and `src/field.rs` keep the earlier CPU checks. The draw path does not call them.

- The picture clears to the scene's sky through the tone curve (black with no sky), so a pixel that meets nothing shows the light a ray that leaves the scene brings.
- `Renderer::take_light_builds` returns the GPU time of each light build that finished since the last call. `LightBuildTimes::passes_ms` holds the spans copy, world direct, world bounce and tier, in that order. A still scene in a settled tier runs no build and returns none.
- A wall draws its top face. A raised wall (`base` above 0) also draws its underside. A square solid draws turned by its `yaw`.
- `MAX_OBJECTS` is 8192. Lamps, occluders and their grids are packed in the scene buffer tail; see [Lighting](lighting.md).

## Game use

```text
let world = World::from_scene(scene);
renderer.draw(&world, &camera, false)?;
renderer.draw_with_overlay(&world, &camera, &rects, false)?;
```

Each `ScreenRect` is a pixel rectangle. The draw places those rectangles after the world. The overlay does not test depth. The overlay does not write depth.

The draw starts in `off`. That mode is the single-sample picture. `Renderer::set_antialias` selects the next picture. FXAA is a luminance-edge pass on the scene color. That pass runs before the overlay. SSAA draws the scene at twice the presented width and twice the presented height. A tent filter then downsamples that image. Neighbor presented pixels share samples. The overlay is drawn at the presented size after the filter. A switch back to `off` draws the unfiltered image. The filter keeps no history.

Set `object.hidden = true` to hide an object and keep its light. Push a `DrawKind::Mesh` and replace `pose` to animate. Push a `DrawKind::Particles` to draw cards. `color` is the base albedo. Also set `lit`, `emission`, `face_camera`, `angle`, and `image`. Push a `DrawKind::Shader` with a `Displacement` to build a grid from a height map.

The camera steps a seeded emitter on the red solid. A stationary flame stays on that solid and animates in place. Embers and a smoke trail leave that flame. The fire light is at the center of the stationary flame. The draw submits those cards.

Open the renderer with the window display and the window surface. Call `resize` when `WindowEvent::Resized` arrives.

The camera calls `draw_profiled` in detailed profile mode. Basic mode and off mode call `draw_with_overlay`. The camera calls `poll_gpu_times` while a detailed frame runs. The camera calls `finish_gpu_times` before a detailed run exits.

## Limits

There is no skinned clip player. A mesh animates only when the game writes a new pose.

A particle does not occlude. `affects_light` is not the lit flag and is not the emission flag. Emitting particles share one fire term. There is not one lamp per ember.

A screen-space shader does not run a custom program yet. Custom SPIR-V is not loaded.

Lighting and raster are two GPU stages in one submit. The CPU does not wait between them. A normal draw does not stall on the GPU fence. There are two frames in flight.

The picture modes are `off`, FXAA, and SSAA. The draw does not run MSAA, SMAA, or TAA. The filter does not change the light field. The filter does not change the overlay.

The swapchain uses mailbox when the surface lists mailbox. The next choice is immediate mode. The last choice is FIFO. Each in-flight frame has its own dynamic patch. The shape pool is one shared buffer.

A write into a live pool block waits until both frames are done. A new block at the end of the pool does not wait. An unused block can be reused after those frames finish.

The profile is not part of `Renderer::draw`. There is no per-shader GPU time. The timestamp span does not include a CPU wait.

`GENOS_GPU_TIMES=1` prints a `GPU_MS` line every 120 frames (`GENOS_GPU_TIMES=N`: every N). It gives the mean GPU time of each frame pass: the scene raster (which shades the near field per pixel), the antialias resolve, the overlay, and the copies. It also gives each light build's passes: the tier copy, the world direct and bounce passes, and the tier rays, with builds per frame and rays per build. After a `|` come wall-clock builds per second (all builds, and those that ran tier work), the most tier rays one build traced, the ray budget of the last batch (`settle` while settling), and frames per second. `GENOS_NEAR_RAYS=N` sets the near-field rays per pixel (0 turns the near field off), so the raster difference is the near field's share. `GENOS_TIER_MS` is the tier's GPU budget per build (1.5 ms). `GENOS_PASS_TIMES=1` prints every light build. The tier learns what a ray costs from its timed builds: a faster build is taken at once and a slower one moves the cost only a little, since a build that shares the GPU with the picture only ever looks slower.

`genos-camera` with `GENOS_BENCH=<seconds>` runs unattended: a still phase, a lamp orbit, then the lamp drops beside the boxes and rises back, each until the tier is settled, one `BENCH` line per phase, then it exits. `GENOS_BENCH_SHOTS=<dir>` also saves the picture at fixed times (0 to 4000 ms) into each drop and rise, as drawn at the normal budget, mid-change (`Renderer::set_live_readback`); the captures stall the frame they are taken in, so time a separate run.

## Decisions

- [ADR 0003](../adr/0003-one-vulkan-renderer-moltenvk-on-macos.md) keeps one Vulkan renderer.
- [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md) forbids a renderer framework.
- [Lighting](lighting.md) owns the light pass that this draw calls.
