# 0037: The world radiance cache relearns when a lamp's power changes

Status: **Proposed** (needs Anthony's review before any renderer code).

## Context

On lighting-v3 (34cc80c), with the world radiance cache (WRC, world mode 3) on, bounce light remembers the old lamp state after lamps switch. The effect works in both directions:

- **Off → linger.** Lamps go off, but their bounce stays.
- **On → stays dark.** Lamps come on, but bounce stays near the dark value.

master (0140486) doesn't have the bug: bounce drops to 0 the moment the lamps go off.

The scenes are stress/hall with `lights 32`, 22:00, sun frozen, camera held, 160×90 lavapipe. They live in `/tmp/linger/*.rhai`. The table shows the mean bounce view in linear luma.

**Switch-off** (`dark.rhai`: 300 frames dark, 300 frames lit, then off). "Lingering" is the share of the lamp-added bounce still there after switch-off.

| Run | Never lit | Lit | Off 30 | Off 300 | Off 1200 | Lingering (30 / 300 / 1200 frames) |
|---|---|---|---|---|---|---|
| master 0140486 | 0 | 0.1031 | 0 | 0 | 0 | 0 / 0 / 0 % |
| lighting-v3 34cc80c | 0.0160 | 0.0406 | 0.0293 | 0.0254 | 0.0238 | 54 / 38 / 32 % |
| v3 + off lamps recorded in the lamp-change ring | 0.0169 | 0.0418 | 0.0300 | 0.0267 | 0.0248 | 53 / 39 / 31 % |
| v3 + camera nudged every 30 frames (no settle) | 0.0169 | 0.0400 | 0.0289 | 0.0250 | 0.0220 | 52 / 35 / 22 % |
| v3, world cache off | 0.0068 | 0.1442 | 0.0099 | 0.0099 | 0.0099 | 2 / 2 / 2 % |

**Switch-on** (`onramp.rhai`: 300 frames dark, then on). "Indirect" is live minus the direct capture.

| Frames after on | 60 | 300 | 600 | 1200 | 2400 | Lit from the start (600 frames) |
|---|---|---|---|---|---|---|
| bounce | 0.028 | 0.041 | 0.041 | 0.043 | 0.047 | 0.146 |
| indirect | 0.082 | 0.110 | 0.111 | 0.114 | 0.122 | 0.241 |
| bounce, settle disabled (`light_quiet >= 100000`) | 0.025 | 0.039 | 0.040 | 0.043 | 0.046 | |

### Cause

The stale light is held in the WRC cells' running mean (`lighting_cache.comp`, the mode-3 hit-patch branch).

- A cell sums samples up to `LIGHTING_WRC_CAP` (128). The weight of a new sample is `add / (t + add)`.
- That weight is raised to `LIGHTING_WRC_LOCAL_ALPHA` (1/8) only for cells near a recorded change.
- It is raised to `LIGHTING_WRC_CHANGE_ALPHA` (1/32) only while `!lighting_sc_light_quiet()`, a window of a few rounds after the light changes.
- After that window, a full cell moves about 1/128 of the way per relight. Each value it reads back through other cells also lags, so the bounce converges at a crawl, and the trend flattens toward the old value.

Four things that are **not** the cause, from the tables:

- **The settle/freeze rule.** Disabling it changes nothing (switch-on table, last row).
- **Camera stillness.** Nudging the camera barely helps.
- **The lamp-change ring missing off lamps.** `lighting_wrc_note_lamps` only records lamps where `moved && l.color.w > 0.0`, so a lamp going to colour 0 is never recorded. This is a real gap, but recording off lamps alone changed nothing measurable.
- **A π, cosine or irradiance-versus-radiance error.** The cell estimate is E = 2π/N Σ L cos θ over uniform-solid-angle directions (`lighting_hemi_dir` draws cos θ uniformly), which is correct. The hit returns `albedo/π · E` (`lighting_direct.comp` ~958, `reflectance = albedo / π`), also correct.

### Energy loss (the "3.5× darker" and "6–10 % dark")

The 3.5× gap (0.041 against 0.144) is **this same history lag, not an energy-loss term**. It was measured 300 frames after switching on from dark.

When the scene is lit from the start and settled for 600 frames, against a 64-bounce reference (`energy.rhai`, indirect = b64 − b0 reference = 0.2778):

| | direct / ref | indirect (live − direct) | indirect / ref |
|---|---|---|---|
| WRC on | 0.997 | 0.2407 | 0.87 |
| WRC off | 0.997 | 0.2621 | 0.94 |

So the WRC on its own costs about 7 % of indirect light at convergence, on top of about 6 % that both paths share. I found no single clear wrong term (no π, cosine or double factor). The remaining suspects are coarse hit-patch size and normal-bucket leak at corners, and cells read before they are full. They need their own measurement, and this ADR does not decide them.

## Options

- **(a) Relearn on lamp power change (recommended).**
  - Record a lamp in the change ring when its power changes, including going to 0. For a lamp that went off, record its old range and power.
  - Every WRC cell inside a recorded lamp's old or new sphere then drops its history to a small count, as ADR 0013's local invalidation does for movers. It re-gathers at the local rate.
  - This removes both the linger and the switch-on dark, with no global effect.
  - Cost is bounded by the lamp's sphere and by the existing per-preset relight cap: a worst-case switch event defers work, it doesn't exceed the frame budget.
  - Until the cells refill, they show their history scaled toward the parent or probe value, so there is no black pop.
- **(b) Fast blend or reset only where incoming light drops sharply.** Cheaper, but it is driven by noisy per-cell estimates. It risks flicker and still misses slow drift.
- **(c) Settle only after the running averages converge.** Require cache rounds that scale with the time the cells need to converge after a light change before mode 3 counts as settled. This is useful robustness, but the switch-on test shows the settle rule is not the cause, so it fixes nothing by itself.

## Interaction with existing decisions

- **ADR 0013 (local invalidation, lamp-change tracking, #59).** Option (a) extends the same ring. A power change becomes a change record like a move. No full-frame fallback, and work stays under the caps.
- **Anthony's rules:**
  - No ghosting: the linger is ghosting, and (a) removes it.
  - No pops: cells blend from history, not from 0.
  - Worst-case light-switch events hit the frame budget: the work is capped and deferred, never forced.
  - Full-relight cost matters most: (a) re-gathers only the cells inside the switched lamps' spheres, never the whole cache.

## Gate

A test in `crates/render/tests/lighting_frame.rs` on the lighting-v3 High preset, Bounce view: hall, 32 lamps, 22:00, held camera.

- **Off case.** After the lamps switch off, the lamp-added bounce left must be ≤ 5 % by 120 frames, where it is 54 % at 30 frames today.
- **On case.** After switching on from dark, bounce must reach ≥ 90 % of the lit-from-the-start value by 300 frames, against 28 % today.
- The worst-case switch frame must stay within the preset's frame budget.

The energy number (indirect / 64-bounce reference ≥ 0.95) is tracked separately.

## Remaining energy gaps (diagnosis, lighting-v3 34cc80c)

**Measurement fix first.** `oracle_metrics.lin_luma` decoded the PNGs as sRGB. They actually store the shoulder tone, which is linear up to 0.64. The fix is PR #148, with a test and the fix as separate commits. With the right decode:

- The bounce view equals live − direct (0.386 against 0.385). The earlier 0.146 against 0.241 was entirely the wrong decode.
- The real gaps are larger than reported: indirect / 64-bounce ref is **0.77** with the WRC on and **0.88** with it off. The earlier figures were 0.87 and 0.94.

Every number below uses the corrected decode. The setup is hall, 32 lamps, 22:00, settled, held camera, 160×90 lavapipe, with references b0 and b64. Each row changes one toggle; the hooks are local only and nothing is committed.

| Toggle | WRC on @600 | WRC on @1800 | WRC off @600 / @1800 |
|---|---|---|---|
| baseline | 0.776 | 0.788 | 0.883 / 0.883 |
| cells read only at ≥ 64 sets (`LIGHTING_SC_READ_SETS` 4 → 64) | 0.778 | 0.789 | 0.888 / 0.888 |
| fixed 25 cm hit patches (`SC_FLAGS` 256) | 0.740 | 0.752 | 0.801 / 0.801 |
| no cache light at hits, one bounce (`SC_FLAGS` 2097152) | 0.321 | 0.322 | 0.333 / 0.333 |

All values are bounce / reference indirect. Direct / ref is 0.997 in every run.

**What these rule in and out:**

- **Cells read before they fill:** ruled out (+0.2 %).
- **Max ray distance:** ruled out. The trace limit is 200 m, more than the hall.
- **Coarse hit patches:** do not cause the loss. Forcing coarser 25 cm patches costs about 4 % (WRC on) and 8 % (WRC off), so patch size matters, but the shipped sizes are already the better case.
- **Slow convergence:** the WRC on its own still gains about 1 % between 600 and 1800 frames, so part of its 11 % is slow convergence (the history lag above).
- **Where the shared 12 % sits:** one bounce gives 0.33 of the reference indirect, and the cache's later bounces add 0.55, where the reference's later bounces give 0.67. So the shared 12 % sits in the multi-bounce chain through the light cache, not in the first bounce. That points at albedo or energy per cache hop, or truncation by the cache's own bounce depth. The direct term matches, and the first-bounce estimator was checked correct above.

**Not measured in this pass:**

- Normal-bucket leak at corners.
- Probe ray count and termination.
- Albedo per hop: the next toggle is to scale the hit albedo and see whether the gap scales like a geometric series.
- Sky and ambient: at 22:00 the sky is near 0, so they are unlikely to matter here.

### Per-bounce series (lighting-v3 34cc80c, 31 Oct 2026)

Stress hall, lamps lit, 160x90, live settled 900 frames, tone-inverse decode. Each value is a fraction of that row's 64-bounce reference indirect. `k` scales the albedo of every secondary hit, both live (`reflectance` in `lighting_direct.comp`) and in the reference (`path_from`). These were local debug hooks and were not committed.

| k | ref b1 | ref b2 | ref b4 | ref b8 | ref b64 (abs) | live 1-bounce | live WRC off | live WRC on |
|---|---|---|---|---|---|---|---|---|
| 1.0 | 0.335 | 0.504 | 0.741 | — | 0.4986 | 0.333 | 0.881 | 0.773 |
| 0.9 | 0.416 | 0.604 | 0.834 | 0.969 | 0.3615 | — | 0.928 | 0.816 |
| 0.5 | 0.707 | 0.884 | — | — | 0.1183 | — | 0.988 | 0.901 |

**Findings:**

- **First bounce is exact.** Live one-bounce is 0.333 and reference b1 is 0.335. The whole shared gap is in hops 2 and later.
- **The hall has high loop gain.** The reference fits I(k) = c·k/(1 − g·k) with c = 0.156 and g = 0.68, which predicts 0.486 at k = 1 against the measured 0.499. Raising k from 0.9 to 1.0 grows the indirect by 38 %, so a small per-hop loss is amplified.
- **A constant per-hop loss fits the data.** WRC off fits the same form with g·f = 0.645, which gives f ≈ 0.95, a loss of about 5 % per later hop. That predicts 0.335 at k = 0.9 (measured 0.335) and 0.115 at k = 0.5 (measured 0.117).
- **A hard truncation also fits.** Truncating at about 6–7 hops would match k = 1 and 0.9 as well. The data cannot separate the two models, but no hard depth limit is needed. The 1.2 % that is still missing at k = 0.5 leans slightly towards the per-hop loss, because truncation at 6 hops would predict about 0.998.
- **WRC on costs a further 9–12 % at every k**, including 3.6 % at one bounce. It is not mainly hop-dependent, which matches the history lag noted above.
- **Ruled out by reading the code:** rgb9e5 packing (it rounds to nearest, so it is unbiased), and the hit reflectance chain (tint·0.8/π is stored as ×π in unorm8, then read as /π, which matches the reference's LAMBERT).
- **Still not measured:** the source of the ≈5 % per hop, with cache cell normal or position interpolation as the lead suspect, and a corner-free open-plane comparison.
