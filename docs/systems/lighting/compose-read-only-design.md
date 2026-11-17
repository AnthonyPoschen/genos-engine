# Read-only compose: removing the High quit stall (#142)

Status: proposal for review. No shader code yet. Evidence: #142 comments 6550259859 and 6550314968, and `ge-4070-verify-28oct.md` (1 Nov, Tasks 4–5). Comment 6549812392's `sum += bil*v.rgb` trigger is superseded: the WORLD=1/2 stalls were flaky.

## Problem

On the 4070 (1080p High, mode 3, master a644835c / 4c4fa62d), `vkDestroyDevice` stalls about 20 s on quit. The stall equals the `lighting compose` pipeline compile time, 1:1:

| variant (6 frames) | compose create ms | vk_destroy_device ms |
|---|---|---|
| base | ~20 600 | 20 400–20 900 |
| no first-fill | 15 902 | 16 166 |
| no resample | 6 622 | 6 545 |
| no parent | 5 744 | 5 599 |
| warm private NV disk cache | 2.5 | 20 600 / 20 422 |
| resample (~L2851) + parent (~L2872) gated off | 1 459 | 22 / 39 |

Mechanism: compose inlines the whole mode-3 body of `lighting_world_irradiance_in` (`lighting_cache.glsl`), including the fill, resample, parent and `sc_offer` side effects. The NVIDIA compiler takes about 20 s on it. After about frame 5 the driver starts a second, uncached background recompile, and `vkDestroyDevice` joins it (gdb: a worker thread in `libnvidia-gpucomp` at ~90% CPU; GPU idle; `live_allocs=0`). A warm disk cache removes the startup compile but not the exit stall. We pass `PipelineCache::null()`, but a pipeline cache would not help either for the same reason. The cost follows compile complexity, not any one line.

## Proposal: compose is a pure lookup

Split `lighting_world_irradiance_in` into:

- **Lookup (compose):** find the texel and its parents, bilinear-weight the lit sets, and return irradiance plus a miss flag. No atomics, no allocation, no buffer writes.
- **Maintenance (cache pass, or a small new compute pass right after compose):** mode-3 first-fill, resample, parent seeding and `sc_offer`. Compose leaves a compact miss list (append-only; slot plus screen tile), and the maintenance pass drains it.

Compose then compiles to about the "gated off" size (~1.5 s). With no huge pipeline left, the driver has nothing big to recompile in the background, so the destroy stall should go away. The cache pass gets bigger but has a much smaller body, and its compile time has to be measured too.

## Alternatives

1. **Specialization constant** (`MODE3_SIDE_EFFECTS`, false in compose). This is the smallest change and is what run s1/s2 measured. But turning it off removes the effects, it doesn't move them, so fill, resample and parent stop happening from compose's misses. On its own it's a picture regression. It's still useful as a kill switch and an A/B tool.
2. **Non-inlined lookup path.** GLSL has no reliable no-inline, so this would mean a loop or branch structure that keeps the body cold. Whether the driver obeys depends on the driver version, and the side effects stay in compose. It's fragile and not recommended.
3. **Do nothing and skip destroy on quit** (exit without `vkDestroyDevice`). That hides the 20 s startup compile only on a warm cache and breaks the leak checks. Rejected.

## Picture equivalence

The split changes when fill happens: a texel first seen by compose in frame N is filled in frame N (now) or N+1 (after the split).

- Box oracle diffs (lavapipe, `diff.py` against the pre-split head): turn.rhai still/turned/turned+16, and the hall jump. Pass if mean |d| ≤ 1.5 and p99 ≤ 8, the same bounds as the #70 controller-off reference.
- C5 scenes on the 4070 (spin, sun-x10, sun-x100, switch, turn), near-camera error against the path-traced reference. Pass if no scene gets worse by more than run-to-run noise (about ±0.01 on the near metric).
- A frame-by-frame check over the first 10 frames after a jump, to catch the one-frame fill latency: allowed only as a single darker frame, with no pop on the frame after.

## Expected compile and frame time

- Compose create: about 1.5 s (measured with the side effects gated off). The maintenance pass is expected to take seconds at most; measure it.
- Exit: `vk_destroy_device` under 0.05 s (measured: 22 / 39 ms).
- Frame time: compose gets cheaper. The new pass costs about as much as the work it moves, plus the miss-list traffic. Expect a net change of about ±0.2 ms on the 4070 at 1440p.

## Risks

- **One-frame latency on fill.** A disocclusion reads a parent or a miss for one more frame. Mitigation: compose already falls back to the parent level, so the result is briefly coarse rather than black. The latency check above bounds it.
- **Miss-list overflow** on a fast turn. Mitigation: a cap with drop-oldest, and counters in the debug HUD. Dropped misses are found again next frame.
- **Ordering:** `sc_offer` and quiet-light bookkeeping (`sc_light_quiet`) assume they run in compose order. Keep the same per-frame order in the maintenance pass, and diff the unfilled tallies before and after.
- The driver may still recompile something big in the background, for example the cache pass. Exit-stall check per pipeline.

## Test plan

1. Specialization-constant kill switch first, to reproduce s1/s2 on the current head (compose create, destroy).
2. Implement the split, then run the box oracle diffs and unit tests (miss-list cap, ordering).
3. 4070, 1080p and 1440p High: per-pipeline create times, `vk_destroy_device` over 6, 40 and 150 frames (cold and warm NV cache), `--gpu-times` per pass, the C5 scenes, and the quit-gap budget. Run when no one is using the PC.
4. Land only if every pipeline compiles in under 3 s, destroy takes under 0.1 s, and the picture checks pass.
