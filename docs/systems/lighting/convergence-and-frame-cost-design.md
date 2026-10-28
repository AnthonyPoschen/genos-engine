# Lighting convergence, sun checkerboard, edge pops and frame cost

Status: **proposed**. ADR: [0021](../../adr/0021-show-converged-light-only.md), proposed. Builds on ADR [0013](../../adr/0013-lighting-invalidation-is-local.md) (budgeted world cache) and the [localized invalidation design](localized-invalidation-design.md). Terms follow [`docs/lighting/GLOSSARY.md`](../../lighting/GLOSSARY.md).

## 0. What Anthony reported (RTX 4070, 28 Oct 2026)

| # | symptom | his words, short |
|---|---|---|
| 1 | objects spinning into and out of light stay patchy for 3–5 s | "the ones that do update quickly go to great looking" |
| 2 | sunrise/sunset: a fixed-grid checkerboard for a few seconds, scene wide (seen at **10x** sun speed) | "likely due to our budget" |
| 3 | whole frame caps near **50 fps** on the 4070 | "this won't run well at all on old hardware" |
| 4 | side pop-ins when turning the camera | still there |

He also said it is much better than last week. The quality rules still hold: no flicker or pops, only finished values are shown, errors near the camera matter most, no look hacks, and the path-traced reference is the judge. Targets: basic scene over 500 fps at 1440p with a moving light, the stress scene around 100 fps, and a Low preset for GTX 1060 / RX 580.

## 1. Measurements

### 1.1 New tools (this PR)

| tool | what it measures |
|---|---|
| `scripts/lighting/grid_metric.py` | Reads a PNG frame sequence. For each frame-to-frame change it computes the **grid score**: the mean absolute gap between each 8 px tile's change and the median change of its 8 neighbours, divided by the frame's mean change. A smooth global change scores about 0. A change where some cells updated and their neighbours did not (the checkerboard) scores above 0.5. It reports the **worst** frame of the run (never an average), the **longest run** of frames above 0.15 (how long a grid stays visible), and **converge frames** (frames until every tile is within 0.02 of the last frame). `--max-grid` and `--max-converge` turn it into a gate. `--self-test` checks the detector on synthetic frames (smooth ramp 0.00, synthetic cell checkerboard 0.92). |
| `examples/stress/scripts/sun_cycle_grid.rhai` | **One full day** (sunrise and sunset included) at **100x, 10x and 1x** sun speed, starting at 05:00, every frame saved. Frames per day = 120 s × 60 / speed + 30: 102 at 100x, 750 at 10x, 7230 at 1x. Frames are saved in 100-frame parts because `flicker` stops after 121 s. **100x is the main repro and the before/after metric**: the sun moves about 3.6° per frame, so the cache is never caught up. 10x is Anthony's setting, and 1x is the shipping speed. |
| `examples/stress/scripts/spin_converge.rhai` | Spins a box by the hall lamps for 240 frames, then stops it and records 360 more frames. It gives the grid score while spinning and the frames the box takes to settle after the stop. |
| `scripts/lighting/sun_cycle_grid.sh <dir>` | Scores everything. The gates are grid ≤ 0.15 at every speed (100x is the stress case) and spin-stop settled within 30 frames (0.5 s). The 1x run compares frames 4 apart, because the sun moves only 0.05° a frame. |

Run (box, lavapipe, headless weston):
```
weston --backend=headless --socket=gl-hl &
WAYLAND_DISPLAY=gl-hl target/release/genos-stress --size 480x270 \
  --script examples/stress/scripts/sun_cycle_grid.rhai --report out/sun
scripts/lighting/sun_cycle_grid.sh out/sun
```

### 1.2 Baseline numbers (master 42231ce, box lavapipe, 480×270, High, stress yard view)

| case | frames | worst grid | at | longest run > 0.15 | verdict |
|---|---|---|---|---|---|
| sun **100x**, full day | 102 | **0.54** | f97 (dusk→night) | 6 frames | grid visible (FAIL ≤ 0.15) |
| sun 100x, `GENOS_LIGHTING_FULL_EFFORT=1` | 89 of 102 (flicker timed out at 1.4 s/frame) | 0.58 | f14 (sunrise) | — | **grid stays with full effort**, so the sun band amortisation is not the only cause |
| sun **10x**, full day | 750 | **0.39** | f45 (sunrise) | 4 frames | FAIL; 14 of the 104 frames with visible change are over 0.15. Dawn worst 0.39, dusk worst 0.19, noon 0 |
| sun 1x, full day | 7230 | pending: about 2 h on lavapipe, run on the 4070 | | | |
| spin box | 600 | see §1.4 | | | |

