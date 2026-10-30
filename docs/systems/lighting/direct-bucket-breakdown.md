# Direct lighting bucket: measured breakdown and ranked cuts

Status: for Anthony's review. Replaces the code-reading estimates in the first
version of this note, which the 4070 run disproved.

## What was measured

4070, 30 Oct 16:40 AEDT, lighting-v3 `34cc80cb`, 2560x1440 stress scene,
`GENOS_GPU_DISPATCH=1`, last 30-frame window. The PC was in use, so absolute
times are under load; the differences between rows are what matter.

| Run | light_px ms | light_hit ms | lighting total ms |
|---|---|---|---|
| Base | 14.32 | 0.83 (2.9 first window) | 37.3 |
| GI off | 4.27 | 0 | 4.7 |
| Sun only (no lamps, no sky, noon) | 3.76 | 1.02 | 27.1 |
| Sun only, shadows off | 3.23 | 1.03 | 26.5 |
| No lamps, sun or sky | 2.94 | 0.90 | 23.1 |
| Emissive (flame) off | 14.29 | 0.76 | 37.3 |
| Direct off, GI on | 0 | 9.84 | 32.1 |

`light_px` is one dispatch that shades the screen pixels **and the screen
probes' kept hits**. `light_hit` is the light cache's ray hits and probe fills.

## Corrected breakdown

| Part | Earlier estimate | Measured | From |
|---|---|---|---|
| Point lamps at probe kept hits | (not counted) | ~10 ms | base 14.3 - sun only 3.8, and the lamps cost almost nothing with GI off |
| Point lamps and sun at pixels | 0.3 (lamps) | ~1.3 | GI off 4.27 - nothing lit 2.94 |
| Sun shadow ray | 4-6 | ~0.5 | sun only 3.76 - shadows off 3.23 |
| GI hit lighting (light cache, fills) | 6-8 | 0.8-2.9 | light_hit |
| Fixed per-point overhead | 1.5-2.5 | ~2.9 | nothing-lit row |
| Flame | 0.5-1 | ~0 | emissive off |

The lamps are the bucket, and they cost only where GI is on.

## Where the ~10 ms comes from (code)

All in `lighting_direct.comp` on lighting-v3:

- **Invocations.** The light_px dispatch covers `pixels + kept hits`. Kept hits
  are `probes x rays x LIGHTING_STRATA` = 160 x 90 probes (16 px tile) x 64 rays
  x 8 strata = 7.4 M slots at 1440p. Each frame relights two strata
  (`LIGHTING_KEPT_RELIGHT`: the stratum traced this frame and the one halfway
  round), so about **1.8 M kept-hit points a frame**, half the 3.7 M pixels. The
  halfway relight is skipped only when no moved lamp reaches the hit
  (`lighting_wrc_lamp_changed`), and the stress scene's lamps move, so it
  mostly runs.
- **Lamps per invocation.** Kept hits do not pick. `LIGHTING_PROBE_PICKS` is
  `false` ("S60A: no gain measured"), so `pick` is false, `exact` is true and the
  shade loop walks **every lamp in the hit's lamp list**. Light-cache rays do
  pick, with at most `LIGHTING_CACHE_PICKS` = 2.
- **Shadow rays per lamp.** One `trace_occluded` per lit lamp, unless the S74A
  shadow cell has a stored answer. The cell is used only for a static lamp
  with world mode 3, so moving lamps always trace.
