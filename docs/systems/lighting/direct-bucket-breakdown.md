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

## Open decision for Anthony

Order of work: cut 1 first (smallest change, biggest measured target), then
cut 2 if the 4070 A/B leaves the kept hits above ~2 ms. Cut 3 only if both fall
short.
