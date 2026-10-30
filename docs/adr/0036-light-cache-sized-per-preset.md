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

## Measurements (31 Oct 2026, box lavapipe, master @ this branch's base)

Runs used genos-stress (small stress world, which has no hall checkpoint) at `--size 1920x1080`, `GENOS_LIGHTING_PRESET=high`, `GENOS_LIGHTING_DEBUG=1` and `GENOS_LIGHTING_SC_TRACE=1`. The counters are read from the existing `lighting frame … live N missed M` and `lightingsc … claimed N` lines. No code was added. "Peak" is the highest `live` of any readback. "Peak after 30" leaves out the first 30 readbacks (the cold-start fill). "Steady" is the mean of the last 30 readbacks.

| Scene (1080p, High) | Readbacks | Peak live (frame) | Peak after 30 | Steady live | Max claimed in one readback | Missed |
|---|---|---|---|---|---|---|
| Basic (default world, 240 frames still) | 243 | 53,996 (f6) | 37,510 | 622 | 11,607 | 0 |
| Stress (default world, still; stopped at 64 frames by wait timeout) | 62 | 153,188 (f4) | 100,288 | 63,247 | 75,562 | 0 |
| Switch (two views, teleport; stopped early) | 40 | 178,628 (f39, after the teleport) | 178,628 | 112,106 | 75,562 | 0 |
| Sun x100 (full rotation stepped per frame, twice) | 469 | **407,116 (f6)** | **315,982 (f46, mid-rotation)** | 33,451 | 78,928 | 0 |
| Spin / turn (`hall` checkpoint missing in this world, aborted) | 4-7 | 153,340 (f4) | n/a | n/a | 75,562 | 0 |

Findings:
- No run came near 1,048,576. The worst value seen is 39% of today's table, and `missed` was 0 everywhere.
- The worst moment is the cold-start fill of the sun scene and the sun moving: the light changes everywhere, so patches stay live. A camera teleport (switch) is the next worst.
- Most runs are partial. Spin, turn and the stress run were cut short (rc=1) by a script error or a timeout. Only basic and sun ran to the end.
- `allocates_cache()` by preset (code reading, `crates/scene/src/lighting.rs`):
  - Potato sets `gi.enabled = false`, `bounce.count = 0` and `probes.enabled = false`, so it is **false**. Potato allocates no cache. This confirms the #73 quit-test choice.
  - Low keeps GI and probes on with `bounce.count = 1` (`world_cache.enabled = false`), so it is **true**. Low currently pays the full 1M-slot table (~368 MiB) even though it has no world cache. That is the clearest waste.

**Not measured (gaps):**
- 1440p. The reruns failed at once (MCP port bind clash), and the box was then busy with the #98 oracle run.
- Sponza.
- A full spin, turn and switch with the `hall` checkpoint. These need `genos-lighting --scene stress --checkpoint hall`.
- Medium, Low and Ultra occupancy.
- Any 4070 run.

**Lavapipe versus the 4070:**
- Slot occupancy depends on the scene, the camera, the fixed_dt and the resolution, not the GPU, so it should transfer.
- It may not transfer for three reasons:
  - Lavapipe runs at ~1-4 s per frame, so each readback covers fewer frames, and the GPU→CPU readback lag differs.
  - Frame-budgeted work (batch and boost) may claim at a different rate per frame.
  - `LIGHTING_CACHE_LIFE` counts frames, so expiry is the same per frame but not per second.

## Decision

1. **The slot count is a runtime value, not a compiled constant.** It comes from preset × resolution, rounded to a power of two (the hash mask needs one). It is passed to shaders as a push or uniform `slots_mask`, which replaces `LIGHTING_CACHE_SLOTS - 1u`. Every `* LIGHTING_CACHE_SLOTS` layout offset in `lighting_gpu.rs` and `lighting_dispatch.rs` becomes a function of the chosen count.

2. **Table (High from measurement; the others scaled from it and marked as estimates):**

   Rule: slots = next power of two ≥ measured peak live × 1.5 headroom. Linear probing with a 16-slot window starts to miss well before 100% load, so the target is at most ~66% load at the worst moment.

   | Preset | Basis | Slots at 1080p | Load at measured worst | Table + keys (~428 B/slot) |
   |---|---|---|---|---|
   | Fallback (potato) | `allocates_cache()` is false (measured in code) | 0 | n/a | 0 |
   | Low | ESTIMATE: has no world cache and is probe-only, so it claims fewer patches than High; unmeasured | 256K | n/a | ~107 MB |
   | Medium | ESTIMATE: same paths as High with fewer rays; take High's | 512K | n/a | ~214 MB |
   | High | MEASURED: worst live 407,116 (sun x100 cold fill) → ×1.5 = 611K → **1M**; peak after fill 315,982 → ×1.5 = 474K → 512K | **512K** plus burst handling (below), else 1M | 78% (burst) / 60% (after fill) | ~214 MB |
   | Ultra | Keep today's size | 1M | 39% | ~428 MB |

   Why High is 512K and not 1M: the 407K peak is the first ~6 readbacks of a cold start. It sits above the 316K seen once warm, and no other scene comes near it. At 512K it is 78% load, which is acceptable for a few frames *if* overflow is graceful (point 5). Missed claims during the fill fall back to the screen probes, which is the same path a cold start already uses. If the overflow test (test plan step 2) shows visible artefacts at 512K during the fill, High stays at 1M and only Low, Medium and the 4 GB cap shrink.

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

   These measured peaks replace the remaining estimates in Decision 2 (Low, Medium, 1440p, Sponza) before implementation is merged.
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
