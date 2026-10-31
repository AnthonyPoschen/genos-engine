# 0037: The light cache stops before the light converges

Status: **Proposed** (a design for Anthony to review; no renderer code yet).

Earlier rounds of this ADR were titled "The world radiance cache relearns when a lamp's power changes". The investigation log is kept in the appendix. Some of its "not the cause" findings were measured with the surface cache off (mode 0), so they do not hold for mode 3. Round 6 explains this.

## Decision summary

Three symptoms share one root cause:

- lamp-off linger
- lamp-on lag
- the settled energy shortfall (box 0.58 of reference, hall 0.77)

The cause is the surface cache's convergence and staleness policy (world mode 3). Each of its four rules stops or slows relighting before the multi-bounce light has converged:

1. **Settled freeze** (`lighting_gpu.rs`, `frozen = lighting_stats().settled`). About 300 frames after the camera stops, light-cache rounds stop altogether. The freeze is set by a still-frame count, not by whether the light has converged.
2. **Young-only rounds** (`lighting_gpu.rs`, `young_only` when `light_quiet >= 2`). Once the light is "quiet", only young patches relight. Cells at the 128-set cap and older texels never refresh.
3. **Capped running mean** (`lighting_cache.comp`, mode-3 hit-patch branch). A full cell weighs a new sample at about 1/128, and its mean keeps the dark sets from early rounds when fewer bounces had arrived. Every later hop reads that lagging value.
4. **Lamp power change not recorded** (`lighting_wrc_note_lamps` records only lamps with `color.w > 0`). A lamp switching off never triggers the fast local alpha.

The proposal is one policy. Keep relighting until a measured convergence test passes, use a mean that forgets, and invalidate on lamp changes.

## Proposed design

1. **A convergence test replaces the still-frame freeze.**
   - Each relight already computes the change between the new sample and the stored mean (`lighting_wrc_check_take`).
   - Keep a per-round reduction: the share of relit cells whose relative luma change is above a threshold ε, for example 2% after noise allowance.
   - The cache counts as settled only after K consecutive rounds (for example 8) with that share below a small fraction, for example 1%.
   - Only then may the freeze and young-only rounds start.
   - Any lamp, sun or geometry change clears the settled state, as camera motion does today.
2. **A mean that forgets.**
   - Replace the plain mean capped at 128 sets with an exponential mean: α = max(1/n, α_min), with α_min about 1/32, and a 1/8 floor for the first rounds.
   - Also reset the count (t → 4) when the incoming sample's luma exceeds the mean by more than X (10% tested best). This happens when more bounces arrive or a lamp turns on.
   - The symmetric case, a sample well below the mean (lamp off), should reset as well. This is to be tested with the linger scenario.
3. **Lamp-change invalidation.**
   - Record lamps whose power drops to 0 in the lamp-change ring.
   - Cells within a changed lamp's reach take the local alpha or the reset above.
4. **Keep the idle saving.**
   - Once the test passes, freeze exactly as today. Idle cost then returns to the frozen ~0.1 ms.
   - Extra work is spent only while the light is actually changing.
   - This needs measuring: how many rounds the box and hall take to pass the test, and the cost per round until then.

Each step needs a failing test first, on the scenes below, before any code.

## Evidence

### Patch probe, mode 3 (thin-wall box, albedo 0.8, frame-880 dump)

The table compares each patch's stored E with a CPU 64-bounce reference irradiance at the same centre and normal.

| Patch | Set count w | E / ref | Last relight |
|---|---|---|---|
| 0.125 m texels | ≈22 | 0.54–0.69 | ≈ frame 300 |
| 0.25 m cells | 128 (cap) | 0.70–0.84 (mostly ≈0.82) | ≈ frame 216 |
| 0.5 m cells | 9–21 | ≈0.51–0.61 | ≈ frame 5 (no longer read) |

The dumps at frame 880 and frame 2680 are identical: nothing relights after about frame 300. The samples themselves are low, and the cache is frozen at an under-converged fixed point.

### Policy toggles, thin-wall box (160×90 lavapipe, #148 tone decode, 64-bounce ref)

> **Superseded (round 7, inflated).** This build carried `DBG_PATCHVIEW = true` (see the round 8 correction). The "best" numbers here are not current. Use the round 8 and round 9 tables.

| Run | live/ref, 900 frames | f, 900 | live/ref, 2700 frames | f, 2700 |
|---|---|---|---|---|
| default (34cc80c) | 0.581 | 0.781 | 0.583 | — |
| stale check off | 0.564 | — | 0.567 | — |
| no freeze | 0.638 | 0.828 | 0.647 | 0.834 |
| no freeze + no young-only | — | — | 0.686 | 0.861 |
| + exponential mean (α floor 1/8 under 64 sets, then 1/32) | 0.696 | 0.867 | 0.707 | 0.874 |
| + count reset when sample > mean +10% | **0.740** | **0.893** | **0.741** | **0.894** |
| + count reset when sample > mean +3% | 0.708 | 0.875 | 0.704 | 0.873 |

The reset rule wins. 3% fires on noise and loses history. The remaining gap of about 0.26 is mostly in the 0.125 m texels, whose own mean (`LIGHTING_CACHE_MATURE`) was not changed in this round.

### Hall and lamps-spin, best combination (no freeze + no young-only + reset at 10%)

> **Superseded (round 7, inflated).** This build carried `DBG_PATCHVIEW = true` (see the round 8 correction). The "best" numbers here are not current. Use the round 8 and round 9 tables.

