# Frame reuse: implementation plan

The design is [Frame reuse](frame-reuse.md). This plan lists the remaining work as small steps. Each step is one commit. Do them in order. Each step builds on its own.

## Already done on `cursor/frame-output-reuse-6556`

- `bff96c0` The design note.
- `611449f` `genos-ui` reports damage: `DamageTracker` (`crates/ui/src/damage.rs`), `Frame::damage` from `lighting_frame` and `button_panel`, and tests in `crates/ui/tests/damage.rs` (8 passing).
- `cb6150c` Lighting settles in the default surface cache mode. `Lighting::light_rounds` counts the finished light cache rounds since the light last changed, on the CPU, and `lighting_stats()` uses it in place of `light_quiet`. This commit compiled, but the render tests have not been run.

## Rules for every step

- Read `AGENTS.md`, `docs/systems/renderer.md` and `docs/frame-reuse.md` first.
- Never read GPU memory back in the frame loop. Every decision uses CPU state: uploaded bytes, UI paints, and lighting counters.
- `cargo build --workspace` must pass with no warnings, because the workspace denies them. Run `cargo test -p <crate>` for each crate you touched. To build, you need `glslc`, `wayland-scanner`, `libwayland-dev`, `wayland-protocols`, `libxkbcommon-dev`, and Rust 1.85 or newer.
- Don't add Vulkan crates. Follow the existing raw `#[repr(C)]` struct style in `gpu.rs`.
- Update the system record (`docs/systems/renderer.md`) in the same commit when a step changes the game use or a limit.

## Vulkan constants used below

The layouts are: 0 is undefined, 6 is transfer source, 7 is transfer destination, and 1000001002 is present source. The stages are: 0x1000 is transfer, 0x400 is color attachment output, and 0x800 is compute. The access masks are: 0x800 is transfer read, 0x1000 is transfer write, 0x100 is color write, and 0x80 is color read. The image usages are: 0x1 is transfer source, 0x2 is transfer destination, and 0x10 is color attachment.

---

## Step 1: The renderer accepts reported overlay damage

Files: a new `crates/render/src/reuse.rs` (add `mod reuse;` in `lib.rs`), `crates/render/src/gpu.rs`, and `lib.rs` exports.

1. In `reuse.rs`, add these:
   - `pub struct DamageRect { pub x: u32, pub y: u32, pub w: u32, pub h: u32 }`. Derive `Clone, Copy, Debug, PartialEq, Eq`.
   - `pub(crate) type Region = [i32; 4]`, holding x0, y0, x1, y1 in pixels.
   - `pub(crate) fn add_region(list: &mut Vec<Region>, r: Region, extent: (u32, u32))`. It clips `r` to the extent and drops it if empty. It merges `r` with any rectangle within 16 px. If the list then holds more than 8, it replaces the whole list with their bounding box. Match `merge` in `crates/ui/src/damage.rs`; a simpler version is fine here.
   - `pub(crate) fn rects_region(rects: &[ScreenRect]) -> Option<Region>` and `lines_region(lines: &[ScreenLine]) -> Option<Region>`. These give bounds rounded outward, plus 1 px. For lines, add `width / 2` around the ends.
2. In `Renderer`, add the field `reported: Option<Vec<DamageRect>>` and this method:
   `pub fn report_overlay_damage(&mut self, rects: &[DamageRect])`. It stores the rectangles and covers the next draw only.
3. In `set_overlay_lines`, remove the NaN-rectangle trick. Add a field `lines_dirty: bool`, and set it when the lines change.
4. In `draw_with_overlay`, before the overlay vertices are rebuilt, compute `frame_damage: Vec<Region>` like this:
   - If the overlay rectangles equal the last ones and `!lines_dirty`, the damage is empty. Ignore any report.
   - Otherwise, if a report is present and not empty, use its rectangles.
   - Otherwise, use the fallback: the old bounds and the new bounds of the rectangle lists and the line lists.
   - Clear `reported` and `lines_dirty`, then rebuild the vertices as the code does today.
