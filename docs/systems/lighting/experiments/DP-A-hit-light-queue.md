# DP-A hit-light queue (not measured here)

Item A of [dynamic-perf-plan.md](../dynamic-perf-plan.md), steps A1–A5. A6 is not in this branch. The default is off, and the shipped direct shader is a separate SPIR-V from the enqueue shader. No timings in this file. Measure on the RTX 4070.

## What the toggle does

`GENOS_LIGHTING_HIT_QUEUE=1` is read every frame.

- Direct dispatches bind the enqueue shader. After the TDL decision, a GI hit (`idx >= pixels`) finishes inline when the texel is reused (`tdl` has bit `0x80000000`; `~0u` is "no texel", not that flag) or every lamp with weight above zero already has a stored shadow (`lighting_shadow_stored`, so the pre-scan does not count d2/d3). A sun or the flame has no stored shadow.
- Otherwise the hit appends `uvec2(canonical index, tdl)` and returns. The consumer does not repeat the gates or the TDL budget `atomicAdd`.
- Capacity is `2^21` entries (16 MiB in `cache_keys`). A hit that does not fit lights inline. Nothing is dropped.
- An args step (pass 0, `params.y` bit 16) writes the indirect dispatch, stores `dp[19]` (entries appended) and `dp[20]` (lit inline because the queue was full), and clears the counter. It does not call `lighting_write_args`, so shadow epochs are not bumped a second time.
- The hit-light pass (pass 9) runs one thread per queued entry and shadow-traces only those.

The canonical index is the mode-0 layout: pixels, kept hits, cache rays, fills. Mode 1 numbers fills where mode 0 numbers cache rays, so a mode-1 fill is stored with `LIGHTING_CACHE_BATCH * LIGHTING_CACHE_RAYS` added.

One queue serves both direct dispatches of a round. Order: the directs, then the args step, then hit-light, then gather. A frame without a round inserts the same pair between its direct pass and gather. Frozen frames do not.

`GENOS_LIGHTING_ASYNC=1` keeps the shipped direct shader on the async submit and does not set the arm bit. Those cache rays stay inline and do not share the frame's counter.

Toggle off: the shipped direct SPIR-V, no args step, no hit-light dispatch.

The queue, the counter, and the indirect args stay in device memory. The frame loop does not map them. `dp[19]` and `dp[20]` come back through the existing stats copy, printed only when `GENOS_LIGHTING_SC_TRACE=1`.

## Toggles

| Variable | When | Effect |
|---|---|---|
| `GENOS_LIGHTING_HIT_QUEUE=1` | runtime, every frame | enqueue shader, args step, hit-light pass |
| `GENOS_LIGHTING_DIAG=1` | compile time | `dp[0]`–`dp[18]`. Not required for `dp[19]` / `dp[20]` |
| `GENOS_LIGHTING_SC_TRACE=1` | runtime | prints `lightingdp` from the previous frame's stats copy |
| `GENOS_GPU_TIMES=600` | runtime | `LIGHTING_MS` every 600 frames |
| `GENOS_GPU_DISPATCH=1` | runtime | `LIGHTING_DISPATCH i:pass=ms` in dispatch order |
| `GENOS_DP_DIVERGE=1`, `2`, or `3` | compile time | phase 0.3 only. The picture is wrong. Leave it unset for the A/B |

Do not combine a non-zero `GENOS_DP_DIVERGE` with the hit-light toggle. The divergence test is the shipped direct pass.

Pass ids: 0 compact (every args step is also 0), 1 place, 2 trace, 3 direct, 4 gather, 5 filter, 6 cache, 7 compose, 8 a-trous, 9 hit-light. `LIGHTING_MS` names pass 9 `hit`. With the toggle off, `hit` is 0.

A round with the toggle on is 23 dispatches. The timestamp pool holds 24 stamps (one before the list and one after each dispatch).

## What to measure

Still scene and `perf/moving.rhai`, 1920×1080. Timing runs leave `GENOS_LIGHTING_DIAG` and `GENOS_LIGHTING_SC_TRACE` unset. Counter runs are separate.

1. **Phase 0.1 profile.** Toggle off, diverge unset. `GENOS_GPU_TIMES=600 GENOS_GPU_DISPATCH=1` for the moving scene and the still scene. Record `LIGHTING_DISPATCH`: trace, both direct dispatches, gather, the three filter rounds, cache, each of the seven a-trous dispatches, compose, and compact. That table is `experiments/DP0-moving-profile.md`. This branch does not invent it.

2. **Phase 0.2 counters.** Rebuild with `GENOS_LIGHTING_DIAG=1`. Run with `GENOS_LIGHTING_SC_TRACE=1`, toggle off. Read `lightingdp` (not `lightingdg`):

   | `dp` | Meaning |
   |---|---|
   | 0 / 1 | shadow rays traced by GI hits / by pixels |
   | 2 / 3 | hit shadow-cache hits / misses in the lamp loop |
   | 4 | GI hits with no fine texel |
   | 5 | GI hits that lost the TDL budget |
   | 6 / 7 | pixel lamp-list entries whose epoch held / all entries in that loop (rounds only) |
   | 8–12 | a-trous work-list count, levels 1–5 |
   | 13 / 14 | stage-0 texels recomputed / level outputs that scattered |
   | 15 / 16 | pixels with world trust `< 0.999` / tiles with at least one |
   | 17 / 18 | hash lookups that miss (`claim == false`) / that return a slot |

   `lightingchg` slots 6–8 are still kept-hit direct light: TDL reuse, check, lamps. `perf/chg.py` reads that line.

3. **Phase 0.3 divergence.** Three rebuilds, `GENOS_DP_DIVERGE` 1, then 2, then 3. Toggle off. Moving scene. Compare the direct pass (`LIGHTING_MS` `light`, and the two pass-3 lines in `LIGHTING_DISPATCH`) with the step 1 baseline.

   - 1 skips every GI-hit shadow ray.
   - 2 skips a random half (`lighting_hash(idx) & 1`).
   - 3 skips a coherent half (`lighting_hash(idx >> 6) & 1`), one decision per 64 threads.
   - If 2 saves far less than half of 1, and 3 saves about half, warps are diverging and this queue is the lever.
   - If 2 and 3 save the same, the cost is the ray count. Then B and C rank above A.

4. **Empty hit-light pass.** The A3 commit dispatches the pass before anything enqueues. On that revision, toggle on, still scene: `hit` plus the extra pass-0 line. The plan's ceiling is 0.02 ms.

5. **Item A, this revision.** Same binary, toggle off then on. Diag and the divergence macro unset.

   - Toggle off: `LIGHTING_MS` `light`.
   - Toggle on: `light` + `hit`, and the extra pass-0 args line in `LIGHTING_DISPATCH` (the one after the directs).
   - Moving and still.
   - Queue length, separate run with `GENOS_LIGHTING_SC_TRACE=1` (diag not required): `lightingdp` slot 19 is entries appended, slot 20 is how many lit inline because the queue was full. The hit-light dispatch count is 19 minus 20.

6. **Picture.** Pixel-exact compare of a still frame, toggle off against on, using the S78S dump's per-pixel direct and kept-light CSVs. The lamp loop is the same code in the same frame. Which hits win the TDL budget `atomicAdd` was already nondeterministic.

7. **A6 stays out** unless the hit pass itself is diverging on lamp count: `hit` stays high while most queued hits are misses, and those misses differ in how many lamps they trace. Only then split one thread per (hit, lamp-with-a-miss).
