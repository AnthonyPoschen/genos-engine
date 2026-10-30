# What the "direct" lighting bucket spends, and how to cut it

Status: findings and proposal, for review. No renderer change rides on this note.

## Why this matters

The goal is a 2 ms frame for the basic scene at 1440p on an RTX 4070 (about 500 fps), about 100 fps for the stress scene, photoreal against the path-traced reference, no flicker, about 200 MB of lighting VRAM, and a Low preset that runs on a GTX 1060. What counts is the cost of a full relight, not tricks that skip unchanged frames.

On 30 Oct the 4070 measured the direct bucket (`light=` in `LIGHTING_MS`) at **16.0 ms** with the ReSTIR flag off, at 2560×1440 while the PC was in use. On its own, that is eight times the whole frame budget. The point-lamp shading people usually picture as "direct light" is only about 0.3 ms of it.

## What the bucket actually is

The bucket times one compute pass, `lighting_direct.comp`, which runs in up to three dispatches each frame:

| Dispatch | Points shaded | What each point does |
|---|---|---|
| Pixels + kept probe hits | every 1440p pixel (3.69 M) plus 2 × the kept screen-probe hits | Every sun: an exact shadow ray (pixels always trace; hits may reuse a cached shadow cell). Point lamps and the flame: a weighted pick sharing one shadow ray. Hits also add the light cache's bounce and write the light sent back along the probe ray. |
| Cache-ray hits (`B_HIT`, async rounds) | the light cache's own rays | The same lamp, sun and flame evaluation, for bounce 2 and up |
| Kept hits only | when direct is off but GI is on | The same work on hits only |

So the "direct" bucket is really **all lights evaluated at every surface the lighting system touches**: the picture itself and every GI ray hit. Most of that cost is sun visibility and GI hit lighting, not lamps.

### Timing double-count

`gpu.rs` also added each of these dispatches to a `shadows` bucket. That bucket is not a separate pass, so the 30 Oct reading "shadows 15.7 + initial 8.1 + …" counted the same work twice. PR #123 drops `shadows=` from `LIGHTING_MS` and prints `light_px=` and `light_hit=` instead, so the bucket can be split honestly.

## Estimated shares of the 16.0 ms

These are estimates from the code and the existing 4070 numbers. None of them has been measured on its own yet.

| Part | Estimate | Reasoning |
|---|---|---|
| Sun shadow ray per pixel | 4–6 ms | One incoherent ray query per pixel, 3.69 M rays, in a compute shader without hardware ray reordering |
| Lighting GI hits (kept probe hits + cache rays): sun ray, lamp pick, cache read | 6–8 ms | 2 × kept hits plus cache rays, each a scattered memory read and usually a trace. Matches #70, where direct scaled with hit count (1.7–1.9×). |
| Flame (picked as a lamp, plus its signature and hash work) | 0.5–1 ms | Joins the pick sum at every point |
| Point lamps (pick loop over the list) | ~0.3 ms | Measured with ReSTIR on and off |
| Per-point overhead (G-buffer and history reads, packing, change counters) | 1.5–2.5 ms | Wide, branchy shader with low occupancy |
| Load inflation | ~2.7 ms | The clean run on 30 Oct 08:00 read 13.3 ms against 16.0 ms under load |

## Ways to cut it, ranked

| # | Change | What industry does | Est. saving (4070, 1440p) | Quality risk |
|---|---|---|---|---|
| 1 | **Light GI hits from the surface/radiance cache, not from lights.** A probe or cache hit reads the cached lit surface, and the cache is relit in a small budgeted pass. | Lumen: final-gather hits sample the lit surface-cache cards and never trace a lamp. Also DDGI and SHaRC. | 5–7 ms | Medium: lag after a light changes, which is already handled by cache invalidation. Leaks if the cache is coarse. Check against the oracle grid score. |
| 2 | **Sun visibility from shadow maps**: cascaded shadow maps now, virtual shadow maps later, with ray-traced contact or penumbra only near edges. | UE5 virtual shadow maps; most shipped games use cascades | 3–5 ms (cascades ~1 ms for 4 × 2048²) | Low to medium: blocky or soft edges, peter-panning. VRAM is 16–64 MB. Also suits the GTX 1060 Low preset. |
| 2b | Alternative to 2: **half-resolution or checkerboard sun rays plus a denoiser** | MegaLights, the RTX shadow denoiser (SIGMA) | 2–3 ms | Medium: temporal blur or flicker on thin casters, and the flicker gate is strict |
| 3 | **Fold the flame into the lamp list** (one reservoir candidate), with no separate per-point path | MegaLights / ReSTIR treat every emitter the same | 0.5–1 ms | Low |
| 4 | **Slim the per-point shader**: split pixels from hits so the pixel dispatch stays coherent, move change counters behind a debug flag, cut history reads | Standard occupancy work | 1–2 ms | Low |
| 5 | **Budget GI hit counts** (kept hits ×2, cache rays per round) against a fixed ms target | Lumen's fixed probe and ray budgets | 1–3 ms | Medium: noisier GI, so it depends on the denoiser |

Even if every saving lands, the bucket still ends up at about 2–4 ms. So a 2 ms whole frame needs #1 and #2 together, plus the GI and compose cuts already planned. Direct light can't reach that target alone.

## Recommendation

1. Merge #123, then take one clean 4070 run that prints `light_px` and `light_hit` separately. Every estimate above depends on that split.
2. Do #1 first (cache-lit GI hits). It is the biggest saving, it is what Lumen does, and it also stops each extra light making GI cost more.
3. Then do #2 with cascaded shadow maps as the default sun path, keep ray-traced sun shadows as a High-preset option, and use cascades only on Low and the GTX 1060.
4. Keep ReSTIR DI (#120) only for point lamps. It is not where the time goes.

## What 4070 measurement confirms each estimate

All runs at 2560×1440 with `--proof`, during a clean-GPU window, with `GENOS_GPU_DISPATCH=1`.

| Estimate | Run that confirms it |
|---|---|
| Pixels vs GI hits split | `light_px` vs `light_hit`, plus the per-dispatch `LIGHTING_DISPATCH` line |
| Sun ray cost | Run with sun shadows off (`GENOS_LIGHTING_SHADOWS=0`, sun only) and compare `light_px`; sun-x100 scene |
| GI hit lighting | Run with `direct=0` and GI on (the kept-hits-only dispatch), and run with GI off |
| Flame | Fire on vs off in the stress building |
| Overhead | The basic scene with no lamps and no sun: what remains is per-point overhead |
| Load inflation | Repeat the 30 Oct numbers in a clean window, and compare with 13.3 ms |
