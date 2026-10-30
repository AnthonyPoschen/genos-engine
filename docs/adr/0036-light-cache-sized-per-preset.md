# ADR 0036: GI light cache sized per preset and resolution

Status: Proposed (31 Oct 2026). No renderer code changes until Anthony accepts it.

## Context

- The GI world/surface light cache has a fixed size of `LIGHTING_CACHE_SLOTS = 1048576` slots. The value is set in `crates/render/src/lighting_gpu.rs` and repeated in `shaders/lighting_cache_slots.glsl`.
- The table is `SLOTS * 368` bytes, which is 368 MiB. The keys and side words (key table, screen-owner election, compose, and a-trous marks) add several more `SLOTS * 4` blocks.
- `allocates_cache()` in `crates/scene/src/lighting.rs` is true whenever GI, bounces > 0, and probes or the world cache are on. When that holds, every such preset pays the full 1M slots.
- The roadmap note "lighting VRAM" (27 Oct) puts lighting VRAM at 1440p at about 175 MB for Low, 750 MB for Medium and 1,024 MB for High. These are estimates. The same note estimates that 262K slots would bring the table plus keys down to about 107 MB. The soft budget is about 200 MB.
- The 31 Oct #73 root cause (lavapipe) is that genos-camera and lighting_ref run at High and so allocate this cache. The kernel then frees about 0.5 GB at vkDestroyInstance, which takes about 85 ms. The same memory is the main lead for the 4070's ~24 s quit stall. That lead is not yet proven.
- Lookup is open addressing over a power-of-two table. It probes a 16-slot window (`LIGHTING_CACHE_SEARCH`) and claims with CAS on a free key. A patch expires after `LIGHTING_CACHE_LIFE = 1800` frames without a read, unless its pixel is still on screen.
- When a lookup finds neither its patch nor a free slot, it bumps `gstats.missed`. `gstats.live` and `gstats.claimed` are already counted. So occupancy is measurable today without any new instrumentation.
- **No measured occupancy exists yet.** No test, note or log records peak `live` against SLOTS for any scene. Every slot count below is an estimate, and step 1 of the test plan exists to replace them with measurements.

## Decision

1. **The slot count is a runtime value, not a compiled constant.** It comes from preset × resolution, rounded to a power of two (the hash mask needs one). It is passed to shaders as a push or uniform `slots_mask`, which replaces `LIGHTING_CACHE_SLOTS - 1u`. Every `* LIGHTING_CACHE_SLOTS` layout offset in `lighting_gpu.rs` and `lighting_dispatch.rs` becomes a function of the chosen count.

