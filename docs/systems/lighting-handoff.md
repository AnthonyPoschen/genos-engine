# Handoff: local lighting updates

Paused. Do not treat the frame-rate band as done. The next agent continues from the walk, not from prominence or the light tree.

The plan that was being executed is the local-update plan: a nearby change relights only the probes it can affect, on a sparse camera-sized 3D occluder grid, then prominence spacing, then an angle-clustered light tree. Criterion 5 is the gate. Two release 600-frame runs of small `genos-stress`, sun running and boxes moving, must be within 15 percent of each other, and each must be at least 80 percent of one idle-outside run on the same binary. The idle eye must be outside and far enough to see the whole 25 m section. Frames per second is 600 divided by wall-clock seconds after one uncounted warmup. `--frames` prints `frames=600` only.

## Shipped

Commits on `master`, not pushed:

- `ce8c235a` keeps local light updates on the sparse 2 m box.
- `cdd6d91d` records why the hall stays under the idle band.
- `3c0b4a72` and `3e34b577` record that a shared 2×2 sun shadow was slower.

What those commits contain:

- A moved box marks its sun shaft and the probes that can see it. A lamp marks its own reach. A slow sun marks the shaft wedge. A run finishes its passes before the next move starts one. A sun climb gathers outside probes first. A finished shell that differs from the picture by half or more snaps.
- Occluders sit in a camera box. The cell stays 2 m. Bricks are 16 m. An empty brick is one jump. After a miss, the walk crosses the fine cells that repeat the shapes just tested. `crates/render/src/occ_grid.rs` is the CPU walk. `crates/render/shaders/scene_rays.glsl` is the picture walk. ADR 0011 records the grid. ADR 0008 is unchanged.
- Prominence functions exist. The live GPU path, when `GENOS_TIER_SPACING` is unset, picks one spacing for the whole window from the highest score. It is not a 1 m, 2 m, 4 m, and 8 m lattice per surface.
- The CPU shade can cluster lamps. The fragment shader still shadow-tests the lamp list. The debug view is off unless asked, and it does not draw spacing, age, or the skip reason.

## Measured

Release, small stress, 1280×720, 600 frames, on the shipped shader:

- Moving hall: 319 fps and 328 fps. Within 15 percent of each other.
- Idle, eye `12.5 6 48`, yaw 0, pitch -0.25, sun frozen, boxes still, dynamic 0: 583 fps.
- Moving is about 55 percent of idle. The bar is 80 percent, about 466 fps.
- Hall raster was about 2.7 ms.

At 2560×1440 on an RTX 4070 (`9ae7541`): moving hall 102 to 104 fps, idle 246 to 251 fps, about 41 percent. Hall raster plus near is 9.1 to 9.4 ms. The moving hall is raster bound.

That idle eye frames the 25 m south face at a 60 degree view. A closer eye cuts the sides off. Do not use a sky-heavy eye, and do not use a closer eye that cannot see the whole section.

## Tried and rejected

Do not repeat these. They either exhaust the NVIDIA compiler or make the hall slower.

- An incremental 3D DDA, a nested short-ray loop, and a cached brick lookup. The driver compiler climbs until the process is killed.
- A walk that does not test shapes. A miss then marches the whole ray, and the raster gets slower.
- One shadow ray per pixel. That drops the other lamps, and the raster was still about 2.1 ms.
- One sun ray shared by a 2×2 quad. About 8.8 ms when a triangle edge threw the ray, and about 3.6 ms with that throw clamped.
- An empty-cell chebyshev jump in the shader, on top of the shipped safe-box jump. A later 600-frame pair was about 250 and 247 fps moving, and about 385 fps idle. Slower than the shipped shader. It was reverted. The CPU walk still has that jump. The shader steps one cell when the slot's high bit is set.

- An any-hit walk: `occ_walk` and the shape list stop at the first blocker (a `first` flag), and `scene_ray_any` tests the planes first. It passed lavapipe, but on the RTX 4070 the driver compiler took the stress process to 23 GB resident before it was killed. It was reverted. `any` is a GLSL builtin, so do not name a function or variable that.

Forcing early fragment tests, and turning depth writes off on the shade pass, left the hall raster at about 2.7 ms. The depth prepass is already vertex-only. That change was reverted.

Sun shadow rays from the floor at day fraction 0.12 average about 2.7 steps on the CPU walk, max 4. The sun is high during the 600-frame window (the default start is day fraction 0.12, and 600 frames at 1/60 s only advance a short way through a 120 s day). The remaining cost is a few fat steps per shadow ray, times the lamps that reach the pixel, on a full hall of pixels.

## Left

1. Cheapen `occ_walk` in `scene_rays.glsl` until the two moving runs clear 80 percent of the idle run. Keep the lib tests green: empty jump, both sides of a lone box, room wall, slab scroll. No fixed trip count above 64. Loops in `scene_rays.glsl` stay `[[dont_unroll]]` unless the bound is under 3. Do not kill a `genos-stress` already bound to MCP port 8765. Bind another port with `GENOS_MCP_PORT`.
2. Only after that band: per-surface prominence spacing (1, 2, 4, 8 m), screen-first budget, keep a finer probe inside the notice band, and a debug view that shows spacing, age, and skip reason without allocating. The flag may be requested. Do not extend the MCP schema.
3. Then the light tree on the picture path: one chosen node is one shadow ray. Far lamps that subtend a small angle stay one light. The CPU tree is in `probe_tier.rs`. The fragment shader does not read it.

## Where this stopped

The working tree matched `3e34b577` aside from this note. No push. Restart `genos-stress` to load the committed binary. Lib tests last run: `cargo test -p genos-render --offline --lib` (58 passed) and `cargo test -p genos-render --offline --test shader_loops` (2 passed), before this note.
