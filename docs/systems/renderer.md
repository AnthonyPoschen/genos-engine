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
- Each light build reads its own copy of the scene. The picture reads the scene of the frame it draws (one buffer per frame in flight): its direct light, shadows, inside tests and near-field rays use the solids and lamps where it rasterizes them, while the bounce comes from the last finished build. A box that turned since that build never tests its own faces as inside its old pose (which left them black until the next build landed).

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

The raster draws the opaque shapes' depth first, with no shading, then shades with an equal-or-nearer depth test, so each pixel shades one opaque surface however many faces cover it. Lighting and raster are two GPU stages in one submit. The CPU does not wait between them. A normal draw does not stall on the GPU fence. There are two frames in flight.

The picture modes are `off`, FXAA, and SSAA. The draw does not run MSAA, SMAA, or TAA. The filter does not change the light field. The filter does not change the overlay.

The swapchain uses mailbox when the surface lists mailbox. The next choice is immediate mode. The last choice is FIFO. Each in-flight frame has its own dynamic patch. The shape pool is one shared buffer.

A write into a live pool block waits until both frames are done. A new block at the end of the pool does not wait. An unused block can be reused after those frames finish.

The profile is not part of `Renderer::draw`. There is no per-shader GPU time. The timestamp span does not include a CPU wait.

`GENOS_GPU_TIMES=1` prints a `GPU_MS` line every 120 frames (`GENOS_GPU_TIMES=N`: every N). It gives the mean GPU time of each frame pass: the scene raster (which shades the near field per pixel), the antialias resolve, the overlay, and the copies. A frame copies its picture to the swapchain; only a readback draw also copies it into host memory, and `read_picture` copies the last picture when called (`set_keep_pictures` copies every frame, for `read_earlier_frame`). It also gives each light build's passes: the tier copy, the world direct and bounce passes, and the tier rays, with builds per frame and rays per build. After a `|` come wall-clock builds per second (all builds, and those that ran tier work), the most tier rays one build traced, the ray budget of the last batch (`settle` while settling), `seen_settled`, the bricks in view with no work left out of all bricks in view, and frames per second. `GENOS_NEAR_RAYS=N` sets the near-field rays per pixel (0 turns the near field off), so the raster difference is the near field's share. `GENOS_TIER_MS` is the least GPU time a build may take (1.5 ms); a build may take as long as the running time between pictures, since it runs on its own queue and its measured time holds the frames it overlapped. The first line is preceded by `GPU_MEM`: how many of the buffers the CPU writes went to device-local memory (resizable BAR or the BAR window) rather than host memory the GPU reads over PCIe. `GENOS_PASS_TIMES=1` prints every light build. The tier learns what a ray costs from its timed builds: a faster build is taken at once and a slower one moves the cost only a little, since a build that shares the GPU with the picture only ever looks slower; only a build that used most of its budget can raise the cost, since a small build cannot fill the GPU. The budget holds in every scene: a pass bigger than what is left of it runs the share of the brick's probes that fits, and the rest in the next builds, so no build passes its budget by more than one probe's rays. A new brick shows once all its probes have their first light.