2. **Table (ESTIMATES, to be replaced by measured peak `live` × 2 headroom, rounded up to a power of two):**

   | Preset | GPU class | Base slots at 1080p (estimate) | Table + keys (estimate) |
   |---|---|---|---|
   | Fallback (potato) | 4 GB floor, iGPU, lavapipe | 0 when `allocates_cache()` is false; otherwise 64K | ≤ ~27 MB |
   | Low | GTX 1060 6 GB / RX 580, Deck 800p | 128K | ~54 MB |
   | Medium | mid-range | 256K | ~107 MB (roadmap figure) |
   | High | 4070 class | 512K | ~214 MB |
   | Ultra | 16 GB+ | 1M (today's size) | ~428 MB |

   Per-slot cost is about 428 B/slot. That figure is derived from the roadmap's "262K → ~107 MB" and is consistent with the 368 B table plus the keys and side words.

3. **Resolution scaling.** On-screen surface patches grow with pixel count. The base count is scaled by `pixels / (1920*1080)`, rounded to the next power of two, and clamped to between ½× and 2× of base. At 1440p (1.78×) that rounds to 2×. Medium at 1440p therefore becomes 512K. Whether occupancy really scales with pixels is to be checked in test step 1. If it does not, the scaling is dropped and the count follows preset only.

4. **The 4 GB floor.** On a device with ≤ 4 GB of device-local heap, slots are capped at the Low value. Even under a Medium or High preset, cache memory then stays ≤ ~54 MB (estimate). This keeps ADR 0023's minimum spec (4 GB, Low runs) inside budget. The budget helper (`budget.rs`) reports the chosen count and bytes.

5. **Overflow behaviour (table full, `missed > 0`):**
   - Eviction is by priority, not first-come. A claim that finds no free slot in its 16-slot window may take the window's lowest-priority slot. Priority is a mix of how recently the slot was shown (`shown` frame), its distance to the camera, and how converged it is (sets). Near-camera, on-screen and young slots are never evicted for far ones. That way, errors near the camera are weighted the most.
   - No pops or flicker. An evicted slot is not shown cold. The new patch starts from the screen probe or irradiance fallback that unclaimed lookups already use today, and blends in over its normal set count. The old patch simply stops being read. Far, off-screen patches go first, so the change is not visible.
   - Lifetime pressure. When `missed` stays above 0.1% of lookups for 60 frames, the effective `LIGHTING_CACHE_LIFE` halves, with 300 frames as the lowest it goes. It recovers once misses stay at 0 for 600 frames. This trims stale patches before eviction is needed.
   - A sustained miss rate (> 1% for 5 s) is reported to auto-tune as "cache undersized". Auto-tune may raise the count within the VRAM budget. That counts as a resize (point 6).

6. **Resizing.** A change of preset, resolution, or auto-tune count reallocates the table. This uses the existing `want_cache` reallocate path: destroy, then make, then `cache_clear = true`. The reallocation is deferred to a frame boundary after a queue idle, as the current path already does.
   - Contents are not migrated. The slot index depends on the mask, so the cache refills. To avoid a visible GI drop, the frame of the resize and the following ~30 frames weight the screen probes and history more, using the same mechanism as a cold start.
   - A resize on window drag is debounced by 500 ms.
   - Shrinking always frees memory at once, so quitting from Low or Fallback releases ≤ ~54 MB.

7. **Out of scope:** trimming High kept hits (~150 MB) and the screen-denoise slices (~113 MB). Those are the other levers in the roadmap's VRAM item and get separate decisions.

## Consequences

- At default settings, High lighting VRAM drops by about 214 MB (estimate). Medium drops by about 320 MB (estimate). Medium and High at 1080p get close to the ~200 MB soft budget for the cache.
- If the 4070 A/B confirms the lead, #73 is fixed at its source instead of being hidden by running the quit test at Fallback.
- Cost: one extra uniform, and offsets computed at run time instead of as consts. The layout asserts in `lighting_gpu.rs` (around line 1055) must hold for every allowed count, and a unit test enumerates them.
- Risk: an undersized table in large open scenes. The miss counter, eviction priority and auto-tune feedback contain that risk, and the test plan measures it.

## Test plan

1. **Lavapipe occupancy test (box).** Run it with `/home/box/shared/toolchains/genos/bin/genos-box-test`. For each preset (Fallback, Low, Medium, High) and at 1080p and 1440p, run the gi-check room and Sponza for 600 frames with a scripted camera sweep. Dump `gstats.live`, `claimed` and `missed` per frame using the existing stats JSON (`cache_slots` is already in the header). The test asserts:
   - The peak `live` at the chosen count is ≤ 50% of SLOTS.
   - `missed == 0` with the new counts.
   - Slots × 428 B stays within the budget table above.

   These measured peaks replace the estimates in Decision 2 before implementation is merged.
2. **Forced-overflow test (box).** Force SLOTS = 4K on Sponza. Assert:
   - `missed > 0`.
   - No slot within 5 m of the camera that is shown in the last 60 frames is evicted.
   - The frame-to-frame luminance delta of the near-camera pixels stays under the existing pop gate.
3. **Resize test (box).** Switch the preset High → Low → High and the resolution 1080p → 1440p mid-run. Assert:
   - No validation errors.
   - Buffer bytes match the table.
   - No NaN or black frames.
   - The luminance drop is under the pop gate within 30 frames.
4. **#73 A/B on the 4070 (Sunday 05:18 AEDT window, zanven-pc).** Run genos-camera and lighting_ref with `GENOS_LIGHTING_PRESET=potato` against High, 3× each, and record every quit phase.
   - If potato is ≤ 100 ms and High still shows the ~24 s stall, the lead is confirmed and this ADR fixes #73.
   - If both stall, the cache is not the cause, and this ADR stands on VRAM grounds alone.
5. **4 GB floor.** A unit test on the slot selector uses a mocked 4 GB heap and checks that the result is ≤ Low for every preset.