The worst frames sit at sunrise and sunset in every run. That matches Anthony's report. The frame-delta image at the worst 100x frame shows blocky patches on the building's south wall (cell-sized tiles), not shadow-edge motion.

Caveat: lavapipe frame time is about 1 s, but the scene step is fixed at 1/60 s, so the light schedule per frame is the same as on the 4070. Absolute ms numbers come only from the 4070 (§1.3).

### 1.3 4070 per-pass cost (from `/home/box/shared/ge-4070-verify-28oct.md` and the PR #70 data, 1080p High, stress hall, p50 ms)

| pass | master s1 | #70 s1 (46a688e0) | master s5 | #70 s5 |
|---|---|---|---|---|
| direct | **8.1** | 10.8 | 6.6 | 11.2 |
| resolve | 3.99 | — | — | — |
| à-trous | 3.49 | 1.75 (budgeted) | — | — |
| trace | 2.60 | 1.53 | — | — |
| compose | 2.13 | 3.65→(display crop) | — | — |
| **lighting total** | **18.6** | 19.8 | 16.0 | 20.2 |
| lighting p99 | 31.0 | 28.9 | 29.0 | 29.0 |
| cache entries refreshed / frame | 6948 | 371 (after A1) | 1499 | — |

The whole frame comes to about 18.6 ms of lighting plus about 1–2 ms of the rest, which is ≈ 20 ms, or **50 fps**. That is Anthony's cap. The 8 ms High lighting cap of ADR 0013 fails on master (p99 21–38 ms on s1–s6), and the budget controller sits at its 1/64 floor because it scales only trace and à-trous. Those two are only 6.1 of the 18.6 ms.

### 1.4 Spinning object

Fill from `scripts/lighting/sun_cycle_grid.sh` once `spin_converge.rhai` finishes (see the PR body for the box result).

## 2. Root causes (code)

### 2.1 Patchy spinning objects (issue 1)

1. **A texel needs 8 full sets before it shows its own light.** `LIGHTING_DN_MIN_SETS = 8` (lighting_cache.glsl:1395), `LIGHTING_SC_SET_RAYS = 64` and `LIGHTING_CACHE_RAYS = 16`, so a set takes 4 relights (`LIGHTING_SC_QUARTERS`, lighting_cache.comp:153). That makes 32 relights before a texel is "guided", and the S72K handover fades the coarser prior out over 16 more sets (`LIGHTING_SC_HANDOVER`, glsl:1812). At 60 fps a texel that converges in 3–5 s is relit once every 6–9 frames. That matches the budget: High has 64 k relights a frame against several hundred thousand live entries.
2. **Several gates stretch it further.** Each one decides per texel on a hash, and that per-texel decision is what turns the delay into a pattern:
   - S72O decorrelation sits a refining texel out one round in four (`lighting_compact.comp:332`);
   - the S72J page bar holds texels more than 2 sets ahead of their page (`LIGHTING_SC_PAGE_LEAD = 2`);
   - IC and layer-2 thinning relight flat texels one round in 4 (compact.comp:305, 344);
   - and "unsettled" is a flat ×4 weight (`LIGHTING_SC_UNSETTLED_W`), not ranked by screen size or distance to the camera.

   A spinning box's faces are near the camera and cover many pixels, yet they compete equally with far texels.
3. **A mover's history is cut per texel, not per object.** S62D replaced the history drop with an EMA floor near a change (`lighting_near_change`, cache.comp:213). Texels relit this frame jump to the new light, while their neighbours keep the old value until their turn. The display shows each texel's own state. The coarser parent is mixed in only by the sample count of a young texel, not for a texel that is old but stale. **This is the patchwork Anthony sees.** Values that are not finished are being shown, which breaks the "finished values only" rule.

### 2.2 Sun checkerboard (issue 2)

1. **The sun change is applied per texel at relight time.** On relight, `lighting_cache.comp:178-197` carries the history's sun share to the new sun with `lighting_sc_sun_gain` (glsl:702). That gain is a **scalar luminance ratio** of the sun light then and now (`lighting_ls_ratio`). It fixes sun brightness, but not sun **direction**. A texel whose sun visibility or bounce path changed keeps its old light until the scheduler relights it. At sunrise and sunset, visibility and colour change fastest, so the error between relit and not-yet-relit texels is at its largest. Those texels are scattered by hash and page, which is the grid.
2. **The band amortiser draws the grid explicitly at 1x.** With a step under 2° (`sun_step_small`, lighting_gpu.rs:4321), `note_sun_amortise` holds the committed sun, and each frame relights only the band `row % 8` of probe rows or `page_x % 8` of 2 m page columns (`lighting_sun_pixel_band`, glsl:3103). Bands live for up to 8 frames, against suns committed at different moments.
3. **At 10x and 100x the step is over 2° per update**, so every frame is a "full" light change. Mature texels (`old.w > 0.5`) are relit as the budget allows, and IC records are invalidated per texel by `lighting_sc_global_change` against `LIGHTING_IC_GLOBAL_STALE`. The full-effort run still grids (0.58), which shows that it is the per-texel scheduling plus showing the texel's own stale value, not a lack of rays.