5. Store `frame_damage` on the `Gpu` for later steps, as `self.gpu.reuse.frame_damage`. For now, nothing reads it.
6. Export `DamageRect` from `lib.rs`.
7. Add unit tests at the bottom of `reuse.rs` (`#[cfg(test)]`): merging, clipping, the 8-rectangle cap, and line bounds.

Commit: `render: accept reported overlay damage (report_overlay_damage, fallback diff)`.

## Step 2: The scene key

Files: `gpu.rs`, `lighting_gpu.rs`, `reuse.rs`.

1. In `reuse.rs`, add `pub(crate) fn fast_hash(seed: u64, bytes: &[u8]) -> u64`. Read the bytes as `u64` words: `h = (h ^ w).wrapping_mul(0x9E3779B97F4A7C15).rotate_left(31)`, then the tail bytes, then a final avalanche. It must be fast, because it hashes the particle vertices every frame.
2. In `lighting_gpu.rs`, move the scene hash out of `lighting_passes` into `fn lighting_scene_hash(&self) -> u64`. It covers the same three inputs: `frame_bytes`, `mesh_field_key`, `lighting.draw_key`. Store the hash in `self.lighting.frame_hash` before recording, and have `lighting_passes` read `frame_hash` rather than hash a second time. This step must not change lighting behaviour.
3. Add these counters to `Gpu`: `shape_gen: u64`, incremented in `upload_shapes` whenever it writes anything, and `image_gen: u64`, incremented when `upload_image` runs.
4. In `draw_with_overlay`, after `upload_scene`, compute the key:
   `scene_key = fast_hash` over `lighting_scene_hash()`, the world dynamic vertex bytes (`dynamic[..world_dynamic]`, cast to bytes the same way `upload` does), each draw span's `(shapes, first, count, instance)`, `shape_gen`, `image_gen`, `antialias as u8`, `wireframe`, `background` bits, `extent_w` and `extent_h`, and `lighting_active()`. Store it in `self.gpu.reuse.scene_key`.

Commit: `render: key the scene picture (one hash of what the raster and lighting read)`.

## Step 3: The scene picture image and its capture

Files: `reuse.rs`, `gpu.rs`, `aa_gpu.rs`.

1. In `reuse.rs`, add `pub(crate) struct FrameReuse` with these fields:
   - `enabled`: true unless `GENOS_FRAME_REUSE=0`.
   - `scene: Image`, and `scene_layout: u32`.
   - `cached_key: Option<u64>`.
   - `generation: u64`.
   - `slot_gen: [u64; 2]`.
   - `slot_damage: [Vec<Region>; 2]`.
   - `frame_damage: Vec<Region>` and `scene_key: u64`, from steps 1 and 2.
   - `stats: ReuseStats`.

   Add `pub struct ReuseStats { pub scene_drawn: u64, pub ui_repaired: u64, pub full_restore: u64, pub skipped: u64, pub rects: u64 }`.

   `FrameReuse::default()` uses `Image::empty()`. Because `reuse.rs` needs `Image` from `gpu.rs`, put `FrameReuse` in the `impl` blocks of an `include!("reuse_gpu.rs")` file like `aa_gpu.rs`, or make `Image` `pub(crate)`. Pick one.
2. Add `reuse: FrameReuse` to `Gpu`. In `create_targets`, after the colour images, create `reuse.scene = make_image(self.format, 0x1 | 0x2, 1, w, h)` and set `scene_layout = 0`. Then call `invalidate()`, which sets `cached_key = None`, sets `slot_gen = [0, 0]`, clears `slot_damage`, and increments `generation`. In `destroy_targets`, destroy the image.
3. Overlay order: make the overlay always draw in its own pass after the scene. In `record_and_submit`, pass `false` as the last argument of `raster_scene`. Then call `raster_overlay()` once, after FXAA (and after lighting when FXAA is off), whenever `overlay_count > 0`. The picture must look the same as before.
4. Capture: in `record_and_submit`, after the scene, FXAA and lighting, and before the overlay:
   - `capture = reuse.enabled && (!lighting_active || self.lighting.held >= 1) && !self.lighting.world_busy`. `held >= 1` means this frame ran the lighting's frozen path.
   - If `capture`: barrier the scene image from `scene_layout` to 7. Then `cmd_copy_image` the colour image (6) into the scene image (7) at full extent; reuse `copy_color_to_swapchain` and add a destination argument. Barrier the scene image from 7 to 6 and set `scene_layout = 6`. Set `cached_key = Some(scene_key)`, increment `generation`, and set `slot_gen[slot] = generation`.
   - Otherwise, set `cached_key = None` and `slot_gen[slot] = 0`.
   - Clear `slot_damage[slot]` and increment `stats.scene_drawn`.

