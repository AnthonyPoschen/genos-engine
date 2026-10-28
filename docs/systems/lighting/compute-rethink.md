# Lighting compute rethink: cheapest complete re-light per frame

Status: **study, proposed**. Docs only. Builds on [ADR 0012](../../adr/0012-gi.md), [ADR 0013](../../adr/0013-lighting-invalidation-is-local.md) and the PR #94 design (`convergence-and-frame-cost-design.md`, ADR 0021 proposed).

## 0. Goal (Anthony, 28 Oct 2026)

The main goal is the lowest cost of a **complete, correct re-light every frame**, reached through reuse and better algorithms. If a full re-light is cheap enough, skipping unchanged work is not needed. Change-driven skipping is a **secondary bonus** for power and headroom. It is not load-bearing, and no target below depends on it. Candidates are ranked by **full-recompute cost at matched quality** against the path-traced reference: no look hacks, no flicker or pops, and near-camera error weighted highest.

Targets: lighting about 8 ms on the 4070 (stress scene about 100 fps), basic scene over 500 fps at 1440p with a moving light (about 2 ms for the whole frame, so about 1.2 ms of lighting), and a Low preset for the GTX 1060.

## 1. Today's cost structure

The 4070 numbers are 1080p High on the stress hall, p50, master (PR #94 §1.3): direct 8.1, resolve 4.0, à-trous 3.5, trace 2.6, compose 2.1, total **18.6 ms**. From `path-to-200fps.md`, direct alone launches about 10.4 M threads (2.07 M pixels plus 2 × 4.18 M kept probe hits), à-trous 7 × 1.05 M, and trace 8.9 M.

Why it scales badly:

1. **Cost is per pixel × per lamp candidate × per pass.** Direct traces a shadow ray per pixel per candidate, plus light on every kept probe hit. Doubling lamps or going to 1440p (1.78× pixels) scales it linearly: about 14 ms of direct at 1440p.
2. **Five full-resolution passes** (direct, resolve, 7 à-trous stages, compose) touch every pixel with wide bandwidth-heavy kernels. Indirect light is low-frequency, yet it is computed and filtered at pixel rate.
3. **Noise is fixed in screen space after the fact.** Indirect is gathered noisily and then denoised (resolve + à-trous = 7.5 ms, 40%). Stable estimators would need far less filtering.
4. **Only 6.1 ms of the 18.6 ms is budgeted** (trace, à-trous), so presets cannot reach the 1060.

### How much of the work recomputes unchanged results?

This is an estimate from the stress-hall structure and the frozen-vs-moving times. Nobody has measured it per texel; stage A below adds the counter.

| work | share of 18.6 ms | inputs that change on a typical moving frame | estimated recompute of an unchanged result |
|---|---|---|---|
| direct, static lamp × static receiver | ~50% of direct | none (only movers' shadows and moving lamps) | **~60–70%** of direct |
| hit light on kept probe hits | ~25% of direct | hits on movers, moving lamps | ~70% |
| resolve + à-trous | 7.5 ms | filter input is re-noised every frame by design | most of it is filtering noise the estimator created |
| compose | 2.1 ms | changes when the camera moves | low (it is really per-frame work) |

Overall, **roughly 55–65% of a moving frame recomputes values whose inputs did not change**. The frozen path is 0.43 ms against about 20 ms moving, which is the upper bound. A moving camera does not change world-space light at all. Only screen projection changes.

Under the reframe, the lesson is not "skip it". It is that **the representation forces per-pixel recompute of world-space-invariant quantities**. Move those quantities into a representation where a complete re-light is cheap: fewer samples (texels, probes, reservoirs) than pixels, and estimators with low variance, so less denoising is needed.

## 2. Candidates

Costs are **modelled estimates, not measurements**: ms of lighting for a complete re-light each frame at 1440p on a 4070, and at 1080p Low on a GTX 1060 (about 0.3× the 4070's throughput). Each assumes matched reference error (stage A defines it). The 1060 has no hardware RT, so it uses the software SDF tracer.

### C1. Object/texture-space lighting at a decoupled rate, screen space for detail only

Shade direct and indirect into a world-space surface cache (cards or texels) at a fixed texel density, about 0.25–0.5 M texels in view instead of 3.7 M pixels at 1440p. A complete re-light of every visible texel happens every frame. Screen space keeps only contact shadows, the specular term and a 1-tap texel fetch. This is the Lumen surface cache made per-frame-complete, and texture-space shading (Hillesland & Yang, "Texel Shading", Eurographics 2016 short paper; Baker, "Object Space Lighting", GDC 2016).
- Cost: direct ~2.0, indirect gather into texels ~1.5, screen apply + specular ~1.2: **~4.7 ms at 1440p on the 4070; 1060 ~7 ms at Low** (half texel density).
- Quality risk: texel-density aliasing at near-camera grazing angles and thin geometry; card coverage gaps (Lumen's known weakness). Shadow edges need screen-space refinement.
- Reuses: the existing surface cache, cards, MSME blend, mover attachment (GIBS-style), and the reference compare.
- Bonus: texels whose inputs did not change can skip (power), but the budget assumes all are relit.

### C2. Stochastic direct lighting with reuse (ReSTIR DI / MegaLights-style)

Replace brute per-pixel, per-candidate shadow rays with a fixed **1–2 shadow rays per pixel**, picked by resampled importance from the light tree, with spatial and temporal reservoir reuse. Cost becomes independent of lamp count.
- Cost: direct 8.1 → **~2.0–2.5 ms at 1440p**, plus ~0.8 ms of denoise for the DI signal. 1060: ~3 ms at half rate.
- Quality risk: temporal reuse can lag a moving lamp; mitigate with ReSTIR's visibility reuse plus a confidence reset from change records (ADR 0021's confidence). Noise in penumbrae needs a denoiser that is graded against the reference, not by eye.
- Reuses: the light tree, ADR 0013's planned ReSTIR DI resampling, and the change records.
- Composes with C1 (the shading target can be texels instead of pixels).

### C3. Probe hierarchy for indirect: radiance cascades or SH/SG world probes

Replace per-texel indirect gathering and screen-probe gather + à-trous with a world-space hierarchy whose angular resolution increases with distance (Radiance Cascades), or DDGI-style irradiance probes with visibility, fully re-traced every frame at fixed cost.
- Cost: radiance cascades 3D at a coarse base, ~1.5–2.5 ms; DDGI at 32k probes × 128 rays ~1.0 ms trace + 0.3 ms update; screen apply 0.5 ms. Indirect total **~2–3 ms** at any resolution, versus 10–12 ms today (trace + resolve + à-trous). 1060: ~3–4 ms with fewer probes.
- Quality risk: probe leaks through thin walls (the wall-leak bug class; DDGI's Chebyshev visibility addresses it), and low-frequency only, so near-camera contact GI needs screen-space short rays. 3D radiance cascades are still research-grade (mostly shown in 2D).
- Reuses: world probes (already "answer the rest of each ray"), the SDF tracer, and the shared `hash_cell3`.

### C4. Half/quarter-resolution lighting with a guided upsample

Run indirect, and optionally direct, at half resolution (0.25× pixels) and upsample with a joint bilateral guided by depth, normal and albedo.
- Cost: about 0.35× today's per-pixel passes: **~7–8 ms at 1440p** without other changes.
- Quality risk: edge halos and errors near the camera (where it matters most); ADR 0013 already rejected half-res denoise (S79). Best used only for the low-frequency term inside C1/C3.
- Reuses: everything; smallest code change.

### C5. Neural or other compression of the cache

Store the radiance cache compactly (a neural radiance cache, Müller et al. SIGGRAPH 2021; or SG/SH coefficients per cell) to cut bandwidth.
- Cost: an NRC inference ~1–2 ms on a 4070 with tensor cores; the 1060 has none, so cost is far higher there. SH/SG compression of probe data is cheap and fits C3.
- Quality risk: online training is a trained field standing in for the live answer, which `path-to-200fps.md` constraints and ADR 0012 reject; non-deterministic, hard to grade. **Not recommended** beyond SH/SG encoding.

### Secondary bonus: change-driven skipping

Skip pixels and texels whose inputs did not change. With C1+C2+C3 the full re-light is already within budget, so skipping only saves power and buys headroom (estimated 55–65% fewer dispatches in a still-camera frame). The 500 fps basic scene is the one place where this bonus helps, but the budget below does not count on it.

## 3. Comparison (complete re-light each frame, matched quality, modelled)

| rank | candidate | 4070 1440p lighting ms | 1060 1080p Low ms | quality risk | reuses |
|---|---|---|---|---|---|
| 1 | **C1+C2+C3 combined** | **~4–5** | **~8–10** | medium: card coverage, probe leaks, DI lag; each is gated | surface cache, light tree, world probes, SDF tracer |
| 2 | C1 texture-space alone | ~4.7 | ~7 (indirect still per-texel) | medium | surface cache, cards |
| 3 | C3 probe hierarchy (indirect only) | ~2–3 indirect + today's 14 direct ≈ 16 | ~16 | medium-high (leaks) | world probes |
| 4 | C2 ReSTIR DI (direct only) | ~3 direct + today's ~13 rest ≈ 16 | ~14 | low-medium | light tree, change records |
| 5 | C4 half-res upsample | ~7–8 | ~12 | high near camera | everything |
| 6 | C5 neural cache | ~ +1–2 on top | not viable | high, not gradeable | — |
| today | master | ~30 (18.6 at 1080p × 1.78 for per-pixel passes, rough) | far over | — | — |

Basic scene (one moving light, few surfaces): C2 + C1 at ~1.0–1.5 ms meets the 500 fps target without skipping, borderline; skipping makes it comfortable.

## 4. Recommendation

Move lighting from per-pixel to a **decoupled world-space pipeline**:
- **Direct:** ReSTIR DI with 1–2 shadow rays per pixel (C2). Biggest single win: 8.1 → ~2–2.5 ms.
- **Indirect:** cards or texels re-lit completely every frame at fixed density (C1), fed by a DDGI-style probe hierarchy (C3) for far bounce. That removes most of resolve and à-trous (7.5 ms), because the estimator itself is stable.
- **Screen space:** only contact detail, specular and compose.
- Half-res (C4) only for the probe apply; no neural cache.
- Change-driven skipping later, as a power bonus.

This keeps ADR 0012's rule that stored light never depends on the camera, and replaces ADR 0021's convergence machinery with complete re-lights. Nothing stale is shown, so the checkerboard and mover patchiness go away by construction rather than by blending.

## 4a. Worst case: a light switches on in a dark room

Skipping unchanged work cannot help here. When a lamp turns on in a dark room, every visible pixel, texel and probe changes in the same frame. This is the frame that sets the real budget, and the design has to meet it without spreading the work over later frames.

**Cost of a full re-light at that moment (modelled, 4070 at 1440p):**

| part | calm frame | light-switch frame | why it is the same |
|---|---|---|---|
| direct (C2, ReSTIR DI) | ~2.0–2.5 ms | ~2.0–2.5 ms | fixed 1–2 shadow rays per pixel, whatever the number or state of lamps; the new lamp joins the light tree and gets sampled by importance in its first frame |
| texel re-light (C1) | ~1.5 ms | ~1.5 ms | every visible texel is already relit every frame at fixed density, so there is no backlog of out-of-date texels |
| probe trace + update (C3) | ~1.3 ms | ~1.3 ms | every probe is re-traced every frame at a fixed ray count |
| screen apply + specular | ~1.2 ms | ~1.2 ms | per pixel, no dependence on what changed |
| **total** | **~4–5 ms** | **~4–5 ms** | the calm frame already pays for a complete re-light |

1060 at 1080p Low: the same reasoning gives ~8–10 ms in both cases. Today's master, by contrast, would mark every cache cell out of date at once; the budget controller then either overruns the frame or spreads the refresh over seconds, which is the checkerboard Anthony saw at 100x sun.

**Why the design meets budget then:** the cost is set by fixed sample counts (rays per pixel, texels in view, probe count × rays), not by how much changed. A full re-light is the normal case, so the worst-case frame costs the same as any other. Reuse lowers the cost of every frame instead of being a shortcut that only works when nothing changes:
- light adds up, so the new lamp's share is just one more term in the resampled direct estimate, with no recompute of other lamps' results;
- visibility (what can see what) is separate from lamp intensity, so dimming or switching a lamp reuses the shadow work already cached for it;
- neighbouring pixels and the previous frame share reservoir samples, so 1–2 rays per pixel are enough.

**What is not free in that frame (quality, not cost):**
- Temporal reservoirs and probe history describe the dark room. A change record from the switch has to reset their confidence (ADR 0021) so the old darkness does not lag in; the first frame then uses spatial reuse only and is noisier.
- If texels gather bounce light from last frame's probes, multi-bounce light builds up over a few frames (one bounce per frame). The first frame shows correct direct light and first bounce, and the rest converges within about 3–5 frames, blending only finished values, so no grid.
- Stage A adds a dark-room light-switch scenario that measures the cost of the switch frame, the frames until error is under the threshold, and grid and flicker scores. Those numbers replace these estimates.

## 5. Staged migration (each step checked against the reference)

| stage | what | gate |
|---|---|---|
| A. Harness and oracle | Per-frame reference path-trace of fixed camera paths (sun cycle at 100x/10x/1x over at least one full rotation, spinning object, dark-room light switch, camera turn); a near-camera-weighted error metric (FLIP or relative MSE weighted by 1/depth); a "recompute of unchanged" counter; the PR #94 grid metric | runs on lavapipe and the 4070; today's master scored as the baseline |
| B. Design ADR | ADR for the decoupled pipeline, accepted by Anthony | review |
| C. ReSTIR DI behind a flag | replace brute direct | error ≤ master's, grid ≤ 0.15, direct ≤ 3 ms at 1440p on the 4070 |
| D. Full-coverage card re-light | every visible texel relit each frame at fixed density | spin settle ≤ 1 frame, grid ≤ 0.15 at 100x, no leak test regressions |
| E. Probe hierarchy for far bounce | DDGI probes with visibility, then try radiance cascades | wall-leak tests pass; indirect ≤ 3 ms |
| F. Remove resolve/à-trous stages that are no longer needed | one stage at a time | error unchanged within noise |
| G. Low preset on a 1060-class budget | densities and probe counts | ≤ 16 ms modelled, measured if hardware is found |
| H. Bonus: change-driven skipping | power and headroom | identical images to the full re-light (bit-exact or within ε) |

Each stage is a small PR with an A/B flag. A stage that misses its gate is dropped under the standing drop rule.

## 6. What can't be measured yet

- All ms in §2–3 are models from launch counts and today's per-pass ratios, not measurements; stage A makes them measurable.
- The "unchanged recompute" fraction is estimated; no per-texel counter exists yet.
- No GTX 1060 is available; its numbers are scaled from throughput.
- 3D radiance cascades have little published production data.

## Sources

- Wright, Narkowicz, Kelly, "Lumen: Real-time Global Illumination in Unreal Engine 5", SIGGRAPH 2022 Advances: https://advances.realtimerendering.com/s2022/index.html
- Narkowicz, Costa, "MegaLights: Stochastic Direct Lighting in Unreal Engine 5", SIGGRAPH 2025 Advances: https://advances.realtimerendering.com/s2025/index.html
- Bitterli et al., "Spatiotemporal reservoir resampling for real-time ray tracing with dynamic direct lighting" (ReSTIR), SIGGRAPH 2020: https://research.nvidia.com/publication/2020-07_spatiotemporal-reservoir-resampling-real-time-ray-tracing-dynamic-direct
- Ouyang et al., "ReSTIR GI", HPG 2021: https://research.nvidia.com/publication/2021-06_restir-gi-path-resampling-real-time-path-tracing
- Majercik et al., "Dynamic Diffuse Global Illumination with Ray-Traced Irradiance Fields" (DDGI), JCGT 8(2), 2019: https://jcgt.org/published/0008/02/01/
- Sannikov, "Radiance Cascades: A Novel Approach to Calculating Global Illumination", 2023: https://github.com/Raikiri/RadianceCascadesPaper
- Hillesland, Yang, "Texel Shading", Eurographics 2016 short paper: https://diglib.eg.org/handle/10.2312/egsh20161018
- Baker, "Object Space Lighting" (Oxide), GDC 2016 session "Object Space Rendering in DirectX 12"; slides: https://www.oxidegames.com/wp-content/uploads/2016/03/Object-Space-Lighting-Rev-21.pdf (session page: https://www.gdcvault.com/play/1023511/Advanced-Graphics-Techniques-Tutorial-Day)
- Müller et al., "Real-time Neural Radiance Caching for Path Tracing", SIGGRAPH 2021: https://arxiv.org/abs/2106.12372
- Halén, Hayward et al., "Global Illumination Based on Surfels" (GIBS), SIGGRAPH 2021 Advances: https://advances.realtimerendering.com/s2021/index.html
- Andersson et al., "FLIP: A Difference Evaluator for Alternating Images", HPG 2020: https://research.nvidia.com/publication/2020-07_flip