### 2.3 50 fps (issue 3)

The total is fixed per-pixel work that the budget never touches. **direct is 8.1 ms (44%)**:
- direct runs a full shadow ray per pixel per lamp candidate plus the B_HIT cache-ray light, at full resolution over the guard band;
- resolve (4.0), à-trous (3.5) and compose (2.1) are full-res passes;
- only trace and à-trous are controller-scaled.

A frame that "moves" (anything at all moves) runs everything, because the 0.43 ms frozen path is all or nothing (ADR 0013 §1).

### 2.4 Edge pops on turn (issue 4)

- The guard band was cut to 0.08 on High (from 0.15, for cost, #70 2c41cfad), so about 4% of the screen width on each side is pre-lit.
- A fast turn reveals more than that in one frame. The revealed pixels read cache entries that are new: they are spawned at ≤1 per screen tile per frame (ADR 0013 §2, GIBS §1.3).
- Those entries are seeded from a parent only once the parent is mature (#70 A3), and otherwise from the WRC or the ambient fill. When the fine texels then come in over the next frames, the change is visible. That is the pop.
- Screen probes for the new columns have no temporal history, so they also start noisy.

## 3. Proposed fixes

Each fix is measured by the §1 tools. Expected numbers are estimates and are marked so.

### F1: show converged light only: a confidence-weighted fallback chain (issues 1, 2, 4)

What a pixel shows is `mix(prior, own, c)`, where `c` is the texel's **confidence** for the *current* light, not its sample count:
- `c` drops to 0 when a change record touches the texel (mover, lamp, sun direction beyond its gain's validity) and rises with sets taken since.
- `prior` is, in order:
  1. the mature parent level (2×, 4×, bilinear, `lighting_sc_parent_bilerp`) relit for the current light;
  2. a 3×3 same-plane neighbour blend of confident texels;
  3. the WRC;
  4. ambient fill.

This removes the hard per-texel step: a not-yet-relit texel shows the same coarser value as its not-yet-relit neighbours, and the coarse levels are few, so they are relit within 1–2 frames. That is GIBS-style multiscale blending; Lumen does the same with its radiance cache when card pages are stale. No look hack: the prior is the same physical estimate at a coarser size, and the reference check judges it.
- Cost: one more parent read per pixel in compose. Already paid on young texels, so about +0.1 ms (estimate).
- Expected: spin grid score drops from patchy to under 0.15; a mover settles visually as fast as its coarse level (2–4 frames) instead of 3–5 s; the final detail still takes 1–3 s but fades in as a refinement, not patches.

### F2: importance-driven refresh budget (issues 1, 4)

Replace the flat `LIGHTING_SC_UNSETTLED_W` with `w = coverage_px × (1 + k·change) / max(d_cam, 1 m)`. Screen coverage comes from the compose pass's per-entry pixel count, which is already counted for `on_screen` (compact.comp:418). Exempt near-camera texels with low confidence from the S72O, S72J, IC and layer-2 skip gates, since those gates were written for still refinement. Keep the 1-in-N starvation slice (ADR 0013 §3).
- Expected: near-camera mover faces get relit every 1–2 frames instead of every 6–9, so convergence takes 3–5× less time (estimate).

### F3: sun change is a global low-frequency update (issue 2)

- (a) Split stored light into **sun-direct-bounce**, **sky** and **local**. That extends the existing S63A/S64B sun share (`LIGHTING_SC_SUN_AT`) from a scalar fraction to a low-order directional term: 2nd-order SH of the incoming sun-path bounce per coarse cell (L2 = 4 m), relit for the whole coarse level each frame the sun moves.
- (b) Fine texels carry their sun share as a ratio to the coarse cell. A sun move then updates every texel in the same frame (coarse relight × stored ratio), and per-texel rays only refine the ratio.
- (c) Delete the 8-band row/page amortiser. The coarse relight replaces it, and bands are a grid by construction.
- (d) The sun's direct light already comes from fresh shadow rays per frame. Only the bounce goes through the cache.
- Cost: L2 coarse cells under the 24 MB High coarse cap; about 4 k cells × 16 rays ≈ 64 k rays per sun frame, ≈ 0.3 ms on a 4070 (estimate, from the trace pass's 2.6 ms for 256 k cache rays plus probes).
- Expected: the 100x worst grid drops from 0.54 to under 0.15; at 10x and 1x there is no visible grid.

### F4: pre-warm in the direction of rotation (issue 4)

- Make the guard band asymmetric. Put the 0.08 on the side the camera is turning toward, scaled by angular velocity: `band = clamp(ω·Δt_converge / fov, 0.04, 0.25)` on the leading side and 0.02 on the trailing side. The total area stays about the same as today's symmetric 0.08.
- Spawn and pre-fill entries for the leading band with the spawn throttle lifted for that band only.
- Reuse F1's coarse prior for any pixel the band did not cover, so a reveal shows the coarse level (finished) rather than WRC or ambient.
- Expected: turn pop below the 16 limit in `a_turn_toward_the_lamp_does_not_pop_the_bounce` at no extra cost.

### F5: frame cost: from about 20 ms to the targets (issue 3)

Expected ms are estimates scaled from the measured passes, on a 4070 at 1080p High, stress hall s1:

| cut | today | after | how | risk |
|---|---|---|---|---|
| direct at the display crop + ReSTIR reuse, fresh shadow rays budgeted by near-camera importance (ADR 0013 §3 table: High 2 M px-equiv) | 8.1 | **3.5** | #70 A2 already gave 15.7→10.8 for the crop; the rest is reusing visibility for pixels whose lamp and geometry did not change (Item B, dynamic-perf-plan) | shadow lag on movers; judged by gpu_frame tests |
| resolve at half res + joint-bilateral upsample (not the denoiser: S79 rejected half-res denoise) | 4.0 | **1.3** | quarter of the pixels; the upsample is guided by depth and normal | edge bleed; reference check |
| à-trous: 3 → 1 iterations on High, since F1 feeds already-converged values | 3.5 | **1.2** | the denoiser filters young texels less once F1 hides them | noise; noise_gate |
| compose merged into resolve (one full-screen read) | 2.1 | **1.0** | removes one full-res read and write | — |
| trace and cache on async compute, overlapping G-buffer and raster | 2.6 | **~1.0 visible** | NVIDIA has a separate compute queue (the #93 lesson: check queue caps) | sync bugs; lavapipe cannot test overlap |
| **lighting total** | **18.6** | **≈ 8** | | |

- Stress scene at 1080p: about 8 + 2 ≈ 10 ms, about **100 fps** (target).
- At 1440p, the per-pixel passes scale ×1.78, giving about 12 ms.
- The **basic scene** with one moving light at 1440p reaches 500 fps (2 ms) only if the per-pixel GI is skipped where nothing changed. That needs the frozen path made local: reuse the previous frame's lighting for pixels whose change record is empty, which is ADR 0013's scheduler applied to screen pixels.
- **Low preset (GTX 1060 ≈ 0.35× a 4070):**
  - drop direct to 1 lamp candidate;
  - resolve at half res, 1 à-trous iteration, no async;
  - cache refresh at shift 2.
  - Target 4 ms lighting at 1080p on a 4070, which is about 11 ms on a 1060.

Order: F1, then F3, then F2, then F4, then F5. F1 is the prerequisite: it makes every budget cut safe, because under-refreshed texels show a finished coarse value instead of patches.

## 4. Verification (for agents)

| fix | gate | today | target |
|---|---|---|---|
| F1, F3 | `sun_cycle_grid.sh`: worst grid, 100x full day | 0.54 | ≤ 0.15 |
| F1, F3 | same, 10x / 1x | 0.39 / pending | ≤ 0.15 / ≤ 0.15 |
| F1, F2 | `spin_converge.rhai`: grid while spinning, frames to settle after the stop | §1.4 | ≤ 0.15, ≤ 30 frames |
| F4 | `a_turn_toward_the_lamp_does_not_pop_the_bounce` (gpu_frame, 4070) | 13–21 | < 16 on 3 salts |
| F5 | stress bench p50 / p99 lighting, 4070 1080p High | 18.6 / 31 | ≤ 8 / ≤ 10 |
| all | `crates/lighting-ref` reference check | per scene | no regression |

All gates run on lavapipe except F5 and the 4070 gpu_frame test.

## 5. Not done here

- No engine code changes. F1 to F5 each touch the shader schedule and need 4070 gates. None of them is small and clearly correct on lavapipe alone, so none is implemented in this PR.
- 1x full-day and spin numbers on the 4070 are pending the next clean-GPU window.