Commit: `render: keep the last held scene picture on the GPU; overlay in its own pass`.

## Step 4: Reuse the scene and repair only UI damage

Files: `gpu.rs`, `aa_gpu.rs`.

1. In `draw_with_overlay`, add `frame_damage` to both `slot_damage[0]` and `slot_damage[1]` with `add_region`.
2. Compute `reuse_scene` as true when all of these hold:
   - `reuse.enabled`
   - `cached_key == Some(scene_key)`
   - `!lighting_active() || self.lighting_stats().settled`
   - `!self.lighting.world_busy`

   Pass it to `record_and_submit` as a field: `self.reuse_scene`.
3. In `record_and_submit`, when `reuse_scene` is true, skip `lighting_begin`, `raster_scene`, `lighting_passes` and `resolve_fxaa`. Keep `update_resident_scene` and `update_resident_fields`, the swapchain barriers, the copy, the readback copy, and the frame stamps. Then:
   - If `slot_gen[slot] != generation`: copy the whole scene image (6) into the colour image. Barrier the colour image 6 to 7, copy, then barrier 7 to 6. Call `raster_overlay()` at full extent, set `slot_gen[slot] = generation`, and increment `stats.full_restore`.
   - Else, if `slot_damage[slot]` is not empty: copy each region from the scene image into the colour image, with one `cmd_copy_image` call and N regions. Then call a new `raster_overlay_in(&regions)`. It begins `keep_pass` with the render area set to the bounding box of the regions. For each region it sets `cmd_scissor` to that region and draws the overlay quads; split `draw_overlay_quads` so the scissor is not reset inside it. Increment `stats.ui_repaired`, and add the region count to `stats.rects`.
   - Else, do no colour work.
   - Clear `slot_damage[slot]`.
4. Add a method `fn lighting_note_reused(&mut self)` that increments `lighting.still` and `lighting.held` with saturation. Call it on every reuse frame, so `settle_stats` and `picture_settled` keep counting.
5. The colour image keeps layout 6 at the end of every frame. `color_layout[slot] = 6` stays as it is.

Commit: `render: reuse the held scene picture and redraw only the UI damage`.

## Step 5: Skip the whole frame when nothing changed

Files: `gpu.rs`.

1. In `draw_with_overlay`, after step 4's decision, skip the frame when all of these hold:
   - `reuse_scene`
   - `frame_damage` is empty
   - `!readback`
   - `!self.keep_pictures`
   - the previous call did not recreate the swapchain

   When skipping, call `lighting_note_reused()`, increment `stats.skipped`, set `self.skipped = true`, and return `Ok(None)` before `record_and_submit` and `present`. Do not toggle `flight`, and do not touch the slot damage.
2. Add `pub fn frame_skipped(&self) -> bool` (true when the last draw submitted nothing) and `pub fn reuse_stats(&self) -> ReuseStats`.
3. In `draw_profiled`, if the frame was skipped, return `DrawProfile { cpu, gpu: Some(Duration::ZERO) }`. The renderer record already says a stage with no GPU work has GPU time 0.
4. After `recreate` (resize, or present returning OUT_OF_DATE or SUBOPTIMAL), `create_targets` calls `invalidate()`, which forces the next frame to draw.
5. In `collect_frame_times`, when it prints `GPU_MS`, also print `FRAME_REUSE scene_drawn=.. ui_repaired=.. full_restore=.. skipped=.. rects=..`, then reset the stats. Skipped frames don't run `collect_frame_times`, so also print from `draw_with_overlay` every `every` skipped frames if no `GPU_MS` line has appeared.

Commit: `render: skip acquire, submit and present when the screen would not change`.

## Step 6: Present damage regions

Files: `gpu.rs`.

