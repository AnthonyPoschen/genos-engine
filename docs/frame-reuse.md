# Frame reuse

A frame redoes only the work whose inputs changed. The UI reports the screen regions it changed. The renderer keeps the last lit scene picture on the GPU, repairs only the changed UI regions over it, skips acquire and present when the screen would not change, and tells the compositor which regions of a presented frame are new.

Every decision is made on the CPU from state the CPU already holds: the bytes it uploads, the UI paints, and the lighting counters. Those counters were copied to host memory at the end of an earlier frame. The draw reads them only after that frame's fence has been waited. It does not add a GPU readback.

## Layers

A frame has three pictures:

- The **scene picture** is one device image (`FrameReuse::scene` in `crates/render/src/reuse.rs`). It holds the lit, filtered scene without the UI, from the last frame whose picture can be reused.
- The **frame picture** is the colour image of a frame in flight (`colors[slot]`, two of them). It holds the scene with the UI drawn over it. Readbacks copy this picture.
- The **swapchain image** gets a whole copy of the frame picture each presented frame.

## How dirty regions flow

1. **UI.** `genos-ui` diffs each frame's paints against the last frame's, per UI element (`DamageTracker` in `crates/ui/src/damage.rs`). An element whose rectangle, look, text or paints changed dirties its old and its new bounds. A new or removed element dirties its bounds. A viewport change dirties the whole screen. Close rectangles merge, and at most eight are kept. `lighting_frame` and `button_panel` return those rectangles in `Frame::damage`. A producer with no layout, such as the profile graph, feeds its own `DamageTracker`.
2. **Caller.** The frame joins the damage of each UI producer and hands it to `Renderer::report_overlay_damage` before the draw. A caller that reports nothing still draws correctly: the renderer diffs the overlay rectangles and lines itself, entry by entry.
3. **Renderer.** The draw adds the frame's UI damage to the damage of both frames in flight. A frame picture is two frames old when its slot records again, so its slot carries the damage of every frame since it was last written.
4. **Frame graph.** The draw keys the scene: the scene block (camera, lamps, occluders, debug view), the traced mesh instances, the draw instances, the dynamic vertices (particles), the draw list, the shape pool and particle image generations, the picture filter, the wireframe switch, the sky and the size. Then it picks one of four paths:

| Scene key | Lighting verification | UI and screen | Work this frame |
| --- | --- | --- | --- |
| changed, or the scene picture is not cached | any | any | Raster, the full lighting passes, the filter, the whole UI. Once the key has repeated for a frame, the scene without the UI is copied into the scene picture. A moving camera does not pay that copy. |
| same as the scene picture | gradient checks and the probe check rays found no real change | UI unchanged, and the swapchain already shows it | No acquire and no present. The verification budget still runs. |
| same as the scene picture | the same | some UI damage | Copy the scene picture into the damaged regions, draw the UI scissored to those regions, copy to the swapchain and present. Verification runs as well. |
| same as the scene picture | the same | this flight's colour image is a different picture | Copy the whole scene picture, draw the whole UI, present. |

5. **Present.** When the device has `VK_KHR_incremental_present`, the present carries the regions that changed since the last presented frame: the whole screen after a scene draw, else the UI damage. The swapchain image is still written whole, so the regions are a hint the compositor may use. On Wayland, Mesa passes them on as surface damage.

## The lighting gate

Reuse is not gated on the settle counter. The lighting keeps a small budget running on every reused frame: the cache compact, the dense cache rays (the gradient checks), and one check ray per held screen probe. A frame may be reused only after a finished run of that budget, for this same scene key, actually ran checks and reported no real change. A real change is a gradient check past its epsilon, or a probe check ray whose hit moved. Either one drops the cached picture. The next frame draws the scene again.

A result of zero fails does not count when the frame ran no checks. The frozen short list (direct light and compose only) does not run while reuse is on, because that list stops the checks and a later zero would look clean forever. `picture_settled` still follows the settle counter. It does not decide reuse.

The scene picture is captured once the key has repeated, from a full frame that is not waiting on an async light-cache round. Two flights never write that image at once. With no lighting passes, the scene key alone is the gate.

## Reused when

- **Still camera, still scene, still UI:** after a clean verification, nothing is acquired or presented. The verification budget still runs.
- **Still scene, live UI (hover, a slider drag over a still lamp, the profile graph):** the cached scene is kept and only the dirty UI rectangles are redrawn. The verification budget runs. Raster, the per-pixel direct pass, gather, filter, a-trous and compose do not.
- **Moving camera or moving objects:** the scene is drawn as before. The UI is drawn whole over it. The present regions are the whole screen. The scene picture is not copied on a key that just changed.

## Not done

- **A cached UI layer texture.** The UI is a few thousand opaque rectangles in one draw, and its vertices are rebuilt only when the paints change. Blending a cached layer would read the whole UI area every frame the scene moves, and drawing the rectangles writes only their pixels. Measure `overlay` in `GENOS_GPU_TIMES` before adding one.
- **Partial scene redraw.** A moving object still redraws the whole scene. The lighting passes are full-screen compute, so a scissored raster alone would save little.
- **Partial swapchain copies.** The swapchain image is copied whole. Copying only damaged regions would rely on a swapchain image keeping its contents between presents and on tracking each image's age.

## Switches and measures

`GENOS_FRAME_REUSE=0` turns reuse off and restores the old path, including the frozen lighting short list once the settle counter is reached. Every frame draws the scene, the UI and a full present.

`GENOS_GPU_TIMES` adds a `FRAME_REUSE` line beside `GPU_MS`: `scene_drawn`, `ui_repaired`, `full_restore`, `skipped`, `rects`, `invalidated`. `skipped` counts frames that did not present. When lighting is on, those frames still submitted the verification budget, and the `GPU_MS` span includes it. GPU time is 0 only when a skipped frame submitted nothing. `invalidated` counts verification results that found a real change and dropped the cached picture. `Renderer::reuse_stats` returns the same counts. `Renderer::frame_skipped` is true when the last draw did not present.
