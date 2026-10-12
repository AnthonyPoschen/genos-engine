# Frame reuse

A frame redoes only the work whose inputs changed. The UI reports the screen regions it changed. The renderer keeps the last lit scene picture on the GPU, repairs only the changed UI regions over it, skips the whole frame when nothing on screen changed, and tells the compositor which regions of a presented frame are new.

Every decision is made on the CPU from state the CPU already holds: the bytes it uploads, the UI paints, and the lighting's own counters. The frame loop reads nothing back from the GPU.

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

| Scene key | Lighting | UI damage | Work this frame |
| --- | --- | --- | --- |
| changed, or the scene picture is not cached | any | any | Raster, lighting, filter, the whole UI. When the lighting held its light this frame, the clean scene is copied into the scene picture first. |
| same as the scene picture | holds its light | none since this slot was written, screen unchanged | Nothing. No acquire, no submit, no present. The last presented image stays. |
| same as the scene picture | holds its light | some | Copy the scene picture into the damaged regions of the frame picture, draw the UI scissored to those regions, copy to the swapchain and present. |
| same as the scene picture | holds its light | slot holds another scene | Copy the whole scene picture into the frame picture and draw the whole UI. |

5. **Present.** When the device has `VK_KHR_incremental_present`, the present carries the regions that changed since the last presented frame: the whole screen after a scene draw, else the UI damage. The swapchain image is still written whole, so the regions are a hint the compositor may use. On Wayland, Mesa passes them on as surface damage.

## The lighting gate

The lighting reports no change when it holds its light: the scene has stayed still long enough, the light cache has stopped changing, and a frame was drawn from that held light (`Renderer::picture_settled`). A scene picture is cached only from a frame drawn from held light, and it is reused only while the lighting still holds and no async light cache round is in flight. Any scene draw that does not cache its picture drops the cached one, because the lighting may have moved on before the scene comes back to the same key.

In the default surface cache mode the frame no longer reads the light cache's counters back (S70B), so the rounds since the light last changed are counted on the CPU from the rounds the frame submitted. Without that count the lighting never reported held light in the default mode.

## Reused when

- **Still camera, still scene, still UI:** after the lighting holds its light, the GPU does nothing and nothing is presented.
- **Still scene, live UI (hover, a slider drag over a still lamp, the profile graph):** only the UI regions that changed are rebuilt. No raster and no lighting run.
- **Moving camera or moving objects:** the scene is drawn as before. The UI is drawn whole over it, since it lands on a new picture. The present regions are the whole screen.

## Not done

- **A cached UI layer texture.** The UI is a few thousand opaque rectangles in one draw, and its vertices are rebuilt only when the paints change. Blending a cached layer would read the whole UI area every frame the scene moves, and drawing the rectangles writes only their pixels. Measure `overlay` in `GENOS_GPU_TIMES` before adding one.
- **Partial scene redraw.** A moving object still redraws the whole scene. The lighting passes are full-screen compute, so a scissored raster alone would save little.
- **Partial swapchain copies.** The swapchain image is copied whole. Copying only damaged regions would rely on a swapchain image keeping its contents between presents and on tracking each image's age.

## Switches and measures

`GENOS_FRAME_REUSE=0` turns all of it off: every frame draws the scene, the UI and a full present as before.

`GENOS_GPU_TIMES` adds a `FRAME_REUSE` line beside `GPU_MS`: the frames that drew the scene, reused it with UI repairs, reused it with no GPU work in the slot, and skipped the frame entirely, plus the UI rectangles repaired. `Renderer::reuse_stats` returns the same counts.