1. In `Gpu::open`, load `vkEnumerateDeviceExtensionProperties` with `load` (instance-level). Each entry is 260 bytes: a `char[256]` name, then a `u32`. If `VK_KHR_incremental_present` is listed, add it to the device's extension list, which becomes an array of 2 with `ext_count` set to match. Store the result in `incremental_present: bool`.
2. In `present`, when `incremental_present` is true, chain these structs on `Present.next`:
   - `#[repr(C)] struct RectLayer { x: i32, y: i32, w: u32, h: u32, layer: u32 }`
   - `struct PresentRegion { count: u32, rects: *const RectLayer }`
   - `struct PresentRegions { s_type: i32 = 1000084000, next, swapchain_count: 1, regions: *const PresentRegion }`

   Set the rectangles to the frame's screen damage: the whole extent after a scene draw, else `frame_damage`. A count of 0 means "whole image", so never pass 0 for a non-empty frame.
3. Always copy the whole picture into the swapchain image. The regions are only a hint.

Commit: `render: pass present damage through VK_KHR_incremental_present`.

## Step 7: The camera and stress report damage and idle on skipped frames

Files: `examples/camera/src/main.rs`, `examples/stress/src/main.rs`.

1. Camera: keep a `graph_damage: genos_ui::DamageTracker`. Each frame, call `begin(viewport)`. Call `paints(1, ..)` with the graph paints actually put in the overlay (empty when the graph is off or a readback runs), and `segments(2, ..)` with the lines. Then call `finish()`.
2. Join `ui_frame.damage` with the graph's damage, convert to `genos_render::DamageRect` (round outward, clamp at 0), and call `renderer.report_overlay_damage` before the draw. In bench mode the overlay is cleared, so do not report there; the renderer's fallback is correct.
3. Do the same in stress: the `button_panel` damage, plus a `DamageTracker` over the graph rectangles and lines, plus `tools.overlay()` as its own group.
4. After the draw, if `renderer.frame_skipped()`, call `std::thread::sleep(Duration::from_millis(1))`, so a still screen doesn't spin a core.

Commit: `camera, stress: report UI damage; idle 1 ms on a skipped frame`.

## Step 8: Docs

Files: `docs/systems/renderer.md`, `docs/architecture.md`, `docs/frame-reuse.md`.

1. In renderer.md, under Game use, add `report_overlay_damage`, `frame_skipped` and `reuse_stats`. Under Intent, add the reuse rules and the `GENOS_FRAME_REUSE` switch. Under Limits, add what is not reused, from the design note.
2. In architecture.md, Frame step 6 and 7: a frame whose scene and UI did not change submits and presents nothing.
3. In the design note, fix the stats names so they match the code: `scene_drawn`, `ui_repaired`, `full_restore`, `skipped`, `rects`.

Commit: `docs: renderer record and frame loop for frame reuse`.

## Step 9: One short headless check

Run `make verify` once: the stress smoke script, 320x180, no panel. It must pass as before. Optionally, run once with `GENOS_GPU_TIMES=60` and confirm that `FRAME_REUSE` shows `skipped` rising on a still view after about 300 frames.

## Risks to check on the owner's PC

- After 300 still frames, the default lighting now holds its light, which `cb6150c` turned on. Check for a visible step at the moment it holds. `GENOS_LIGHTING_SC_SETTLE=1000000` turns the hold off, and `GENOS_FRAME_REUSE=0` turns reuse off.
- Frozen frames may not match each other bit for bit: the direct pass picks lamps per frame. Reuse shows one of those frames.
- A wrong UI damage report leaves stale UI pixels. If the report is empty but the lists differ, the renderer falls back to its own diff.
- On skipped frames the FPS readout counts loop turns, not presents.

## What to measure

- `GENOS_GPU_TIMES=120` on a still view, before and after it holds: `GPU_MS frame=` and the `FRAME_REUSE` counts.
- Hover and slider drags over a still scene: `overlay` and `copy` ms. With a full redraw, they include raster and lighting.
- Moving camera: no regression in `GPU_MS frame=`. Expect one extra full copy per frame at most, and only on frames that hold.
- CPU and GPU power or utilisation on an idle still screen.