The picture does not read the tier's probes straight from the light field. Each frame, before the raster, a pass moves a view copy of each probe's coarsest cube toward the latest built one by `1 - exp(-dt / τ)`, so a build landing shows as a smooth fade, whatever the build rate. τ is `GENOS_TIER_VIEW_MS` (33 ms; 0 shows each build as it lands). The shown light is never dropped: a probe that dies (a moved solid now covers it) keeps its light with its sample count negated, so no reader takes it while the view holds it, and when the solid leaves it blends from that light to the new. A probe that moves a little (out of a solid's way) blends too; only a probe that never had light, or a slot that now holds another brick, snaps. A settling draw and a readback show the built field. A change pass traces the same rays for a probe every time, so the passes of one change only add samples, and they never jitter the picture; refine passes turn the rays every build. The built field is the working copy. The view keeps each probe's last whole update and blends to it; a brick's light becomes a whole update when its last change pass lands or its refining reaches its full samples, and only then does the view take it, in one blend; no value part way through reaches the screen. Where the camera stands never decides whether the view takes a whole update. A change that reaches a brick in the middle of its passes starts them over; once six change passes have landed since the picture last took the brick, that is a whole update too, so a brick whose light changes all the time still updates, every six passes. A probe that never showed light takes its first light at once. The tier ranks its work by what the camera sees. Each frame 32 x 18 rays across the view (turned by a sixteenth of a cell each frame) meet the nearest surface box, and each hit counts toward the bricks of the probes the picture reads there; the counts fade by a quarter a frame. Faces hidden behind others and everything off screen count nothing. A brick's priority is its share of the screen, times its nearness, times its staleness. The share is the rays that met it (about the pixels whose light it gives) plus `feed` (0.5) of what its most-seen neighbour got, since its light reaches them in one bounce; the rays reach `margin` (a quarter) past the screen's edges, where a hit counts `edge` (a quarter), so a brick the camera is about to turn to weighs something. Nearness is `near / (near + distance)` (`near` 4 m): a spinning box beside the camera matters more than one far off. Staleness is `1 + age / stale`, the age since the brick last finished an update (`stale` a quarter second), so under load the bricks the picture uses are refreshed in turn; a brick the picture does not use weighs nothing and gains nothing from waiting. A brick with a change under way also takes the share of the view's light the light that changed it brings: one view ray in sixteen (in turn) measures, for each light, its unshadowed light at the hit times the facing, when no shadow-casting box stands between, plus 0.3 of it for the bounce, faded like the seen counts; a change keeps `0.1 + 0.9 x` its light's share against the light that brings the most, so the light the view shows most lands first and a dim or hidden light lags until staleness brings its bricks round (a moved box or a new brick counts 1). An outside brick whose sun and sky light on an upward face (each directional light's height times its brightest channel, the sky as a sun straight up) has moved more than a fifth from what it last settled under takes its change passes: the shafts only mark where shadows moved, so a sun that set or a sky that faded left the open yard at day. Its priority gains eight times that relative change, so once in view the bricks holding day at night go before those that only follow a moving lamp; inside the building that term is zero. `TierStats` keeps the running settle times (change reaching a brick to its whole update) for the lights that bring at least half the top light's share, for dimmer ones, and for the outside drift, and the top light's share; the stress app's lighting text shows them. The weights are `TierWeights`, live through `Renderer::set_tier_weights`; `GENOS_TIER_WEIGHTS=near=4,feed=0.5,margin=0.25,edge=0.25,stale=0.25` sets them at start, and `GENOS_GPU_TIMES` and the bench print them. The tier spends its budget in that order: bricks the picture uses that wait for their first light take that cheap pass first (until then the picture reads the world probes there), then each brick takes its next pass, those with a change under way (their light is wrong) before those refining (right but noisy), each highest priority first, and inside a step of two in priority the brick furthest along first, so bricks finish rather than all creeping forward together. A pass that does not fit runs the share of its probes that does and the batch ends there, so the critical set is what the budget holds and the rest waits. One pass of many bricks keeps the GPU full; a round of a few bricks leaves most of it idle, its rays look dear and the budget shrinks. What is left deepens the bricks picked, in the same order: each takes every change pass it has left, as rounds of one pass each in the same build, before the next takes more. What the camera sees converges, bounces and refining included, while lights keep moving elsewhere. Bricks the picture does not use keep a quarter of the budget while any of them wait, so their light, which feeds what the picture shows, converges wherever the camera is. A brick whose update is done but whose light still moved by more than the notice band (and by more than its probes' noise) refines again, and its neighbours, whose bounce it feeds, take half that move toward their own refresh. With room for everything (a settling draw) every brick runs all its passes together. When solids move, only the boxes that changed are placed again: the bricks within the tier's reach of their old and new places get their probes again, the rest of the window keeps its probes. A brick whose probes came alive, died or moved keeps its light and takes the change passes, like a light change; a probe that came alive has no light, and readers skip it until its first pass. Only the bricks the move can reach take change passes: those within a metre plus two of the solid's sizes of its old or new place (its bounce, or the light it hid), and those a light reaches through its old or new place (its shadow). They join the same weighted order as a light change. A change of surface colours relights every brick. Each `BENCH` drop and rise line gives `view_ms`, when no brick on screen still had a change pass to take (they land one build later), `critical`, the most bricks on screen one batch took, and `batch_us`, the slowest CPU pick of a batch.

`genos-camera` with `GENOS_BENCH=<seconds>` runs unattended: a still phase, a lamp orbit, then the lamp drops beside the boxes and rises back, each until the tier is settled, one `BENCH` line per phase, then it exits. `GENOS_BENCH_SHOTS=<dir>` also saves the picture at fixed times (0 to 4000 ms) into each drop and rise, as drawn at the normal budget, mid-change (`Renderer::set_live_readback`); the captures stall the frame they are taken in, so time a separate run. A last flicker phase orbits the lamp a fixed step per frame (a tenth of a degree, the orbit's pace at 600 frames a second) and reads back every frame (`GENOS_BENCH_FLICKER=<frames>`, 240 by default after a 60-frame lead-in, 0 skips it). A pixel flickers when its 8-bit luminance turns back from one frame to the next (up then down, or down then up), by the smaller of the two steps; the moving lamp and a moving shadow edge change a pixel one way at a time, rounding included, so they read zero. Each of 4 x 4 screen tiles takes the mean of its pixels, so a small flickering patch counts. Its line gives the mean and worst tile and all 16 tiles, row by row; its frame times include the readbacks. Every bench readback is live: it shows the light as drawn at the normal budget, mid-change, never a settled field. Then the red box (or the first solid) is dragged: `Drag` moves it 1 m/s along x for 1.5 s and gives its frame times (spikes in `ms_p99` and `ms_max`), `Rest` holds it and gives its settle times like a drop, and `DragFlicker` moves it back a fixed step per frame (the drag speed at 600 frames a second) and reads every frame back for the flicker measure. Last, `Switch` moves every solid 0.37 m along x and z at once, off the probe grid, so every brick near a face places its probes again, as when another room loads, and gives its settle times. `Rest` and `Switch` read the picture back every 25 ms (every fourth pixel along x and y, as luminance) and compare each with the last, settled one: the pixels the change touched are those more than 6 of 255 codes off it in any picture (`touched`, a share of the screen), `conv95_ms` is when 95% of them came within 6 codes for good, `start_within` the share of them within in the first picture, and `start_err` the first picture's mean error over the screen. A bench draws without the panel and the graph.

## Decisions

- [ADR 0003](../adr/0003-one-vulkan-renderer-moltenvk-on-macos.md) keeps one Vulkan renderer.
- [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md) forbids a renderer framework.
- [Lighting](lighting.md) owns the light pass that this draw calls.