- **Why pixels are cheap and hits are not.** Pixels are coherent, have the S78J
  signature hold (a quiet pixel reuses last frame's sum), and on #120 the
  ReSTIR reservoir. Kept hits land on scattered surfaces in random directions,
  so their lamp lists and shadow rays diverge, and none of those savings apply.
  ReSTIR on #120 (`lamp_pix`) only covers `idx < pixels`. The kept-hit path in
  flag mode 0 is untouched.
- The S69C comment in the shader already records this pass at about 10 ms of
  kept-hit relighting at 1080p, before the two-strata relight cut it down.

## Why the earlier estimates were wrong

- **Point lamps 0.3 ms.** That was the pixel cost with GI off. The kept hits sit
  in the same dispatch and timestamp, so their lamp loop was booked to
  "pixels" and then wrongly put under GI hits.
- **GI hits 6-8 ms.** It assumed the light cache's hits carry the full lamp
  loop. They pick two lamps at most, and `light_hit` measures 0.8-2.9 ms. The
  9.84 ms in "direct off" is the kept hits moving into that bucket, not a real
  hit cost.
- **Sun shadow 4-6 ms.** It priced one incoherent ray per pixel. Sun rays from
  the G-buffer are coherent, and hits outside the sun slice keep the committed
  sun, so it costs ~0.5 ms.

## Ranked cuts

Targets are against ~10 ms of kept-hit lamp work. Quality is judged against the
path-traced reference: photoreal, no look hacks, near-camera error first, no
flicker.

### 1. Sample lamps at kept hits instead of summing them (save est. 6-9 ms)

Give kept hits the same stochastic, light-proportional pick the light cache
already uses: one or two chosen lamps and one shadow ray each, weighted to
stay unbiased. The unshadowed per-lamp weight is cheap. The shadow rays and
divergence are the cost.

- Estimate: shadow rays per kept hit go from the lamp count (several) to <= 2.
  The ~2.9 ms fixed overhead and the unshadowed walk remain.
- Why S60A saw "no gain": it was measured before the kept-hit relight budget
  and two-strata relight, and probably when the loop was not the limiter. It
  needs re-measuring on today's scene.
- Quality risk: low to medium. Kept hits feed the probe gather, which averages
  64 rays x 8 strata, so per-hit noise mostly averages out. The pick offset
  hangs on the hit's 25 cm cell, so a still hit gives the same answer and does
  not crawl. Watch for blotches in indirect light near a bright lamp.
- 4070 A/B: probe picks off/on. light_px, shadow-ray counter slot 49
  (`GENOS_LIGHTING_LAMP_PICK_DIAG`), oracle near/grid on sun-x100, turn, spin
  and switch, and the still-scene unchanged-pixel share.

### 2. Light kept hits from the world-space surface cache (save est. 8-10 ms)

Kept hits read direct light from the surface cache texel under them (the S75
path, as Lumen's final gather reads lit cards) instead of tracing. The cache
already relights texels when a lamp in range changes.

- Estimate: removes the loop and the shadow rays at kept hits, leaving a cache
  lookup. The cache's own relight cost goes up somewhat. That is the light_hit
  bucket to watch.
- Quality risk: medium. Cache texels are coarser than a hit point, and a moving
  lamp's light lags by the cache's relight delay. Indirect light near the camera
  is from close surfaces, where texel size shows most. Needs a check that it
  adds no flicker when lamps move.
- 4070 A/B: kept-hit lighting from the cache vs traced. light_px and light_hit,
  oracle near error (gate: no worse than master), turn and switch.

### 3. Budget kept-hit relights (save est. 2-5 ms)

Relight fewer kept hits per frame, for example only the traced stratum plus
hits a moved lamp really reaches (a tighter `lighting_lamp_delta` threshold), or
a fixed hit budget per frame spent nearest the camera first.

- Estimate: the halfway relight is about half the kept-hit points, so up to
  ~5 ms if it is mostly skipped, and less in scenes with many moving lamps.
- Quality risk: medium. Older kept light lags moving lamps by up to 8 frames,
  which shows as indirect-light ghosting. It must stay near-camera first.
- 4070 A/B: relight budget off/on, light_px, oracle on turn and switch, and
  the lamp-motion lag on spin.

### 4. Reuse the screen reservoirs at hits (save est. <1 ms, low priority)

Most kept hits are off-screen or not at a pixel's surface, so a screen
reservoir rarely applies. Worth trying only after 1 or 2.

## What this means for C5 (ReSTIR, #120)

With the flag off, pixel lamp work is ~1.3 ms with the sun, so screen ReSTIR
cannot pay back its ~10 ms (initial 4.9 + temporal 0.8 + spatial 4.2). Its
value is in scenes with many lamps, and the kept-hit sampler in cut 1 is the
same idea applied where the cost really is. On spin, the still-scene
unchanged-pixel share falls from 0.97 (flag off) to 0.42 (flag on). That points
to reuse picking a different lamp each frame rather than holding one, which
also explains the grid error.

## Update, 30 Oct 18:00 run: the kept hits' shadow rays are not the cost

4070, 30 Oct 18:00 AEDT, 2560x1440 stress, `GENOS_LIGHTING_LAMP_PICK_DIAG=1`.

| Measure | Value |
|---|---|
| light_px, GI on | 14.2-14.7 ms |
| light_px, GI off | ~4.3 ms |
| lamp_px (shadow rays at pixels, one frame) | 17.66 M (~4.8 per pixel) |
| lamp_kept (shadow rays at kept hits, one frame) | ~0.7 M |
| Probe picks on (`LIGHTING_PROBE_PICKS` + `GENOS_LIGHTING_CACHE_PICKS=1`) | saves ~1.5 ms |

This disproves the "1.8 M kept hits x full lamp loop of shadow rays" story
above, and cut 1's 6-9 ms estimate with it. Kept hits trace ~25x fewer lamp
rays than pixels, and picking at them saves only ~1.5 ms. The earlier
sections are kept as written so the history is visible. They were wrong
twice: first blaming GI hits, then blaming kept-hit shadow rays.

### What the code makes certain

- **The counter is honest.** `lighting_direct.comp` increments
  `gstats.diag[48]` (stats word `LAMP_PICK_DIAG_WORD` = 72) once per real
  `trace_occluded` with `idx < pixels`. That covers the sun and every point
  lamp. A shadow-cell hit does not count. Pixels never use the cell
  (`li = LAMP_ALL` for pixels), so every pixel lamp is traced. Slot 49 is kept
  hits, 50 fills and 51 cache rays. The words are one frame's snapshot.
- **No double taps.** The combined dispatch runs once per frame over
  `pixels + 2 * kept` threads (`lighting_dispatch.rs`). A pixel thread shades
  once. 4.8 rays per pixel is about 4-5 lamps in range plus the sun.
- **The pixel path is the same with GI on or off.** The only GI-dependent
  pixel code is S78J pixel reuse (`lighting_pix_reuse_on` needs GI on), which
  only removes work. With movers it is off anyway (`lighting_pix_quiet`).
  The second is reservoir reprojection, which matters only in lamp-pick modes.
  Nothing in the pixel path adds rays when GI is on.
- **`light_px` means different things in the two runs.** With GI on and
  bounce > 0 it times the combined dispatch: pixels plus every kept and fill
  slot. That is 3.7 M + 2 x 7.4 M = ~18.4 M threads. With GI off it is a
  pixel-only dispatch (`ctrl_pixel_only`). The GI-on/GI-off difference is
  therefore the hit half of the dispatch, not the pixels.
- **The "direct off" row already isolates the hits.** With direct off, the
  same kept/fill threads run alone (bias = pixels) under `light_hit`:
  **9.84 ms**. So, to the precision of cross-run subtraction under load:

| Part of GI-on light_px | ms |
|---|---|
| Pixels (17.66 M shadow rays) | ~4.4 (14.3 - 9.8, matches GI off 4.27) |
| Kept + fill slots | ~9.8 |

### Likely cause (code reading, needs a GPU check)

The ~10 ms is the **per-slot work of the kept/fill half of the dispatch,
apart from shadow rays**. It scales with lamps: no lamps gives 2.94 ms total,
sun only 3.76 ms. It is barely touched by picks. Candidates, in order of
likelihood:

1. **The kept-relight gate itself.** All 7.4 M kept slots launch. Each loads
   its hit record and rebuilds its stratum ray (`lighting_stratum_ray`) before
   the S69C stratum test. The ~1.8 M in this frame's two strata then run the
   held-probe checks and `lighting_wrc_lamp_changed` -> `lighting_lamp_delta`.
   That walks the `LIGHTING_WRC_RING` of lamps moved recently, with two
   `length()` calls and a range test each. With moving lamps the ring is full,
   and with no lamps it is empty. That matches the no-lamps and sun-only rows,
   and picks cannot help it. Since only 0.7 M kept rays are traced, most of
   the 1.8 M must leave here or hit shadow cells.
2. **Per-hit lamp bookkeeping that picks keep.** `lamps_at` per hit, and with
   world mode 3 the S75 TDL path (`lighting_cache_find_sized`, a second
   `lamps_at` at the texel, `lighting_tdl_sig`). Then the shade loop's
   `lamp_light` per lamp plus a `lighting_shadow_get` (scattered `cache.v`
   read) for every lamp, and `lighting_shadow_put` writes after traces. The
   pick path still walks every lamp for its weights (the `if (pick)` loop).
3. **Fill hits.** Fills are not under the S69C relight skip. Their traced
   rays are in slot 50, which the 18:00 report did not list.
4. `lighting_cache_irradiance_at` at hits is GI-only but does not scale with
   lamps, so it is bounded by the 2.94 ms no-lamps row. It is unlikely to be
   the 10 ms.

Confidence: high that pixels are ~4.4 ms and do not grow with GI on. High that
the GI-on extra is the kept/fill half. Medium-low on which per-slot work
inside it dominates (1 vs 2 vs 3). Code reading has already been wrong twice
on this pass.

### 4070 A/Bs to confirm (1440p stress, 30-frame windows, same build)

| # | Run | Expect if the hypothesis holds |
|---|---|---|
| A | GI off + `GENOS_LIGHTING_LAMP_PICK_DIAG=1` | lamp_px ~17.7 M, so pixel rays are the same with GI on or off |
| B | Base + DIAG: print all of words 72-75 (px, kept, fills, cache) | fills small; if fills are many M, candidate 3 moves up |
| C | Direct off, repeated at base / sun only / no lamps | light_hit ~9.8 / small / small: the kept-slot cost is lamp-dependent |
| D | Local flip: `lighting_lamp_delta` returns -1 at once (gate always skips), or lamps stopped in the scene | big drop in kept-slot ms = candidate 1 (quality off, timing only) |
| E | Local flip: `LIGHTING_TDL = false` | drop = TDL lookups (candidate 2) |
| F | Local flip: `LIGHTING_SHADOW_CACHE = false` | slot 49 up; ms down would mean the cell reads cost more than the rays they save |
| G | `GENOS_LIGHTING_TOGGLES=direct_only` | bounds the `irradiance_at` share at hits (candidate 4) |
| H | Local diag: `lighting_dg` counters at each early `return` in the kept path (stratum skip, sun hold, relight gate) | how many of the 7.4 M slots get past each gate |

### What changes in the ranked cuts

- **Cut 1 (pick lamps at kept hits):** measured at ~1.5 ms, not 6-9. Keep it
  only as a cheap add-on.
- **Cut 2 (kept hits read the surface cache):** still the right shape,
  because it removes the lamp loop, the cell reads and the TDL lookups at hits.
  The saving is unproven until E/F show where the ms goes.
- **New cut 0:** don't launch threads for kept slots outside this frame's two
  strata. Compact the 1.8 M due hits into an indirect list, or index only
  those strata, and make the relight gate cheap, e.g. one moved-lamp bound per
  probe instead of a ring walk per hit. Size it after D and H.
- **Pixels (~4.4 ms for 17.7 M rays):** the next target after the hits, and
  the place where screen lamp sampling (C5) could pay off. 4.8 rays per pixel
  down to ~2 would be worth ~2 ms if rays dominate the pixel cost.

## Open decision for Anthony

Superseded by the 18:00 update. Run A/Bs A-H first (no rendering change).
Then pick between cut 0 (launch and gate cost) and cut 2 (surface cache at
hits) using whichever of D/E/F moves the kept-slot ms.