The hall is stress hall with 32 lamps, 22:00, sun frozen and the camera held. Indirect/ref is measured at 1140 frames. For lamps-spin, lamps are set to `move` after settling, and the near-weighted error (oracle `ref_err_near` weighting, #148 decode) is taken against a 64-bounce reference every 60 frames.

| | default | best |
|---|---|---|
| hall indirect / ref | 0.768 | **0.966** |
| hall direct / ref | 0.997 | 0.997 |
| lamps-spin near error, frames +60 / +120 / +180 | 0.124 / 0.118 / 0.110 | **0.054** / — / — |

The best run hit the 590 s harness timeout after the first lamps-spin frame, because lavapipe is slow without the freeze. Its lamps-spin figure is one frame.

### Idle cost (hall, still camera, 300 frames after settling, lavapipe)

> **Superseded (round 7, inflated).** This build carried `DBG_PATCHVIEW = true` (see the round 8 correction). The "best" numbers here are not current. Use the round 8 and round 9 tables.

| | lighting ms p50 | p90 |
|---|---|---|
| default (frozen) | 0.10 | 0.13 |
| no freeze + no young-only | 289.8 | 462.6 |

Never freezing is not acceptable. On lavapipe it costs about 290 ms per frame (round 8 clean hooks: 323 ms). GPU numbers are not measured, and they are expected to be a far smaller absolute cost. This is why the design keeps the freeze and changes only *when* it starts: after the convergence test passes, rather than after a still-frame count. The next measurement is how many rounds that takes.

## Round 8: clean hooks, convergence test, linger and lag (31 Oct 2026)

### Correction to round 7

The round-7 build still carried an old local hook, `DBG_PATCHVIEW = true` in `lighting_compose.comp`. It made pixels read the patch irradiance in place of the probes, so the "best" box, hall and lamps-spin numbers above are inflated. The idle-cost numbers are not affected.

Round 8 uses only the env-gated hooks in `/home/box/shared/adr0037-hooks.patch`:

- It is a patch against 34cc80c.
- It contains `GENOS_DBG_NOFREEZE`, `GENOS_DBG_NO_YOUNG`, `GENOS_DBG_MEAN` (1 = exponential, 2 = count reset) and `GENOS_DBG_RESET_PCT` (0 means 10).
- It adds convergence counters, read with `GENOS_LIGHTING_SC_TRACE`.
- Every hook defaults to the shipped behaviour.
- It applies cleanly to a fresh checkout of 34cc80c and builds.

"Best" below means NOFREEZE + NO_YOUNG + MEAN=2 with the 10 % reset.

| | default | best |
|---|---|---|
| box live/ref, 900 frames | 0.562 (f 0.764) | **0.694** (f 0.866) |
| hall indirect/ref, 1140 frames | 0.765 | **0.844** |
| hall still lighting ms p50 / p90 (lavapipe) | 0.09 / 0.12 | 323 / 465 |

The policy change is still the largest single gain, but it closes about a third of the shortfall, not the 75–85 % that round 7 reported.

### Convergence test

Each round counts the WRC cells it relit and the share whose stored value moved by more than ε. The table asks when 8 consecutive rounds first have that share below 1 %.

| scene | ε = 1 %: K = 8 passes at | ε = 0.5 %: passes at | share > 1 % over the run |
|---|---|---|---|
| box (1 lamp) | round 122, frame 126 | round 129, frame 133 | ≈1–2 %, with spikes to 7.5 % |
| hall (32 lamps) | never (1348 rounds) | never | ≈70–80 % every round |

- **A raw per-cell ε test does not work.** In the hall the 16-ray samples are noisy enough that most cells move by more than 1 % on every relight, so the test never passes and the cache would never freeze. In the box it passes at frame 126, long before the box stops brightening (live/ref keeps rising until frame 900), so it would freeze too early.
- **The test has to be relative to noise.** The design should compare each cell's change with its own noise estimate (`lighting_wrc_check_take` already keeps a per-cell σ in GRAD2.w), or test the drift of the round's mean luma over K rounds instead of single cells.
- **Extra unfrozen time and the real idle cost are still open.** They depend on that redesign.

### Lamp-off linger and lamp-on lag (hall, 32 lamps, 22:00, bounce view, #148 decode)

**Linger** (`dark7.rhai`): share of the lamp-added bounce still present after switch-off. The gate is 5 % or less by 120 frames.

| | off 30 | off 120 | off 300 |
|---|---|---|---|
| default | 67 % | 57 % | 47 % |
| best | 92 % | **91 %** | 90 % |

**Lag** (`onramp7.rhai`): share of the settled lit bounce reached after switch-on. "Settled" is the same view lit for 1140 frames. The gate is 90 % or more by 300 frames.

| | on 60 | on 120 | on 300 | on 600 |
|---|---|---|---|---|
| default | 18 % | 25 % | 33 % | 34 % |
| best | 46 % | 71 % | **83 %** | 86 % |

- **Lag improves a lot but misses the gate** (83 % at 300 frames against 90 %).
- **Linger gets worse.** The upward-only reset and the never-ending relights keep the cells near the lit value. Nothing makes a cell forget when its sample drops. This confirms that the design needs the downward reset and the lamp-off entry in the lamp-change ring. Those are untested.

### Changes to the proposed design

1. Run the convergence test against per-cell noise σ, or as round-mean drift, not as a raw ε.
2. Make the count reset symmetric (sample far below the mean), and record lamps that switch off in the change ring. Both are required by the linger gate.
3. The next measurements are the rounds needed to pass the noise-relative test, and the 4070 idle cost, using the hooks patch.

## Round 9: two-sided reset, lamp-off ring, noise-relative test (31 Oct 2026)

Build: lighting-v3 34cc80c plus `/home/box/shared/adr0037-hooks.patch` (regenerated). `DBG_PATCHVIEW` and `DBG_WHOLE` are not in the tree. New hooks, all off by default:

- `GENOS_DBG_MEAN=3`: two-sided count reset. When |sample − mean| > max(10 %, k·σ)·mean, the count drops to 4. σ is the per-cell relative noise in GRAD2.w. k comes from `GENOS_DBG_K` (default 3).
- `GENOS_DBG_LAMPOFF=1`: `lighting_wrc_note_lamps` also records lamps that were lit and are now off, at their previous power. A WRC cell inside the old range sphere drops its count to 1.
- `GENOS_DBG_CONV=1|2`: the freeze also waits for a convergence test. Mode 1 passes when, for 8 rounds in a row, under 1 % of relit cells have |change| > 3σ/√n. Mode 2 passes when the round-mean luma of relit cells is within 0.5 % of its value 8 rounds earlier, for 8 rounds in a row. The test resets on camera or light motion.

"k3" below means CONV=1 + NO_YOUNG + MEAN=3, K=3 + LAMPOFF.

### Correction to round 8

(Round 10: the replacement words 56–63 were also in use. See the round 10 counter fix. The round-9 hall share columns are invalid too.)

The round-8 convergence counters wrote to `chg[12..17]`. Those slots already hold `CHG_G_HELD`, `G_DIRTY`, `G_MOVED`, `G_PICKS`, `TEX_WRITE` and `TEX_D0`, so **the round-8 convergence table is invalid** (shares above 1 were seen). Round 9 counts in the unused `diag[56..63]` words, and only while `GENOS_LIGHTING_SC_TRACE` is set.

### Results

| | default | k3 | k4 |
|---|---|---|---|
| box live/ref, 900 frames | 0.563 (f 0.764) | 0.566 (f 0.768) | 0.562 (f 0.763) |
| box: noise test (mode 1) passes at | n/a | round 18, frame 22 | round 18, frame 22 |
| hall indirect/ref, 1140 frames | 0.765 | **0.820** | not run |
| hall: noise test passes at | n/a | never (1377 rounds) | not run |
| hall: round-mean drift 0.5 % / 1 % / 2 % (offline) | n/a | never / never / never | not run |
| hall still lighting ms p50 / p90 (frames 840–1140) | 0.09 / 0.12 | 232 / 295 (never froze) | not run |
| hall still flicker, per-pixel frame-to-frame luma std | 0 | 0.00000 (mean-luma std 2e-6) | not run |
| linger, off 30 / 120 / 300 (gate ≤ 5 % at 120) | 65 / 56 / 46 % | 90 / **70** / 43 % | not run |
| lag, on 60 / 120 / 300 / 600 (gate ≥ 90 % at 300) | round 8: 18 / 25 / 33 / 34 % | 29 / 50 / **75** / 78 % | not run |

Box drift (offline): within 2 % at frame 36 in every variant, and within 1 % only after frame ~930 for k3 and k4. The default never gets within 1 %.

- **The noise-relative test passes too early in the box.** At frame 22 only the first relights have happened and the light is still rising. With k3 and the conv gate, the box ends no brighter than the default (0.566 against 0.563). In the box the k = 3 and k = 4 results are the same within noise.
- **In the hall it never passes.** A share of 0.6–0.8 of relit cells stays above 1σ/√n every round. The stored σ is capped at 0.2, and that cap looks too low for 16-ray hall samples. So the cache never freezes and the idle cost stays near the no-freeze cost (232 ms against 323 ms in round 8). Flicker is not the issue: the frames are stable. The cost is.
- Counter caveat: the hall `3σ` share exceeds 1 once at round ~232, so one counter is still double-counting there. Treat the hall share columns as approximate.
- **Linger still fails.** The lamp-off ring with a reset to 1 still leaves 70 % at 120 frames. Lag improves from 33 % to 75 % at 300 frames, which is still short of 90 %.
- **Hall indirect** rises from 0.765 to 0.820, below round 8's NOFREEZE best of 0.844.

### Changes to the proposed design

1. Neither test is a usable freeze trigger yet. A per-cell noise test needs a real σ (uncapped, or estimated from the ray count), and it needs a minimum sample count before it can pass. Round-mean drift over relit cells is too noisy, because each round relights a different subset.
2. The linger failure is not explained by the cell count alone. The next step is to check whether the hall bounce view reads the texel cache (`LIGHTING_CACHE_MATURE`), which neither hook resets.

## Round 10: texel reset, counter fix, real noise, trend term (31 Oct 2026)

Build: 34cc80c plus the regenerated `/home/box/shared/adr0037-hooks.patch`. New hooks, all off by default, and all packed with mean mode 3 (head bits 18–20 k, 21 TEXOFF, 22 SIGCAP):

- `GENOS_DBG_TEXOFF=1`: a surface-cache texel inside a switched-on or switched-off lamp's range sphere restarts its mean from the current set. It applies before every texel estimator path.
- `GENOS_DBG_SIGCAP=1`: the stored WRC noise cap goes from 0.2 to 1 (d is ≤ 1 already). It is used by `lighting_wrc_check_take`, by the mode-3 reset and by the counters.
- `GENOS_DBG_CONV=3`: the mode-1 noise test, plus at least `GENOS_DBG_CONV_MIN` rounds (default 64), plus a summed signed mean step of relit cells under 0.5 % over the last 16 rounds.

"r10" below means CONV=3 + NO_YOUNG + MEAN=3, K=3 + LAMPOFF + TEXOFF + SIGCAP.

### Counter double-count (fixed)

Round 9's `diag[56..63]` words are not free:

- `lighting_direct.comp` adds to 57–59 under `lamp_pick_diag()`. Its restir count in 59 is the "3σ share above 1" double count.
- `lighting_screen.comp` adds to 61–63, and `screen_publish` *reads* 61 and 62 for the screen à-trous change fraction. So round 9's trace-on runs also nudged the à-trous fraction, both default and k3. The default numbers still matched round 8 (box 0.563, hall 0.765).

Round 10 counts only in words that nothing writes outside `lighting_dg` (which is off): 0 1 5 6 8 13 14 18 21–27 32 34 38 39 60. With the fix the hall share > 3σ/√n is 0.57–0.62, never above 1.

### Per-sample noise (WRC cells with 8+ sets, |sample − mean| / max)

| scene | < 0.05 | 0.05–0.1 | 0.1–0.2 | 0.2–0.4 | 0.4–0.8 | ≥ 0.8 | rms | share above the 0.2 cap |
|---|---|---|---|---|---|---|---|---|
| box, default | 50.7 % | 31.4 % | 12.5 % | 4.5 % | 0.8 % | 0.1 % | ≈ 0.11 | 5 % |
| box, r10 | 67.8 % | 22.4 % | 8.2 % | 1.7 % | 0 | 0 | ≈ 0.07 | 2 % |
| hall, r10 | 30.2 % | 16.2 % | 20.0 % | 24.1 % | 9.6 % | 0 | ≈ 0.25 | 34 % |

In the hall the cap was binding for a third of the samples. The box sits under it.

### Results

| | default | r10 with CONV=1 | r10 (CONV=3) |
|---|---|---|---|
| box live/ref, 900 frames | 0.562 (f 0.764) | 0.566 (f 0.767) | **0.661** (f 0.845) |
| box test passes | – | round 18, frame 22 | round 599, frame 603 |
| hall indirect/ref, 1140 frames | 0.765 (round 9) | – | 0.821 |
| hall test passes | – | – | never (1381 rounds) |
| hall still ms p50 / p90 (840–1140) | 0.09 / 0.12 | – | 270 / 326 (never froze) |
| hall still flicker, per-pixel luma std | 0 | – | 0.00000 |
| linger off 30 / 120 / 300 (gate ≤ 5 % at 120) | 65 / 56 / 46 % (round 9) | – | 92 / **72** / 38 % |
| lag on 60 / 120 / 300 / 600 (gate ≥ 90 % at 300) | 16 / 23 / **31** / 33 % (fresh) | – | 24 / 44 / **74** / 79 % |

**Why the box passes at frame 22.** A cell's stored step from one update is about σ/n, but the threshold is 3σ/√n, and σ/n < 3σ/√n for any n ≥ 1. A single update can exceed the threshold only if its sample sits more than 3σ√n from the mean. So a mean that is still rising, with every sample inside the noise band, passes at once. This is structural, so removing the cap cannot fix it: with SIGCAP the box still passes at frame 22. The trend term fixes the box: it passes at frame 603 and the box reaches 0.661, against 0.566 when it freezes at frame 22.

**Why the hall never passes, even uncapped.** 57–62 % of relit cells exceed 3σ/√n, and 86–90 % move more than 1 % on every relight, in every round. This is inconsistent with converged cells at count n. Steps that large mean the hall cells' counts stay small. The likely causes are the two-sided reset, which drops a cell to 4 when its sample is outside max(10 %, 3σ), and cell reuse. This is not measured yet (next: a count histogram). So the cache never freezes and the idle cost stays at the no-freeze level, about 270 ms on lavapipe.

**The texel reset has no effect, because the hall has no surface-cache texels.** A diagnostic counter (d[34], texels relit, outside the WRC) is 0 over the whole linger run. The WRC lamp-off reset fires (d[39], 1.4 M cell resets) but d[38] (texel resets) is 0. So `LIGHTING_CACHE_MATURE` texels are not the source of the linger. The 72 % left at 120 frames sits downstream of the WRC: the probes, or the screen and compose history. That is where to look next.

The lag gate is still missed: 74 % at 300 frames, against 31 % for the shipped default.

Note: the "dark" baselines differ (default 0.121, r10 0.026). The shipped cache keeps a large part of the lamp light in the dark state, so the default percentages are of a smaller range.

### Changes to the proposed design

1. The convergence test needs a trend or minimum-sample term. The noise-only test cannot detect a rising mean. CONV=3 is the first rule that does not freeze the box early.
2. Before any freeze rule can pass in the hall, find out why hall cells keep moving by more than 1 % per relight: measure their count distribution and how often the mode-3 reset fires.
3. Linger: instrument the probe and screen-history layers. The cache-side resets (WRC and texel) are now in place and do not move the gate.

## Round 11: which layer holds the old light, and hall sample counts (1 Nov 2026)

Build: 34cc80c plus the regenerated `/home/box/shared/adr0037-hooks.patch` (7 files, +216/−8). New hooks, all 0 by default, in the debug word (`scene.grid_at.w`, bits 26–31, unused by shipped code):

- `GENOS_DBG_LAYER` (bits 26–28): compose shows one layer. 1 probes only, 2 WRC (world) only, 3 surface-cache texels skip the pushed à-trous display field and read the WRC as of now, 5 probes only with the probe gather and filter holds off.
- `GENOS_DBG_HIST` (bit 29): the noise bins count relit WRC cells by their count before any reset; d[32] counts mode-3 resets, d[34] new cells.
- `GENOS_DBG_RESET` (bits 30–31): mode-3 reset variant. 0 drops the count to 4 (r10), 1 halves it (floor 4), 2 halves it and widens the band floor to 20 %.

The counters use only the free words listed in round 10. All runs used the r10 config, 160×90, on lavapipe.

### Linger by layer (dark7, lamps off after 300 lit frames; share of lit-minus-never left)

| layer shown | off 30 | off 120 (gate ≤ 5 %) | off 300 |
|---|---|---|---|
| full picture, r10 (round 10) | 92 % | 72 % | 38 % |
| 2: WRC (world) only | 88 % | 69 % | 39 % |
| 1: probes only | 100 % | 99 % | 99 % |
| 5: probes only, no gather/filter hold | 101 % | 100 % | 100 % |
| 3: texels skip the pushed à-trous field | 45 % | **14 %** | 13 % |

Findings:

- **The pushed à-trous display field is the main holder.** When texels read the WRC directly, the share left at 120 frames drops from 72 % to 14 %. The WRC-only view lingers as much as the full picture because that view still goes through the pushed field. The à-trous log shows `dirty 0.0000` after switch-off, so the field is never marked dirty and never repainted.
- **The probes never let go.** With the probes alone, 99–100 % of the lamp light is still there after 300 frames, with or without the gather and filter holds. A switch-off therefore does not reach the probes at all. My unverified guess is that the probe radiance is read from cached hit shading that a lamp switch does not invalidate. The remaining 13–14 % with layer 3 is consistent with the probes' share of the mix.
- No fix was tested this round. Layer 3 is a diagnostic, not a fix, and still misses the gate (14 %). The lag gate was not rerun.
- `GENOS_LIGHTING_COMPOSE_ALL=1` with layer 5 hung: the frame stopped serving script commands, the run hit its 1500 s timeout, and it produced no frames. Without COMPOSE_ALL, layer 5 ran normally.

### Hall WRC counts before reset (relit cells, frames 600–1140)

| | ≤ 8 | 9–16 | 17–32 | 33–64 | > 64 | resets / relight | new cells | > 3σ/√n |
|---|---|---|---|---|---|---|---|---|
| default | – (frozen; frames 100–400: 0.5 %) | 0.3 % | 1.0 % | 4.1 % | 94.1 % | 0 | 0 | 0.05 |
| r10 (reset to 4) | 39.9 % | 24.2 % | 15.0 % | 10.1 % | 10.7 % | 0.40 | 0 | 0.59 |
| r10, RESET=1 (halve) | 14.8 % | 33.3 % | 23.2 % | 12.2 % | 16.5 % | 0.43 | 0 | 0.59 |

The default froze, so it relit no cells after frame 600; its row is frames 100–400. **The two-sided reset is what keeps hall counts low.** 40 % of relights fire it, and no cells are new, so cell reuse is not the cause. Halving instead of dropping to 4 moves counts up, but the reset still fires on 43 % of relights. The hall's per-sample noise (round 10, rms ≈ 0.25) puts a large share of single samples outside max(10 %, 3σ), so a reset on one sample cannot work in the hall.

### Hall results (hall9, 1140 frames)

| | default | r10 | r10, RESET=1 |
|---|---|---|---|
| indirect/ref | 0.767 | 0.823 | 0.813 |
| CONV=3 test passes | – | never | never |
| still ms p50 / p90 (840–1140) | 0.09 / 0.11 | 345 / 402 | 163 / 220 |
| flicker, per-pixel luma std | 0 | 0.00000 | 0.00001 |

Box live/ref was not rerun. No box-side code changed after round 10's 0.661.

### Changes to the proposed design

1. Linger: a lamp switch must mark the à-trous display field dirty (or repaint the texels inside the lamp's sphere), and it must reach the probes. Both are needed: fixing the field alone leaves about 14 %.
2. Reset: one out-of-band sample must not trigger it. Next to test: a reset after several out-of-band samples in a row, or a reset driven only by the lamp-change record (LAMPOFF) with no sample band.

## Round 12: lamp switch reaches the display field and probes; run-length reset (1 Nov 2026)

Build: 34cc80c plus the regenerated `/home/box/shared/adr0037-hooks.patch` (8 files, +279/−13). New hooks, all 0 by default, packed into a second debug word (bit 0 `GENOS_DBG_FIELD`, bit 1 `GENOS_DBG_PROBE`, bits 2–3 `GENOS_DBG_RUN`, bit 4 `GENOS_DBG_FIELD2`):

- `GENOS_DBG_FIELD`: a texel inside a switched lamp's range is relit and its pushed à-trous display field marked dirty.
- `GENOS_DBG_FIELD2`: inside a switched lamp's range, no idle skip, and the pushed field is replaced rather than blended.
- `GENOS_DBG_PROBE`: a kept probe hit that a switched lamp reaches is reshaded and its history dropped.
- `GENOS_DBG_RUN`: 1 no band reset (lamp-change record only); M = 2 or 3 resets only after M out-of-band samples in a row.

All runs use the r10 config, 160×90, lavapipe. FX = FIELD + PROBE + FIELD2 + RUN=3.

### Linger (dark7; share of lit-minus-never left; gate ≤ 5 % at 120)

| config | off 30 | off 120 | off 300 |
|---|---|---|---|
| r10 (round 10) | 92 % | 72 % | 38 % |
| r10 + FIELD | 99 % | 69 % | 31 % |
| r10 + FIELD + PROBE | 95 % | 65 % | 28 % |
| r10 + FIELD + PROBE + FIELD2 | 64 % | 66 % | 30 % |
| same + RUN=3 (run 1) | 42 % | 64 % | 30 % |
| r10 + FX (run 2) | 44 % | **51 %** | 28 % |

A further run (l12P1: 35 / 11 / 11 %) had a different lit level (0.435 against about 0.32) and its env was not recorded, so it is most likely a single-layer view, not the full picture, and is left out of the comparison.

### Lag (onramp7; share of the settled lit bounce reached; gate ≥ 90 % at 300)

Reference is the matching 1140-frame hall run (h12m1 or h12m3); against round 10's h10c3 reference each value is about 1–2 points lower.

| config | on 60 | on 120 | on 300 | on 600 |
|---|---|---|---|---|
| r10 (round 10) | 24 % | 44 % | 74 % | 79 % |
| r10 + FIELD + PROBE | 46 % | 60 % | **81 %** | 87 % |
| r10 + FIELD + PROBE + FIELD2, RUN=3 | 96 % | 61 % | 79 % | 82 % |
| r10 + FX | 96 % | 58 % | 79 % | 82 % |

The 96 % at 60 frames with FIELD2 is a transient overshoot that falls back by 120 frames. It is not a pass.

### Hall (hall9, 1140 frames; FIELD + PROBE + HIST, RUN = M)

| | default | r10 (round 10) | RUN=1 | RUN=2 | RUN=3 |
|---|---|---|---|---|---|
| indirect/ref | 0.767 | 0.821 | 0.811 | 0.811 | 0.806 |
| CONV=3 test passes | – | never | never | never | never |
| still ms p50 / p90 (840–1140) | 0.09 / 0.12 | 270 / 326 | 35.7 / 46.7 | 96.3 / 156 | 58.9 / 83.2 |
| flicker, per-pixel luma std | 0 | 0.00000 | 0.00000 | 0.00000 | 0.00000 |

### Box (box, 900 frames, live/ref; must stay ≥ 0.66)

| config | live/ref |
|---|---|
| r10, round 10 | 0.661 |
| r10, rerun ×2 this round | 0.643, 0.671 |
| r10 + RUN=3 | 0.639 |
| r10 + FIELD + FIELD2 | 0.642 |
| r10 + PROBE | 0.660 |
| r10 + FIELD + PROBE | 0.647 |
| r10 + FX | 0.635 |

**The box gate sits inside run-to-run noise.** The same r10 config gave 0.643 and 0.671 in two runs, so a spread of about ±0.015. No FX hook can be said to lower the box. The 0.66 gate needs several runs per config, or a longer run, to mean anything.

### Best combined config against all gates

r10 + FX is the best single config. It passes no gate except flicker:

| gate | target | r10 + FX | pass |
|---|---|---|---|
| linger at 120 | ≤ 5 % | 51 % | no |
| lag at 300 | ≥ 90 % | 79 % (81 % without FIELD2/RUN) | no |
| hall CONV=3 passes, still ms | ~0.1 ms | never; 59 ms | no |
| hall indirect/ref | 1.0 (higher is better) | 0.806 | no |
| flicker | 0 | 0 | yes |
| box live/ref | ≥ 0.66 | 0.635 (noise ±0.015) | inconclusive |

### What is still failing

- **Linger: the field and probe hooks remove only about a third of the leftover light** (72 % to 51 %). Round 11's layer-3 diagnostic, which skips the pushed field entirely, reached 14 %. So the hooks don't yet clear the field everywhere the lamp lit: the lamp's range is probably too small for its bounce footprint, or dirty texels are only partly repainted per frame. The à-trous log still shows `dirty 0.0000` in screen mode, so the screen-mode filter history is not being reset either.
- **Lag stays around 80 % at 300 frames** with every combination.
- **The run-length reset makes the hall much cheaper, but it still never freezes.** RUN=1 (no band reset) cuts the idle cost from 270 ms to 36 ms. With RUN=1 the noise test is met from round 537 on, yet the drift test never passes, so CONV=3 never freezes.

### Changes to the proposed design

1. Linger: invalidate by the lamp's bounce footprint, not its direct range, and reset the screen-mode à-trous/denoise history on a lamp switch.
2. Reset: prefer RUN=1 (lamp-change record only, no sample band). It is the cheapest and does not lower hall energy beyond the M = 2–3 variants.
3. Convergence: the drift term needs a tolerance relative to hall noise, because the noise test alone already passes at RUN=1.
4. Box gate: measure with at least 3 runs per config.

## Round 13

Hooks R10 + FX with RUN=1, dark7 / onramp7 / hall9 / box, lavapipe 160x90, lighting-v3 34cc80c. New debug hooks (all 0 = shipped):

- `GENOS_DBG_FOOT=n` (debug word 2, bits 5-7): grows each recorded lamp-change sphere by n metres (lamp range + n, the bounce footprint). Any non-zero value also turns off the held probe filter while the light is not quiet (`lighting_direct.comp`). 7 marks every cell as changed (perfect-invalidation oracle).
- `GENOS_DBG_LAYER=n` (scene word bits 26-28): picks which bounce layer compose shows. 1 is probes only, 2 world only, 3 skips the pushed a-trous field (WRC as of now), 4 the texel's own mean, 5 probes only with no held gather or filter, 6 the full picture with no held gather or filter.
- `GENOS_DBG_CONV=4`: the convergence test. It needs mode 1's noise test (under 1 % of relit cells more than 3σ/√n off), at least `GENOS_DBG_CONV_MIN` rounds (default 64) and 16, and a summed signed step over the last 16 rounds under max(0.5 %, 8 × the std of those steps). It is noise-relative drift rather than a fixed ε.

### Linger (dark7; gate ≤ 5 % at 120)

| config | off 30 | off 120 | off 300 |
|---|---|---|---|
| r12 FX | 43.8 % | 50.6 % | 27.8 % |
| FX + FOOT=3 (footprint clear, no held filter in window) | 47.0 % | 49.2 % | 27.6 % |
| FX, LAYER=1 (probes only) | 25.8 % | 11.8 % | 11.8 % |
| FOOT=7 oracle | aborted: about 0.4 fps, wait timed out at frame 221 | | |

### Lag (onramp7; gate ≥ 90 % at 300; reference h13c4)

| config | on 60 | on 120 | on 300 | on 600 |
|---|---|---|---|---|
| r10 + FX (current best) | 95.2 % | 56.8 % | 78.6 % | 82.4 % |
| cold start lit (perfect-invalidation proxy) | 114.3 % | 84.9 % | **94.8 %** | 96.0 % |

The FOOT=3 lag run (o13A) has no result and was not re-run.

### Hall (hall9, 1140 frames, CONV=4, RUN=1)

| | value |
|---|---|
| freezes | round 500, frame 545 |
| still-camera lighting p50 / p90 (frames 840-1140) | 0.11 / 0.17 ms |
| indirect / ref | 0.801 |
| direct / ref | 0.997 |
| flicker (per-pixel luma std) | 0.00001 |

### Box (900 frames, live/ref, 3 runs each; must stay ≥ 0.66)

| config | run 1 | run 2 | run 3 | mean |
|---|---|---|---|---|
| B | 0.631 | 0.621 | 0.658 | 0.637 |
| A | 0.645 | 0.630 | 0.639 | 0.638 |

Run-to-run spread is about ±0.02, so round 10's 0.66 box result was within noise of the 0.64 seen here. Neither config meets 0.66.

### Verdict

The current architecture can't meet the linger and lag gates with local tuning.

- Settling is solved: CONV=4 freezes the hall at frame 545 at 0.11 ms with no flicker.
- Lag is reachable only when all history is gone: cold start reaches 94.8 % at 300 frames, and the best tuned build reaches 78.6 %.
- Linger doesn't move when the WRC change sphere grows (FOOT=3: 49 %), so the stale light is not in the cells the sphere covers. Probes alone still hold 12 %, over the 5 % gate.
- The stale light sits in several chained histories (WRC cells, texel means, the display field, probe gather and filter, compose history). Each relearns only through the others, so resetting one layer is refilled from the next.

**Design change needed:** event-driven invalidation. A lamp power or on/off event must invalidate every history inside the lamp's influence footprint in the same frame: WRC cells, per-texel means and the display field, probe irradiance and its gather/filter hold, and screen-space or compose history. Each history then restarts from a cold count. The relight runs under a bounded per-frame budget (a capped number of cells or probes per frame, most important near the camera first), so a mass switch defers work rather than spiking the frame. CONV=4 then decides when to freeze again. FOOT=7 shows that relighting everything at once is far too expensive, so the budget is required.

## Round 14

Prototype of the round-13 design as one debug hook on lighting-v3 34cc80c (uncommitted, in `/home/box/shared/adr0037-hooks.patch`). Runs are R10 + FX with RUN=1 and CONV=4, on lavapipe at 160x90, one run per config unless noted.

- `GENOS_DBG_EVT=1` (debug word 2, bit 8): on a recorded lamp power or on/off change, a WRC cell inside the change sphere restarts from zero samples (t = 0). Texel means restart (as TEXOFF). Probe gather and filter hold, plus compose's direct history, are off while the light is not quiet. The display field repaints through FIELD/FIELD2.
- `GENOS_DBG_EVT_BUDGET=n` (bits 9-20, n/16): at most n cell restarts per frame. Cells within 6 m of the eye can use the whole budget and farther cells only half, which approximates nearest-first. A cell over budget is cleared to 0 and relights as new. This is not a true sort.
- `GENOS_DBG_EVT=2` (bit 21): a proportional A-SVGF-style drop instead of the clear. A WRC cell's count is scaled by (1 - λ), where λ = |new - old| / max(new, old) is clamped to 0..1. **Limitation:** λ comes from this pass's sample against the stored mean, not from an old-seed re-trace, and only WRC cells use it. Texels, probes and compose behave as in EVT=1.

### Linger (dark7; gate ≤ 5 % at 120)

| config | off 30 | off 120 | off 300 |
|---|---|---|---|
| r13 best (FX) | 43.8 % | 50.6 % | 27.8 % |
| EVT=1, budget 1024 | 36.8 % | 42.4 % | 24.9 % |
| EVT=1, budget 8192 | 38.0 % | 40.6 % | 21.7 % |
| EVT=2, budget 1024 | 36.7 % | 35.7 % | 18.1 % |

### Lag (onramp7; gate ≥ 90 % at 300; reference: hall9 run h14)

| config | on 60 | on 120 | on 300 | on 600 |
|---|---|---|---|---|
| r13 best | 95.2 % | 56.8 % | 78.6 % | 82.4 % |
| EVT=1, budget 1024 | 97.9 % | 73.5 % | 89.9 % | 92.0 % |
| EVT=1, budget 8192 | 96.4 % | 68.1 % | 85.9 % | 88.8 % |
| EVT=2, budget 1024 | 97.4 % | 65.9 % | 85.1 % | 87.9 % |

The "dark" baseline varies from 0.011 to 0.049 between runs, so lag figures carry about ±4 points of noise. EVT=1 at 1024 is at the gate, not clearly over it.

### Switch-frame cost (lavapipe lighting ms; 120 frames after the switch vs 60 before)

| run | switch | after p50 | after max | before p50 | WRC restarts / over-budget clears |
|---|---|---|---|---|---|
| EVT=1 1024, dark7 | on | 704 | 11075 | 87 | ~37k / ~196k |
| EVT=1 1024, dark7 | off | 366 | 936 | 75 | ~34k / ~187k |
| EVT=1 8192, dark7 | on | 716 | 945 | 95 | ~175k / 80k |
| EVT=1 8192, dark7 | off | 323 | 716 | 76 | ~167k / 75k |
| EVT=1 1024, onramp7 | on | 630 | 831 | 88 | |
| EVT=2 1024, dark7 | off | 380 | 871 | 76 | 262k drops |
| EVT=2 1024, onramp7 | on | 707 | 1037 | 103 | 295k drops |

On lavapipe, a switch costs 4-8x the lit-but-unsettled frame (and far more than the 0.1 ms frozen still frame) for about 120 frames. The budget barely changes this. The cost is dominated by the no-hold probe gather/filter and full-rate relight, not by WRC restarts. The 11 s max on the first switch is a one-off stall, probably pipeline or allocation. A 4070 measurement is needed for real ms.

### Hall and box (no lamp switches, so no EVT events fired)

| | r13 | round 14 |
|---|---|---|
| hall indirect/ref | 0.801 | 0.817 |
| hall freeze, EVT=1 8192, no HIST | frame 545 | never (run lacked GENOS_DBG_HIST=1) |
| hall freeze, EVT=1 1024 + HIST=1 (h14e1) | frame 545 | **never**: no `dbgconv` line at all in 1980 frames |
| hall freeze, EVT=2 1024 + HIST=1 (h14e2) | frame 545 | **never** |
| hall still lighting p50 (no HIST / e1 / e2) | 0.11 ms | 47.6 / 48.5 / 45.8 ms (unfrozen) |
| hall indirect/ref (e1 / e2, HIST=1) | 0.801 | 0.813 / 0.834 |
| hall flicker per-pixel std (e1 / e2) | 0.00001 | 0.00003 / 0.00014 |
| hall flicker (per-pixel std) | 0.00001 | 0.00005 |
| box live/ref, 3 runs | 0.631 / 0.621 / 0.658 | 0.582 / 0.582 / 0.581 |

The box is 0.582 over 3 runs, down from 0.637 in r13: **a regression**, with no lamp events fired. With HIST=1 the hall still never reaches a CONV=4 pass (the r13 hall run h13c4 logged `dbgconv pass mode 4 round 500 frame 545`; h14e1/e2 log no `dbgconv` line at all), so the round-14 flag set (FX plus EVT) disables or bypasses the CONV=4 settle path. That is the likeliest cause of both the box drop and the 46-48 ms unfrozen still cost; not yet traced to a line.

### Step sizes (st_*.rhai: 32 point lamps step together at frame 568; budget 1024; conv % against each config's own pre (frames 560-567) and t600 levels)

| step | config | old → new bounce | conv 30 / 120 / 300 | post-step f2f noise / settled | max mean-luma jump post / settled |
|---|---|---|---|---|---|
| 100 → 90 % | e0 default | 0.382 → 0.391 | 85 / 93 / 95 % | 0.5x | 0.0001 / 0.0003 |
| | e1 clear | 0.404 → 0.457 | 85 / 100 / 100 % | 47x | 0.0043 / 0.0002 |
| | e2 proportional | 0.416 → 0.457 | 90 / 100 / 100 % | 46x | 0.0021 / 0.0001 |
| 100 → 110 % | e0 | 0.382 → 0.393 | 82 / 89 / 92 % | 1.3x | 0.0005 / 0.0006 |
| | e1 | 0.419 → 0.484 | 69 / 99 / 100 % | 35x | 0.0051 / 0.0003 |
| | e2 | 0.404 → 0.493 | 95 / 99 / 100 % | 34x | 0.0029 / 0.0004 |
| 100 → 50 % | e0 | 0.382 → 0.387 | 99 / 100 / 100 % | 0.4x | 0.0001 / 0.0002 |
| | e1 | 0.402 → 0.361 | -54 / 74 / 97 % | 36x | 0.0220 / 0.0008 |
| | e2 | 0.414 → 0.353 | 13 / 86 / 99 % | 33x | 0.0208 / 0.0004 |
| 50 → 100 % | e0 | 0.259 → 0.287 | 87 / 93 / 94 % | 2.1x | 0.0008 / 0.0002 |
| | e1 | 0.205 → 0.378 | 70 / 85 / 99 % | 2254x (settled ≈ 0) | 0.0280 / 0.0000 |
| | e2 | 0.279 → 0.386 | 53 / 93 / 99 % | 218x | 0.0204 / 0.0000 |

**These step numbers do not measure step response cleanly; read them with these caveats:**
- conv % uses each config's own settled levels, as intended. But the pre-step level is not converged at frame 560: the three configs start from 0.38, 0.40 and 0.42 on the same scene, and for e1/e2 the 90 % step makes bounce *rise* (0.40 → 0.46) when it should fall about 10 %. The EVT restart lets the cache climb toward its true level, so the "step" mixes the lamp change with leftover convergence.
- e0 (default) barely moves on any step (+0.005 to +0.028, against expected −0.04 to +0.17). Its high conv % and missing noise come from not responding, not from responding well.
- e1 and e2 do respond, but every step, even ±10 %, adds frame-to-frame noise 33-47x the settled level (218-2254x where the settled frame was frozen) and a mean-luma jump 5-25x the settled one. At 160x90 this reads as a visible pop then shimmer for the first ~30-120 frames. e2 is not quieter than e1 on mild steps, because λ is taken from one noisy new sample against the stored mean: on a ±10 % change the sample noise dominates λ, so cells drop most of their history anyway. This is the expected failure of the pass-sample λ; true A-SVGF compares same-seed old/new samples to cancel that noise.
- The lamp-power-proportional variant (λ = |ΔP| / max(P_old, P_new) by the lamp's share) was not built this round (time).

### Verdict

**The design as prototyped does not meet the gates, and the proportional drop as built is not an all-rounder.**
- Lag: EVT=1 at budget 1024 reaches 89.9 % at 300 (gate 90 %, within ±4 points of run noise). Budget 8192 and EVT=2 are worse (85-86 %).
- Linger: 36-42 % left at 120 frames (EVT=2: 36 %) against a 5 % gate, whatever the budget. The stale light is in a history this hook does not clear, or is refilled from one; candidates are probe irradiance (r13: 12 % alone) and the display field beyond the lamp-range sphere.
- Post-step noise: after every step, including ±10 %, EVT=1 and EVT=2 add 33-47x the settled frame-to-frame noise and a visible mean jump; default shows none, but only because default barely responds to power changes at all. EVT=2's λ comes from one new sample against the stored mean (not a same-seed A-SVGF re-trace), so on mild changes sample noise dominates λ and it drops nearly as much history as the hard clear. It is no better than EVT=1 on mild steps.
- Settling: with the round-14 flags the hall never freezes (46-48 ms still cost on lavapipe vs 0.11 ms) and the box drops to 0.582; a regression to fix before anything else.
- Switch cost: about 120 frames at 4-8x the frame cost on lavapipe; the cell budget does not bound it.
- All-rounder (sun sweep, moving objects, spinning lamps, camera turn): **not tested** this round.

Next: (1) find why CONV=4 never passes under FX+EVT and restore the freeze; (2) a λ from lamp power (|ΔP| / max(P_old, P_new) weighted by the lamp's share or distance falloff) or a true same-seed re-trace, so mild steps keep their history; (3) the oracle that zeroes probe irradiance and the whole display field on the switch frame, to locate the remaining ~40 % linger.

## Round 15

Measurement fixes first, then a power-based λ (`GENOS_DBG_EVT=3`). Hooks are on lighting-v3 34cc80c (uncommitted; `/home/box/shared/adr0037-hooks.patch`). Runs use lavapipe at 160x90.

### Why round 14's settle check never froze, and why the box dropped

The EVT flags were not the cause. The settle test itself had two faults.

- **Box 0.637 → 0.582.** Round 13's boxes ran with `CONV=3` (`r13.sh` sets `R10` to CONV=3), which never passed, so they never froze and kept converging. Round 14's boxes ran `CONV=4`, which passed at round 83 (frame 87) while the box was still rising. Box rounds alternate two cell sets, with round-mean luma about 0.5 and then 0.15. That makes the per-round step's standard deviation large, so mode 4's drift tolerance of 2·sd·√16 is too loose (`dbg_conv_note`, mode 4 arm).
- **Hall never freezes.** Mode 4's noise test is per round: fewer than 1 % of relit cells may move by more than 3σ/√n, for 8 rounds in a row. After the sweep, a round relights only 1-10 cells, so a single noisy cell fails it. In r13 (`h13c4`), frames 538-545 happened to be clean, so it passed at 545. In r14 (`h14e1`), frames 538-543 each had one noisy cell. The next sweep then started at frame 550 and never went quiet. The same 0-2 noisy cells per round appear in r13 at frames 515-536, so r13's pass was luck.
- **Step-test confound.** Round 14's e0 step runs had no CONV and froze as shipped (pre-step lighting 0.1 ms). The e1 and e2 runs had CONV=4 and never froze (pre-step 40 ms), so their pre-step levels were still rising when the lamp stepped. That is why the 90 % step went up under e1 and e2.

**Fix: `GENOS_DBG_CONV=5`.** Counts are pooled over the last 32 rounds. It passes when two conditions hold:
- noisy cells are under 1 % of relit cells, or under 1 % of the sweep's peak round;
- the pooled mean of the last 32 rounds is within 0.5 % of the 32 rounds before it, since both windows mix the same cell sets.

The step scripts (`st15_*.rhai`) now:
- wait 1100 frames before the step;
- log the settled level and the drift from frame 1000 to 1100;
- capture at 1200 frames after the step, so "new level" no longer means t600.

### EVT=3: power-based λ

- On a lamp change, the change ring stores λ = |P_new − P_old| / max(P_new, P_old), with P ∝ range². It is packed in the top 8 bits of the ring entry's frame word, and a new or moved lamp counts as λ = 1.
- Each cell takes λ × falloff, where falloff is 1 inside half the lamp's range and falls to 0.25 at the range edge. λ = 1 applies over the whole range, which is a hard clear.
- WRC cells scale their count by t *= 1 − λ_cell.
- Texels restart only where λ_cell ≥ 0.5.
- **Limitation:** probe gather and filter, and compose, behave as in EVT=1 (no hold while the light isn't quiet). They are not scaled, because the probe history has no per-lamp count in reach.

## Open questions for Anthony

- The values of ε, K and the reset threshold X, plus whether a downward reset (lamp off) uses the same X.
- Whether the texel mean (`LIGHTING_CACHE_MATURE`) gets the same forgetting mean.
- Whether mode 0 (world cache off) should keep relighting its light cache at all (round 6).

## Appendix: investigation log (rounds 1–6)

### Context

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

#### Cause

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

#### Energy loss (the "3.5× darker" and "6–10 % dark")

The 3.5× gap (0.041 against 0.144) is **this same history lag, not an energy-loss term**. It was measured 300 frames after switching on from dark.

When the scene is lit from the start and settled for 600 frames, against a 64-bounce reference (`energy.rhai`, indirect = b64 − b0 reference = 0.2778):

| | direct / ref | indirect (live − direct) | indirect / ref |
|---|---|---|---|
| WRC on | 0.997 | 0.2407 | 0.87 |
| WRC off | 0.997 | 0.2621 | 0.94 |

So the WRC on its own costs about 7 % of indirect light at convergence, on top of about 6 % that both paths share. I found no single clear wrong term (no π, cosine or double factor). The remaining suspects are coarse hit-patch size and normal-bucket leak at corners, and cells read before they are full. They need their own measurement, and this ADR does not decide them.

### Options

- **(a) Relearn on lamp power change (recommended).**
  - Record a lamp in the change ring when its power changes, including going to 0. For a lamp that went off, record its old range and power.
  - Every WRC cell inside a recorded lamp's old or new sphere then drops its history to a small count, as ADR 0013's local invalidation does for movers. It re-gathers at the local rate.
  - This removes both the linger and the switch-on dark, with no global effect.
  - Cost is bounded by the lamp's sphere and by the existing per-preset relight cap: a worst-case switch event defers work, it doesn't exceed the frame budget.
  - Until the cells refill, they show their history scaled toward the parent or probe value, so there is no black pop.
- **(b) Fast blend or reset only where incoming light drops sharply.** Cheaper, but it is driven by noisy per-cell estimates. It risks flicker and still misses slow drift.
- **(c) Settle only after the running averages converge.** Require cache rounds that scale with the time the cells need to converge after a light change before mode 3 counts as settled. This is useful robustness, but the switch-on test shows the settle rule is not the cause, so it fixes nothing by itself.

### Interaction with existing decisions

- **ADR 0013 (local invalidation, lamp-change tracking, #59).** Option (a) extends the same ring. A power change becomes a change record like a move. No full-frame fallback, and work stays under the caps.
- **Anthony's rules:**
  - No ghosting: the linger is ghosting, and (a) removes it.
  - No pops: cells blend from history, not from 0.
  - Worst-case light-switch events hit the frame budget: the work is capped and deferred, never forced.
  - Full-relight cost matters most: (a) re-gathers only the cells inside the switched lamps' spheres, never the whole cache.

### Gate

A test in `crates/render/tests/lighting_frame.rs` on the lighting-v3 High preset, Bounce view: hall, 32 lamps, 22:00, held camera.

- **Off case.** After the lamps switch off, the lamp-added bounce left must be ≤ 5 % by 120 frames, where it is 54 % at 30 frames today.
- **On case.** After switching on from dark, bounce must reach ≥ 90 % of the lit-from-the-start value by 300 frames, against 28 % today.
- The worst-case switch frame must stay within the preset's frame budget.

The energy number (indirect / 64-bounce reference ≥ 0.95) is tracked separately.

### Remaining energy gaps (diagnosis, lighting-v3 34cc80c)

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

#### Per-bounce series (lighting-v3 34cc80c, 31 Oct 2026)

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

#### Simple-geometry scenes (lighting-v3 34cc80c, 31 Oct 2026)

These runs use `--scene` files with built-in shapes: an open 24 m floor with one wall, a 6×6×3 m closed box with 0.2 m walls, the same box with 1 m walls, and the box without its roof. Each has one small lamp. Other settings:

- `world_cache: false`, 160x90, live settled
- tone-inverse decode, with scenes dim enough to stay in the linear range
- direct light within 0.1 % of the reference in every scene

`c_live` is live one-bounce, from surface-cache flag 2097152, which turns off cache reads at hits. Each scene is fitted to live = c_live / (1 − g·f), where g is the reference loop gain, g = 1 − b1/b64.

| scene | ref b1 | ref b64 | live 1-bounce | live | live/ref | g | f |
|---|---|---|---|---|---|---|---|
| open plane + wall | 0.1480 | 0.1945 | 0.1417 | 0.1746 | 0.898 | 0.239 | ≈0.79 (ill-conditioned) |
| closed box, thin walls | 0.0825 | 0.3544 | 0.0816 | 0.1600 | 0.451 | 0.767 | 0.64 |
| closed box, 1 m walls | 0.0791 | 0.3372 | 0.0783 | 0.1562 | 0.463 | 0.765 | 0.65 |
| box, no roof | 0.0386 | 0.0832 | 0.0377 | 0.0579 | 0.696 | 0.536 | 0.65 |

For the box, the reference's per-hop increments are 0.0825, 0.0606, 0.0472 and 0.0365, so b2 = 0.143 and b3 = 0.190. Live, at 0.160, falls between b2 and b3.

**Findings:**

- **First bounce is exact in every scene,** at 0.96–0.99 of reference b1. The whole loss is in hops 2 and later, as in the hall.
- **The per-hop factor is about 0.64–0.65 in every closed or semi-closed scene.** It does not change with wall thickness or with the roof. A 3 m room matched the 6 m box in a first pass. **So this is not a spatial or normal leak through thin walls or corners.** The loss is far larger than the hall's ≈0.95, which suggests the hall's high reflectance and large open area hide it. The open-plane f is ill-conditioned because g = 0.24.
- **None of these changed the box value (0.1600 ± 0.0001):**
  - Freezing: disabling the settle freeze, `GENOS_LIGHTING_SC_SETTLE=100000`
  - Scheduling: disabling young-only scheduling; forcing every texel into the young list; flag 8192 (texels keep relighting at the cap)
  - Hit reads: skipping the sized-texel read at hits, so every hit reads its hit-size patch
  - Estimators: forcing `lighting_sc_sun_gain` to 1; SVGF off (flag 2^31); MSME off (flag 2^30)
  - Earlier rounds: firefly sigma 1e5; flags 256, 32768 (fill unlit patches), 65536, 131072 and 1048576; waiting 3600 frames
  - No cache relight aborted on a none-ray; a counter hook stayed at 0.
- **The cache reaches a fixed point that is low.** It is not truncated by scheduling. Freezing at frame ~70 and relighting for 976 rounds give the same picture.
- **Scaling the light that hits read from the cache by K** (live box: K = 0 → 0.0735, 0.7 → 0.126, 1.0 → 0.160, 1.3 → 0.207) gives a convex response. So the cache does carry more than one hop, but each hop is attenuated by ≈0.64.
- **Ruled out by reading the code:**
  - The relight estimator: E = 2π/N·Σ L·cos with uniform-in-cos θ directions is correct for those samples.
  - `lighting_ray_light` decoding.
  - The hit radiance at cache and probe rays, ρ/π·(E_direct + E_cache).
- **Still open:** the irradiance of the 25 cm-and-up hit patch (`lighting_cache_irradiance`, found with `lighting_cache_find_sized(..., hsize)`) against the reference at the same point. Two leads remain: the patch's offered lighting point (`lighting_sc_offer`, `lighting_sc_centre`), and the WRC-cell running-mean commit in `lighting_cache.comp`. The next step is a probe that writes one patch's E next to the reference irradiance at its centre.
- **No fix is proposed yet.** No clear term turned up, so there is no failing test and no PR.

Note: the first round-5 table (box f 0.49) came from a stale build that still had a round-4 hit-albedo hook. Every number above comes from a clean rebuild with all hooks neutral.

#### Patch probe, round 6 (lighting-v3 34cc80c, 31 Oct 2026)

**Correction: the simple-geometry table was measured with the surface cache off.** The run script sets `world_cache: false` before the wait (as listed above), so `settings.world_mode()` is 0 from frame 5 on (a host print of the change head shows mode 3 for frames 1–4, then 0). In mode 0, `lighting_compact.comp` skips the whole surface-cache scheduling block. A slot-entry counter shows about 12,360 live slots visited each frame, of which 6,530 are 25 cm hit patches, and none of them is listed. The cache relights only in the first ~10 frames (about four hops), and is then read without any further relights. That is the "low fixed point" above, and it explains why no scheduling, freeze or estimator change moved the number.

The thin-wall box at 900 frames, clean build (all debug hooks stashed), same camera and reference:

| run | live | live/ref | f |
|---|---|---|---|
| `world_cache: false` (table above) | 0.1600 | 0.451 | 0.631 |
| `world_cache` left on (surface cache, mode 3) | 0.1989 | 0.561 | 0.763 |

Findings:

- **The f ≈ 0.64 per-hop factor is not a per-hop energy loss.** It comes from the light cache stopping after about four rounds with the surface cache off. It is not a sampler, weight, E vs E/π, blend-sum or decay error.
- **With the surface cache on, f = 0.76.** That is still short of the hall's ≈0.95 and of 1, so the gap with the cache on is still open. Those numbers need the same patch probe, run in mode 3.
- **The read-stamp deadlock hypothesis from the probe session is withdrawn.** It was observed in mode 0, where the mode-3 scheduler never runs.
- **No fix and no PR.** Whether mode 0 should keep relighting its light cache is a design question for Anthony, not a bug fix.
