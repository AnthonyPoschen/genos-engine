# Weighted lamp picking implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring the GI light pass on the owner RTX 4070, at 1080p, in the moving stress scene, from 6.7 ms down to about 2 ms, with no quality loss against a full-effort reference.

**Architecture:** Keep the sun exact. Sample point lamps with the systematic weighted picker already in `lamps_light_only` for bounce hits, probe fills, and cache-ray hits, where the probe gather and the surface-cache estimator already average many samples. Leave primary pixels on every lamp until that hit path has been measured. Add screen-space ReSTIR only on pixels, temporal first, one fresh shadow ray per pixel, and only after the partition says pixels are what is left of the 6.7 ms.

**Tech stack:** Vulkan compute in `lighting_direct.comp`, settings on `LightingSettings` (`EngineSettings`), the existing MCP catalogue, the stress debug menu.

## Global constraints

- This document is the plan. It does not change the runtime.
- Default stays preset `high` with picking off, so today's picture and the tuned dispatch list stay put.
- A setting is a public field on `LightingSettings`, round-trips through `to_text` / `parse` / `set_key`, is writable as `set_lighting_settings` over MCP, and is a debug-menu knob. [Settings principles](../settings-principles.md).
- The workspace denies compiler warnings. Implementation finishes only after `cargo build --workspace` and the tests for the crates touched.
- Quality first. A change that saves no measurable time on the owner GPU is not pushed. No visible noise, no torn shadow edge, no new pop.
- Follow the papers. Any departure is called out in this plan and, when the code lands, in the deviations list in [the lighting document](../../systems/lighting/README.md).
- [ADR 0012](../../adr/0012-gi-v2.md) asks for a deterministic per-tile light-tree cut and no radiance history. This plan conflicts with that sentence. The conflict is named below. It does not override the ADR in silence.
- Owner timing is RTX 4070, 1920×1080, the moving stress scene. Lavapipe checks logic and pictures. It does not check milliseconds.
- Low must still run on a GTX 1060-class GPU. Nothing here may require a minimum candidate count. The candidate and ray knobs are the scale.

---

## Recommendation

Do the hit path first. Do not start by repeating S78Q.

The moving scene has about five lamps per pixel. S78Q's pixel reservoirs moved the light pass from 6.59 ms to 6.38 ms and were shelved. The owner's earlier split, whatever its overlap, puts fills and bounce hits above the whole pixel term. One shadow ray per bounce hit, with the sun still traced exactly, is the change that can reach 2 ms. One shadow ray per pixel, without a reservoir and without a direct-light denoiser, is either a stable biased picture (the same lamp forever) or a flickering one. Pixels stay exact until the measured remainder is the pixel term.

The 100-lamp room is the second gate, not the first algorithm. The cheap loop over a lamp cell is not a light tree until a counter shows that loop in the time.

---

## 1. How lamps are stored and shaded today

### Storage

A lamp in the scene is `Light` (`crates/scene/src/types.rs`): a position, a colour, and a direction. A zero direction is a point lamp. A non-zero direction is a sun, and the vector is the direction the rays travel.

`pack.rs` writes the resident scene block. `scene_data.glsl` reads it. Each `Lamp` is two `vec4`s in the tail: `pos.xyz` plus a directional flag in `pos.w`, and `color.rgb` plus the range in metres in `color.w`. The range is `lamp_range`: the distance at which the brightest channel falls to half an 8-bit step on white paint (`LAMP_CUTOFF`). A sun stores range 0 and reaches everywhere. The flame is not a `Lamp`. It is `fire_pos` / `fire_color` on the scene block, appended as one extra list entry.

Point lamps are binned in a grid (`LAMP_CELL` 4 m, at most 128 cells on a side). `lamps_at` returns the suns, then the point lamps whose range can reach the cell, then the flame. `lamp_reaches` is the exact test. `MAX_LAMPS` is 4096. The world-cache lamp snapshot and the shadow-cell epoch cover the first `LIGHTING_WRC_SNAP_LAMPS` (128) only. A 100-lamp room fits. A scene past 128 silently drops epoch tracking for the rest (`lighting_lamps_sig` returns 0, and the texel direct-light cache refuses to hit).

`lamp_light` is the unshadowed shade: cosine times colour times `LAMP_UNIT` (72) over distance squared, clamped at a 0.1 m bulb. Behind the face, or outside the range, it is zero. A sun uses the same unit as a lamp 7 m away and aims the shadow ray 80 m along the light.

The stress building shares one total power (`LAMP_TOTAL` = 1). Five lamps are 0.2 each, range about 43 m. A hundred are 0.01 each, range about 10 m. `--layout first` puts every lamp in section 0. That is the 100-lamp room Anthony asked for during S78Q. `make stress-big` is a different scene: 100 lamps spread over 16 sections.

### The one shade loop

`lighting_direct.comp` is the light pass. One dispatch chain shades, in order:

1. Primary pixels, from the G-buffer (distance, packed normal, albedo, emissive in `w`).
2. Kept probe hits, all eight strata, subject to the relight gate below.
3. Probe fills: strata no neighbour traced, for a surface the camera just revealed (`lighting_fill_ray`).
4. Light-cache ray hits (surface-cache and world-radiance-cache relights), a separate dense dispatch tagged `hit_light`.

A pixel writes irradiance into the G-buffer. Compose adds it on top of the cache. Nothing denoises that buffer. A hit adds `lighting_cache_irradiance_at` and writes the outgoing radiance.

There is no per-pixel motion vector. The G-buffer is distance, normal, albedo, emission. Last frame's camera (eye and basis) sits in the work block at `lighting_change_at() + 1..4`, copied in `lighting_gpu.rs` from the previous scene bytes. Probe gather reprojects through that camera. Pixel direct does not.

### Who traces every lamp

| Consumer | Where | Shadow rays today | History |
|---|---|---|---|
| Primary pixels | mode-1 direct dispatch, `idx < pixels` | Every lamp in the cell, one ray each. `ctrl_max_lights` 0 means all. | S78J reuses last frame's irradiance when the point, the normal, and the lamp epochs match, and the frame is quiet (no change, no mover). Any mover relights every pixel. +7.9 MiB. Still light pass 1.56 ms; moving light pass did not drop (6.31 → 6.50 ms). |
| Kept probe hits | same dispatch, after the pixels | Every lamp. `LIGHTING_PROBE_PICKS` is `false`. | Visibility only, eight strata. Light is reshade. S69C relights the traced stratum and the halfway stratum. S78D skips the halfway relight unless a lamp, the sun, or the sky moved. Moving scene: about 1.08M hits relit a frame; still: about 0.50M (S78J). |
| Probe fills | the second `kept` span of that dispatch | Every lamp, because probe picks are off. | None. A fill lives one frame. |
| Cache-ray hits | `hit_light`, dense indirect | Every lamp. Picks run only when `GENOS_LIGHTING_CACHE_PICKS=1` (push-constant bit 11). Default is off. | Texel direct-light cache (S75): a hit whose texel signature matches reads the stored direct and traces nothing. A stale or changed signature spends a picked check or a full recompute inside `LIGHTING_TDL_BUDGET` (32768). Moving scene: 50.6k checks a frame, about half escalate (S78J). |
| Hit shadow cells | inside the loop, hits only | A 12.5 cm cell per lamp, invalidated by the lamp epoch. Pixels pass `LAMP_ALL` and never read it. | The cell. S77B tried per-pixel shadow reuse, saved no time, and tore moving shadow edges. Reverted. |

The systematic picker is already written. `lamps_light_only` in `scene_rays.glsl`, and the copy of it in `lighting_direct.comp`, walk the list once for unshadowed weight, then once to trace. If the number of positive lamps is at most `LIGHTING_CACHE_PICKS` (2), both loops trace every lamp (`exact`). Above that, the picker places `picks` samples at `(j + u) / picks` along the running weight. A chosen lamp is scaled by `times * step / w`, which is the unbiased RIS weight for a fixed sample count. `u` is hashed from the patch and the ray, or from the hit cell and the hit index, so a still hit repeats. The comment calls this MegaLights / ReSTIR. It is the initial weighted sample only. There is no reservoir, no neighbour merge, and no previous frame.

`exact` is why S60A can record "no gain" and still be consistent with a five-lamp scene. If a hit's cell holds two or fewer positive lamps, the picker traces them all. S78Q later counted about five lamps per pixel, which is above 2, so the flag is worth turning on again. The const `false` also lets the compiler delete the pick branch. A uniform will not.

### Temporal data that already exists

- Last camera, for probe reprojection. Usable for a pixel reservoir. It has no object velocity. A moving box is a change bound and a mover record, not a motion vector.
- Per-pixel direct: signature plus rgb9e5 irradiance (S78J). Not a reservoir. No lamp id, no sum of weights, no `M`.
- Kept-hit visibility, eight frames, world-stable directions. Light is not stored across frames except as the last shade, which is replaced on relight.
- Lamp epochs and the moved-lamp ring (`lighting_wrc_lamp_changed`, `lighting_lamp_delta`). This is the signal that a lamp moved since a given frame.
- Shadow cells and the texel direct-light cache. Both are stored shades, not samples to resample.

S72B put a one-sample reservoir on surface-cache texels (ReSTIR GI, neighbour reuse, no visibility retrace) and was reverted: correlated low-frequency blotches. S78Q put reservoirs on pixels and was shelved. Those are the two reuse attempts. Neither is in the tree.

### Earlier lamp-picking work

| Id | What | Result | Where |
|---|---|---|---|
| S60A | Probe hits take at most 2 shadow rays, systematic, in proportion to unshadowed light. | Pushed as `a2b41c52`, then hardcoded off. Comment: no gain, light pass unchanged. Before: light pass 27–31 ms of a 41–48 ms frame. After: not recorded. | `LIGHTING_PROBE_PICKS = false` in `lighting_cache_slots.glsl` |
| S78Q | Pixel ReSTIR. Q fused spatial. Q2 temporal only. Q3 plus RIS. Q4: 8 candidates, temporal, 1 spatial neighbour. | Shelved. Anthony asked whether it throws lamps away (answer given: no, unbiased weighted picking) and asked for the 100-lamp room as a benchmark. Decision: shelve Q4. | Patch `g78/S78Q4.patch`, not in git. Screenshots `shad-S78Q4/b` on the owner machine. |
| S77B | Reuse pixel shadow rays, with a check. | Reverted `9e242987`. Light pass stayed 8.6–9 ms. Moving shadow edges tore. | — |
| S72B | ReSTIR GI on texels. | Reverted. Blotches. | `b0e76a3b` / `43690446` |
| S78J | Quiet-frame pixel direct reuse. | Pushed `18894551`. Helps a still frame. A mover relights every pixel. | `lighting_pix_reuse_on` |

S78Q numbers, owner GPU, scene and resolution recorded only as in that note: still 8.62 ms / 100 fps, moving 18.9 ms / 50 fps (light 6.38 vs 6.59), pop 0.086/0.122 vs 0.069/0.104, noise 3.67 vs 3.08. The 100-lamp room light pass went 47.9 → 35.2 ms (17 → 21 fps). Memory +16.6 MiB, which is exactly 1920×1080×8 bytes: one reservoir per pixel, two words. Q (fused spatial) washed out shadow wedges. Q2 (temporal only) noise 9.0 vs 3.1. The note's lesson: the moving scene has about five lamps per pixel, and probe-ray hits still looped every lamp.

### The 6.7 ms and the earlier split

Owner profile, stated for this plan: RTX 4070, 1080p, moving stress scene, light pass 6.7 ms inside an 18 ms lighting frame.

An earlier split, in milliseconds: filling empty areas 4.6, every lamp on bounce-ray hits 3.6, per-pixel 2.3, relighting stored hits 2.2, every lamp on pixels 1.9.

Those five numbers sum to 14.6. They are not a partition of 6.7. They are kept as labels for the consumers below, and phase 0 replaces them with a partition of one frame before any algorithm change. The likely mapping:

| Owner label | ms | Code it most likely timed |
|---|---|---|
| Filling empty areas | 4.6 | Probe-fill span of the direct dispatch (`lighting_fill_ray`), possibly including the empty slots. Cache-ray lighting of empty world-cache cells is the other candidate. The trace pass is not the light pass. |
| Every lamp on bounce-ray hits | 3.6 | Kept hits and/or cache-ray hits with a shadow ray per lamp. |
| Per-pixel | 2.3 | The pixel span of the direct dispatch, shade plus rays. |
| Relighting stored hits | 2.2 | The kept hits that survive the S69C / S78D gate. Overlaps the bounce-hit row. |
| Every lamp on pixels | 1.9 | The pixel shadow rays inside the 2.3. |

S78Q already showed that pixel reservoirs, on this scene, are worth about 0.2 ms. A plan that only resamples pixels cannot reach 2 ms.

---

## 2. Papers, and the choice per consumer

### What each paper actually contributes

**Talbot, Cline, Egbert, "Importance Resampling for Global Illumination", EGSR 2005.** Resampled importance sampling. Draw `M` candidates from a cheap pdf `p`, keep one with probability proportional to `w = p̂ / p`, shade it with `f / p̂ * (1/M) * Σw`. Unbiased when `p` and `p̂` are positive wherever `f` is. The systematic picker in `lamps_light_only` is this estimator with `p̂` = unshadowed luminance and `M` = `LIGHTING_CACHE_PICKS`, implemented as systematic (stratified) samples along the weight prefix rather than independent draws.

**Chao, "A general purpose unequal probability sampling plan", Biometrika 1982.** Weighted reservoir sampling. One stored sample plus a running weight sum stands in for the whole candidate stream. Combining two reservoirs treats each stored sample as one candidate with weight `p̂ * W * M`. This is the data structure ReSTIR reuses across pixels and frames.

**Bitterli, Wyman, Pharr, Shirley, Lefohn, Jarosz, "Spatiotemporal reservoir resampling for real-time ray tracing with dynamic direct lighting", ACM TOG 39(4), Article 148, 2020.** <https://doi.org/10.1145/3386569.3392481> PDF: <https://cwyman.org/papers/sig20_ReSTIR.pdf>

The algorithm this plan means by ReSTIR DI:

- Initial candidates from a cheap light distribution. Their implementation uses a light BVH (they compare against Moreau, Pharr, Clarberg 2019). `p̂` is the unshadowed contribution. Visibility is not in `p̂`.
- One reservoir per pixel: the chosen light, the weight sum, and `M`.
- Temporal reuse: reproject with motion vectors, merge the previous reservoir if the surface matches.
- Spatial reuse: merge a handful of neighbours inside a radius.
- Section 3's merge is biased. Where a neighbour's target is zero for the chosen light, the estimator divides by `M` instead of by the count of non-zero targets, and the picture goes dark. The abstract's biased estimator is faster and loses energy.
- Section 4 and Algorithm 6 remove that bias with an MIS weight over the candidate sources. The naive "divide by the number of non-zero pdfs" fix is unbiased and very noisy when a pdf is merely small (their Figure 7c).
- Visibility reuse is a separate choice. Algorithm 5 tests the selected sample, throws the reservoir away if it is occluded, and then lets neighbours inherit the survivor without a new ray. That is what makes reuse cheap. It is also what lags a moving shadow and what leaks light through a thin wall.

**Wyman and Panteleev, "Rearchitecting Spatiotemporal Resampling for Production", HPG 2021.** The production form: clamp `M`, reject bad surfaces, and accept bias to kill boiling. Cited by MegaLights as the reason practical ReSTIR clamps history.

**Lin, Wyman, Yuksel, "Generalized Resampled Importance Sampling: Foundations of ReSTIR", SIGGRAPH 2022.** The unbiased theory behind Algorithm 6, including the cases the 2020 MIS weight still misses. If a later phase claims "unbiased reuse", this is the paper to match, and any shortcut is a deviation.

**Moreau, Pharr, Clarberg, "Dynamic Many-Light Sampling for Real-Time Ray Tracing Using a Light BVH", HPG 2019.** The light BVH ReSTIR uses as its candidate source and as its speed baseline. One stochastic descent per sample. Unbiased when the traversal probability is the pdf.

**Conty Estevez and Kulla, "Importance Sampling of Many Lights with Adaptive Tree Splitting", PACMCGIT 1(2), Article 25, 2018.** <https://doi.org/10.1145/3233305> PDF: <https://fpsunflower.github.io/ckulla/data/many-lights-hpg2018.pdf>

A BVH over the emitters. Each node stores energy, a spatial bound, and a normal cone. At a shading point, one random number walks the tree; the child probability is the predicted importance (power, distance, orientation). Adaptive splitting traces both children when the cluster is too close for the bound to be trusted. No visibility in the importance. This is the candidate distribution for a long lamp list. It is not a cut: every lamp stays in the pdf.

**Yuksel, "Stochastic Lightcuts", HPG 2019.** MegaLights cites this as the light-hierarchy option they surveyed. A cut that is resampled, so the error of Lightcuts becomes noise. Visibility is not in the tree measure.

**Walter et al., "Lightcuts", SIGGRAPH 2005.** The deterministic ancestor of the design doc's tile cut: a hierarchy, a bounded error, one representative per cluster. Biased by the bound. No noise, no history. The bound is "the light left outside the cut is small", which is false when a hundred similar lamps each carry the same power. That is the hell scene.

**Narkowicz and Costa, "MegaLights: Stochastic Direct Lighting in Unreal Engine 5", SIGGRAPH 2025 Advances in Real-Time Rendering.** Slides: <https://advances.realtimerendering.com/s2025/content/MegaLights_Stochastic_Direct_Lighting_2025.pdf>

What they ship, and why it is not "ReSTIR with a denoiser" as a slogan:

- A fixed ray count, about one per pixel on a console, toward a stochastically chosen subset of lights. Cost does not grow with the lamp count.
- They looked at light hierarchies (Yuksel 2019, no visibility) and at ReSTIR, and did not use either as the baseline.
- ReSTIR's reuse wants a visibility ray per reused sample. One shaded sample per pixel becomes two or three rays. That constant cost is the problem they refused.
- ReSTIR repeats a discrete light from history. The repeats are correlated. Their denoiser wants independent noise. Sampling quality and denoiser quality cancel (they cite Bitterli 2022 on that correlation).
- So MegaLights spends the ray on a new sample, and lets the denoiser integrate. Ray guiding replaces reservoir reuse.
- Disocclusion: two reservoirs, one for lights the history thinks are visible and one for lights it thinks are hidden. The hidden reservoir's weight is clamped before the merge, so a newly visible lamp can appear without a firefly.
- History reprojection fails when the object moves or the surface was off screen. They say so.

**Ouyang et al., "ReSTIR GI", HPG 2021.** Reservoirs of indirect paths. S72B tried the texel version and was reverted. Out of scope here. Direct lamps only.

### ADR 0012 and the design doc

[ADR 0012](../../adr/0012-gi-v2.md) and [the GI v2 design](../../systems/gi-v2-design.md) specify a deterministic light-tree cut per 16×16 tile, K = 4 at Low and 16 at Ultra, one shadow ray per chosen node, a 4-frame fade, hysteresis, and no denoiser. The design's reason: a deterministic choice has no noise and no history, and light outside the cut is small by construction.

That cut is not built. The sentence in the lighting document is still "open: light tree for many lamps". This plan does not build it.

The conflict, for review:

- A cut of K lamps still traces K shadow rays at every pixel and every hit. On the moving stress scene K would have to be 1 to approach 2 ms, and a 1-light cut of five similar lamps drops most of the light.
- On the 100-lamp room the lamps share one total power, so each is small and the cut's error bound does not prefer the right one. Stochastic weighting keeps every lamp in the expectation. A cut does not.
- The plan's light tree, if phase 6 builds one, is Conty and Kulla's sampling tree: a pdf for one sample, not a tile cut. Building the ADR's cut instead would miss the time target or the quality target. Review decides whether to amend the ADR or to keep the cut as a later, separate path for a handful of hero lamps.

### Choice per consumer

| Consumer | Choice | Why |
|---|---|---|
| Primary pixels | Stay on every lamp through the hit-path phases. Then, if the partition still has a pixel term worth taking, ReSTIR DI: cell-list candidates, temporal reservoir, Algorithm 6, one new shadow ray, spatial radius default 0. | The direct buffer is shown raw. Fixed-`u` one-sample picking is a stable bias. Frame-varying `u` flickers. MegaLights integrates with a denoiser we do not have on this buffer; S72C's denoiser-as-the-picture was reverted for baked blotches. S78Q already spent the easy pixel win (0.2 ms) and lost wedges to spatial reuse. |
| Kept probe hits | Systematic weighted pick, sun exact, `lamp_pick_rays` point-lamp shadow rays, fixed `u` from the hit. No reservoir. | The gather averages up to 512 directions. That is the integrator. A reservoir per hit would correlate the hemisphere the way S72B correlated texels. MegaLights' own objection to ReSTIR (correlation, extra visibility rays) applies here. |
| Probe fills | The same picker as kept hits. No history. | A revealed surface traces every stratum in one frame, so it already has the probe's full ray count. One weighted shadow ray per fill ray is the same estimator as a mature probe, on the first frame. |
| Cache-ray hits and world-cache fill | The same picker. The texel direct-light cache stays the reuse. | A cache ray is one sample of a texel that already averages sets. A world-space reservoir (Boissé et al., HPG 2021) is a second history on top of MSME and S72B's failure. Empty cells have nothing to reuse. |
| 100-lamp room, the cheap loop | Keep walking the cell until phase 2's counters say the walk dominates the rays. Then a Conty–Kulla tree as the candidate pdf, one sample, sun still exact. | A hundred unshadowed `lamp_light` calls are cheap next to a hundred traces. S78Q's 47.9 → 35.2 ms was pixels only, with hits still walking every lamp. The tree is phase 6, and only if the counters ask for it. |

---

## 3. Failure modes and mitigations

### Fast-moving lamps

The stress scene moves half its lamps every frame. A reservoir that stores a world position goes stale in one frame. Reconnection stores the lamp index and evaluates `lamp_light` at the lamp's position this frame, then traces the shadow ray to that position. The contribution lags by zero frames. Dropping history whenever `lighting_wrc_lamp_changed` is true would also be correct, and it would turn temporal reuse off for the entire owner profile. Reconnection is the recommendation. It is the paper's moving-light path.

Deviation to avoid: inheriting last frame's visibility. S77B did that and the shadow edge tore. Algorithm 5's "discard if occluded, then reuse" has the same shape. The final sample's shadow ray is traced every frame, at the current point, toward the current lamp.

### Disocclusion

Reprojection uses last frame's camera and the current depth. It fails for a surface that was off screen, for a pixel whose normal or plane disagrees, and for anything a mover crossed. There is no motion vector, so an object that moved under a still camera also fails. That is a deviation from ReSTIR, which assumes motion vectors, and it is the data we have.

On failure, `M` resets to this frame's candidates. A one-sample pixel on that edge is noisy. Mitigation, in the reuse phase only: a pixel with no accepted history traces `min(lit, lamp_pick_rays_miss)` rays, default 4, and still never traces the sun twice. The extra rays are the disoccluded set, not the whole image. Probe fills do not use this path. They already trace a full stratum set.

MegaLights' second reservoir (hidden lights, clamped) is the fallback if a disoccluded pixel stays dark for more than a frame after the miss-ray bump. It is not in the first reuse commit. Copying it later is following the talk. Skipping it and still claiming their disocclusion quality would be a deviation.

### Small strong lamps

`p̂` is `lamp_light`'s luminance (cosine, colour, inverse square), never uniform and never power-at-the-lamp. A lamp that is small in the list and bright at the point keeps its weight. Uniform-among-the-cell would miss it until `M` is huge. That mistake is forbidden in review.

A fixed `u` on a pixel never picks the other lamps. That is the stable bias above. Hits escape it because neighbouring rays use different `u` (the hash includes the hit index) and the gather averages them. Pixels do not get a fixed `u` in any mode that is allowed to look different from `off`.

Boiling: a rare bright sample with a huge `M` sticks in a reservoir and pops when it finally dies. `lamp_pick_history` clamps `M`. The paper's unbiased estimator does not need the clamp for correctness. The clamp is a bias (Wyman and Panteleev). Default for the unbiased knob is on, so the clamp is implemented but the unbiased path does not apply it. If boiling shows up in the unbiased picture, that fact goes to review before anyone turns the knob off.

### The 100-lamp room

`--lights 100 --layout first` on the small building. Each lamp is 0.01 of the shared total, range about 10 m, all of them in section 0. S78Q: light pass 47.9 ms down to 35.2 ms with pixel reservoirs, hits still on every lamp.

Failure mode: a cut, or a cap like `direct_lights`, throws away lamps whose sum is the whole room. Anthony's question on Q4 was exactly this, and the answer that got shelved was "unbiased, so no". The hit-path estimator has to stay in that family: every point lamp in the cell stays in the pdf, the sun is stratified out and traced exactly (see deviations), and the shade of a chosen lamp is divided by its selection probability.

The second failure is the walk. A hundred `lamp_light` calls plus one trace, times a million hits, can still miss 2 ms even after the rays are gone. Phase 2 counts walk iterations and trace counts separately. The tree is the response to the walk counter, not a default.

Past 128 lamps, epoch and texel-direct signatures ignore the extras. The hell scene at 100 is inside the cap. A follow-up that raises the room past 128 has to raise `LIGHTING_WRC_SNAP_LAMPS` or stop using the signature as a reuse key. Out of scope until a scene needs it.

### Noise feeding the denoiser

The a-trous filter reads the surface cache. It does not read pixel direct. Noise in a cache-ray's direct term enters the texel mean, then the filter. Independent noise across texels is what that filter is built for. A reservoir shared across cache rays is the correlation MegaLights warns about, and it is the blotch S72B produced. Cache rays therefore do not reuse reservoirs.

Probe hits are averaged in the gather before the probe filter. One weighted sample per hit is independent across direction cells as long as `u` depends on the ray. A shared `u` per probe would paint a lamp choice across the hemisphere. The hash includes the hit index. Keep that.

Pixel noise has no denoiser. The reuse phase is not allowed to "let a-trous clean it up". If the reuse picture is noisy on a still camera, the phase fails. It does not get a new denoiser inside this plan.

### Temporal lag

Lag here is a shadow or a lamp contribution that follows the truth a frame late. Causes, and the rule for each:

- Reusing a shadow result. Forbidden. One new ray for the winning lamp, every shaded point, every frame.
- Reusing an irradiance (S78J). Already shipped for quiet frames. Picking must not extend it to moving frames. A moving lamp changes the epoch, the signature misses, and the pixel shades again. That interaction stays.
- A reservoir that keeps last frame's contribution instead of re-evaluating. Forbidden. Store the index. Evaluate this frame.
- The halfway kept-hit gate (S78D) already skips a relight when no lamp moved. Picking does not change that gate. A hit that is skipped keeps last frame's shade, which is the existing lag bound (at most four frames, and only when the lamps did not move).

### Bias from reuse, and the choice

Two different biases, often mixed up:

1. **PDF bias.** Merging reservoirs whose targets differ, and dividing by `M`, darkens edges and thin geometry (Bitterli section 3). Algorithm 6 fixes it. The fix is noisier.
2. **Visibility bias.** Trusting a neighbour's shadow ray. Light leaks, lagged shadows, washed-out wedges. Q's fused spatial result looked like this. S77B's torn edges were the pixel version of trusting an old ray.

Choice for this plan:

- Hit path, fills, cache rays: no merge. The systematic estimator is unbiased for the point-lamp sum given a random `u`, and the `u` is stable per hit so a still probe does not crawl. Across the hemisphere the probe is a many-sample average. This is the Talbot estimator. No deviation on the weight. The sun split below is the deviation.
- Pixel reuse, when it is built: Algorithm 6, one new visibility ray, `M` not clamped. That is the unbiased paper path. `lamp_pick_unbiased=0` switches to the section-3 merge plus the `M` clamp. It is off by default and labeled biased in the settings text. Shipping the biased merge as the default would contradict the answer given to Anthony on Q4.

Deviations to carry into the lighting document when the code lands:

| Deviation | From | Why |
|---|---|---|
| The sun (and only the sun) is traced exactly and removed from the point-lamp pdf. | Talbot / Bitterli, where every light is a candidate. | A missed sun on a pixel is a dark frame in a direction the stress scene always has. Stratifying one light and removing it from the other estimator stays unbiased. The flame stays in the point-lamp pdf. |
| No motion vectors. Reject history on camera reprojection plus the mover list. | Bitterli, which reprojects with motion vectors. | The G-buffer does not store them. Adding them is a raster change outside this plan. |
| Hits do not reuse spatiotemporally. | ReSTIR's main result. | The consumers already average, and reuse correlated S72B. MegaLights refused the same reuse on a ray budget of one. |
| Visibility is always retraced for the winning lamp. | Bitterli Algorithm 5's visibility reuse. | S77B. Moving shadows. |
| The ADR 0012 tile cut is not what this builds. | ADR 0012, design doc. | A cut cannot be both 1 ray and unbiased on equal lamps. Review amends the ADR or keeps the cut as later work. |
| `lamp_pick_unbiased=0` clamps `M` and uses the naive merge. | Bitterli section 4. | Optional, default off. Wyman and Panteleev. Energy loss is the expected failure, and the wedge test is the gate. |

---

## 4. Settings

One nested struct on `LightingSettings`, next to `DirectSettings`. Not a new `EngineSettings` type. Lamp picking is a lighting knob, and lighting is the reference implementation of the three surfaces.

```rust
pub enum LampPickMode {
    Off,
    Hits,
    Pixels,
    Both,
    Reuse,
}

pub struct LampPickSettings {
    pub mode: LampPickMode,       // lamp_pick: off | hits | pixels | both | reuse
    pub candidates: u32,           // lamp_pick_candidates, 1..=32, default 8
    pub rays: u32,                 // lamp_pick_rays, 1..=4, default 1
    pub radius: u32,               // lamp_pick_radius, pixels, 0..=64, default 0
    pub history: u32,              // lamp_pick_history, M cap, 1..=64, default 20
    pub unbiased: bool,            // lamp_pick_unbiased, default true
}
```

`off` is the preset `high` value. `to_text` prints the six keys. `parse` and `set_key` accept them. `GENOS_LIGHTING_LAMP_PICK` and the matching variables overlay like `GENOS_LIGHTING_DIRECT_LIGHTS`. A bad token fails `set_key` with `unknown lighting setting` or `lamp_pick is off, hits, pixels, both, or reuse`.

Modes:

| Mode | Hits, fills, cache rays | Pixels | Reservoir |
|---|---|---|---|
| `off` | Every lamp, as today. Bit 11 stays off. `LIGHTING_PROBE_PICKS` stays unused. | Every lamp. | None. |
| `hits` | Sun exact. Point lamps: systematic, `lamp_pick_rays` shadow rays. | Every lamp. | None. |
| `pixels` | Every lamp. | Sun exact. Point lamps: `lamp_pick_candidates` cheap samples, `lamp_pick_rays` shadow rays. No previous frame. | None. Diagnostic. Expect either bias or flicker. |
| `both` | As `hits`. | As `pixels`. | None. |
| `reuse` | As `hits`. | As `pixels`, then temporal merge. Spatial neighbours only when `lamp_pick_radius > 0`. | 16 bytes × 2 per pixel. |

`candidates` and `rays` are different knobs on purpose. In ReSTIR, candidates are unshadowed and rays are visibility. S78Q's "8 cand" was the first number. Conflating them makes a "candidates = 8" run trace 8 shadows and miss the time target while looking correct.

The control block is 16 words at stats word 124, and the stats buffer is 144 words, so four words are free (140–143). The four new words are:

- `ctrl_lamp_pick`: mode enum as `u32` (0 off, 1 hits, 2 pixels, 3 both, 4 reuse).
- `ctrl_lamp_pick_n`: candidates in bits 0–7, rays in bits 8–15, unbiased in bit 16.
- `ctrl_lamp_pick_radius`.
- `ctrl_lamp_pick_history`.

`pack_lighting_ctrl` grows from 64 to 80 bytes. `LIGHTING_STATS_WORDS` stays 144. The shader `gstats` block gains the same four fields at the end. `debug_assert` on the Rust size stays.

`off` writes zeros. The shader's `off` branch is the current loop, including the S78J quiet reuse, the texel direct-light cache, and the shadow cell. A uniform branch that is the same for every thread is acceptable only if phase 1's owner timing of `off` matches the parent commit. If it does not, the pick shader becomes a second pipeline and `off` keeps today's SPIR-V. That split is the escape hatch, not the first commit.

Reservoir memory exists only in `reuse`. `lighting_alloc_key` includes the mode's buffer class so a switch reallocates after the frames in flight, the same way a size knob already does. `off` / `hits` / `pixels` / `both` allocate nothing new. `reuse` allocates two screens of 16-byte records (index, `w_sum` f32, `M` u32, target luminance f32): about 66 MiB at 1080p. S78Q fit in 16.6 MiB by packing two words. The 16-byte record is what Algorithm 6 needs. The 8-byte pack is an open question, not the default.

MCP: no new command. `get_lighting_settings` and `set_lighting_settings` already round-trip the text and return the GPU bucket times. The new keys appear in that text.

Debug menu: `examples/stress/src/knobs.rs`, a choice for `lamp_pick` and numbers for candidates, rays, radius, and history, plus a switch for unbiased. They call `set_key`. The menu copy uses the key names a scene file uses.

`direct_lights` stays what it is: a hard cap, which drops lamps. Picking does not read it as a sample count. A test locks that the two knobs do not share a field.

---

## 5. Phases

Each phase is one commit, default behaviour unchanged until someone sets the knob, and verifiable on its own. Owner milliseconds wait for the owner GPU. Lavapipe covers the tests named here.

### Phase 0: Partition the light pass

**Files:**

- Modify: `crates/render/shaders/lighting_direct.comp` (counters only)
- Modify: `crates/render/shaders/lighting_cache.glsl` (four diag slots)
- Modify: `crates/render/src/lighting_gpu.rs` (print the counters next to `LIGHTING_MS` when `GENOS_GPU_TIMES` is set)
- Test: `crates/render/tests/lighting_frame.rs`

No setting. No change to which lamps are traced. Four atomic counters, one per consumer, incremented once per shadow ray and once per live point:

- `pixel`
- `kept` (passed the relight gate)
- `fill` ( `lighting_fill_ray` returned live)
- `cache` (cache-ray hit that did not take the texel-direct reuse)

Plus the dispatched thread counts, which the host already knows: pixels = `w*h`, kept span = `2 * probes * rays`, cache = the dense indirect count.

- [ ] **Step 1:** Add the counters and a lavapipe test that a one-lamp scene increments `pixel` by the number of surfaces that face the lamp, and increments `kept` only for hits the relight gate allows.

Run: `cargo test -p genos-render --test lighting_frame lamp_pick_counters -- --nocapture`

Expected: PASS. A still frame's `fill` count is 0 after the probes have settled. A frame with a lamp behind the shaded face does not increment.

- [ ] **Step 2:** On the owner GPU, one moving 1080p capture and one still capture, `GENOS_GPU_TIMES=30`, preset `high`. Record `direct`, `hit_light`, and the four ray counts. Write the numbers into the experiment note for this phase. That note is the partition. The 4.6 / 3.6 / 2.3 / 2.2 / 1.9 figures are retired once it exists.

If `fill` rays are a small fraction of `fill` threads, the 4.6 ms is an empty dispatch. Phase 2 then starts with an indirect dispatch of live fills, which is a win even with every lamp traced. If `fill` rays are large, phase 2 starts with the picker.

Lavapipe does not time this. Do not treat a lavapipe millisecond as evidence.

### Phase 1: The setting, inert

**Files:**

- Modify: `crates/scene/src/lighting.rs`
- Modify: `crates/scene/tests/lighting_settings.rs`
- Modify: `crates/render/src/lighting_dispatch.rs` (`Ctrl`, `pack_lighting_ctrl`)
- Modify: `crates/render/shaders/lighting_cache.glsl` (`gstats`)
- Modify: `crates/render/src/lighting_gpu.rs` (upload size 80)
- Modify: `examples/stress/src/knobs.rs`
- Modify: `docs/systems/lighting/toggles.md`
- Modify: `docs/systems/lighting/README.md` (one row in the experiment index, status planned)

- [ ] **Step 1:** Failing round-trip test.

```rust
#[test]
fn lamp_pick_round_trips_and_rejects_a_bad_mode() {
    let mut settings = LightingSettings::preset(LightingPreset::High);
    assert_eq!(settings.lamp_pick.mode, LampPickMode::Off);
    settings.set_key("lamp_pick", "hits").unwrap();
    settings.set_key("lamp_pick_candidates", "8").unwrap();
    settings.set_key("lamp_pick_rays", "1").unwrap();
    settings.set_key("lamp_pick_radius", "0").unwrap();
    settings.set_key("lamp_pick_history", "20").unwrap();
    settings.set_key("lamp_pick_unbiased", "1").unwrap();
    let parsed = LightingSettings::parse(&settings.to_text()).unwrap();
    assert_eq!(parsed, settings);
    assert!(settings.set_key("lamp_pick", "restir").is_err());
}
```

Run: `cargo test -p genos-scene --test lighting_settings lamp_pick_round_trips`

Expected: FAIL, unknown lighting setting `lamp_pick`.

- [ ] **Step 2:** Implement the struct, the keys, the env overlay, the control-block words, and the menu knobs. Shader reads the words and ignores them.

- [ ] **Step 3:** `cargo test -p genos-scene --test lighting_settings` and `cargo test -p genos-render --lib probe_still` pass. `cargo build --workspace` is clean.

- [ ] **Step 4:** Lavapipe picture check. A frame with the keys unset matches a frame with `lamp_pick=off` in the direct buffer. Reuse the harness in `lighting_draws_emitted_light_and_bounces_it` rather than a new window.

Owner GPU, after the lavapipe check: one still and one moving `off` capture against the parent commit. A regression in `direct` + `hit_light` greater than noise means the uniform branch has to become a second pipeline before phase 2.

### Phase 2: Hits, fills, and cache rays

**Files:**

- Modify: `crates/render/shaders/lighting_direct.comp`
- Modify: `crates/render/shaders/scene_rays.glsl` (only if the sun split should live in `lamps_light_only` so the CPU tests and the shader share it)
- Test: `crates/render/tests/lighting_frame.rs`
- Test: a CPU replica of the weight, `crates/render/src/lamp_pick.rs` (new, small, no GPU)

The shader change is the existing pick block, driven by `ctrl_lamp_pick` instead of `LIGHTING_PROBE_PICKS` and bit 11, with two edits:

1. The sun loop always traces, and the sun's weight is not in the point-lamp total.
2. `exact` compares the point-lamp count to `lamp_pick_rays`, not to the constant 2.

`u` stays the hash already in the shader (cell or patch, plus the ray index). It does not include the frame number.

CPU replica, used by the test:

```rust
/// Systematic weights for `rays` samples along `weights`, offset `u` in [0, 1).
/// Returns (times chosen, scale) per lamp. Sum of scale * weight equals sum of weight.
pub fn systematic(weights: &[f32], rays: u32, u: f32) -> Vec<f32> { /* prefix, as lamps_light_only */ }
```

- [ ] **Step 1:** Failing CPU test. Three lamps of weight 1, 1, 8 and `rays = 2`. Over 64 values of `u`, the mean scale of the heavy lamp is 8/10 of the total, and every `u` returns scales that sum to the total weight. A sun passed beside the list is not inside that sum.

- [ ] **Step 2:** Wire `hits`. `pixels` and `off` stay on the old loop. Cache-ray hits and fills follow `hits`. The texel direct-light cache still short-circuits before the picker.

- [ ] **Step 3:** Lavapipe. A scene with one sun and three point lamps, `lamp_pick=hits`, `lamp_pick_rays=1`. The test reads back one cache-ray radiance over 64 frames with the hash offset stepped (the test sets the frame; the hash includes the ray index, so 64 rays cover the prefix). The mean is within 2% of `lamp_pick=off` on that same ray set. A single frame is allowed to differ.

- [ ] **Step 4:** Owner GPU. Moving 1080p, `lamp_pick=hits`, rays 1 and rays 2, against `off`. Record light-pass ms, the phase-0 counters, pop, and the moving-shadow smoke (a lamp orbiting a box, screenshot of the shadow edge). gi-check quick tier against the current limits. Push only if the light pass drops and no grade worsens. If the fill counter said the time was empty slots, this commit is the indirect live-fill dispatch instead, and the picker is the commit after it.

Target check: light pass near 2 ms at rays 1. If it is not, write the counter split into the experiment note and stop. Do not start phase 3 to chase a gap the counters put on the walk or on empty dispatches.

### Phase 3: Pixels, no reuse

**Files:**

- Modify: `crates/render/shaders/lighting_direct.comp`
- Test: `crates/render/tests/lighting_frame.rs`

Same estimator as phase 2, on `idx < pixels`, modes `pixels` and `both`. `u` for a pixel is `lighting_unit(pixel_index)` and does not include the frame. That is the stable, biased picture. A second test mode is not added. The bias is the point of the phase: it shows the error of one-sample pixels before anyone builds a reservoir.

This phase is expected to fail the quality bar. A failed gate is a successful experiment. The commit lands behind the knob, default off, and the experiment note says whether `both` is usable or pixels must wait for phase 4.

- [ ] **Step 1:** Lavapipe still frame, one sun and two point lamps of equal power, a pixel that sees both. `lamp_pick=pixels`, `lamp_pick_rays=1`. The pixel equals one of the two lamps plus the sun, not the sum, and a second frame is identical (no flicker from the frame index).

- [ ] **Step 2:** Owner GPU. Still camera, 600 frames, shimmer and the per-pixel variance from the verification plan. Moving shadow smoke. Mean and p99 against `off` and against the path-traced direct reference (`bounces=0`).

Gate to continue: either the pixel term in the phase-0 partition is large enough that a correct pixel estimator would pay for phase 4, or the 100-lamp room at `hits` is still above the budget because of pixels. Otherwise phase 4 waits.

### Phase 4: Temporal reuse on pixels

**Files:**

- Modify: `crates/render/shaders/lighting_direct.comp`
- Modify: `crates/render/src/lighting_gpu.rs` (reservoir buffer, alloc key)
- Modify: `crates/render/src/lighting_dispatch.rs` (alloc key)
- Test: `crates/render/tests/lighting_frame.rs`

Buffer, per pixel, ping-pong: `u32 lamp`, `f32 w_sum`, `u32 m`, `f32 target`. Empty lamp is `LAMP_ALL`.

Algorithm for `reuse`, after the sun is added exactly:

1. Build this frame's reservoir from `lamp_pick_candidates` point lamps. Candidates are the systematic subset of the cell, not a random extra pass, so the cheap walk stays one loop. `M` starts at the candidate count.
2. Reproject the world position through last frame's camera to a previous pixel. Accept if that pixel had a surface, the plane distance is within the gather's rule (the 25 degree normal test is the one `lighting_gather.comp` already uses), and no mover bound contains either point.
3. If accepted, reconnect: `lamp_light` of the stored index at this point, this frame. Combine with Algorithm 6 (unbiased) or the section-3 merge (biased knob). Clamp `M` to `lamp_pick_history` only on the biased path.
4. If rejected, keep this frame's reservoir only, and trace `min(lit, 4)` point lamps instead of `lamp_pick_rays`. The 4 is `lamp_pick_rays` when that knob is already ≥ 4.
5. Trace one shadow ray for the winning point lamp. Occluded means this frame's contribution is 0. The reservoir still stores the index, so the next frame can reconnect a lamp that was shadowed for one frame. Do not store visibility in the reservoir.

Spatial neighbours are not read in this phase. `lamp_pick_radius` is stored and ignored. A test fails if a radius of 8 changes the picture before phase 5.

- [ ] **Step 1:** Lavapipe. Still camera, two frames, equal lamps. Frame 2's mean over a small patch is closer to `off` than frame 1's. A lamp that jumps 5 m between the frames does not leave a shadow at the old position (read the direct buffer, compare to `off` at the new position: the lit side of the new position is lit).

- [ ] **Step 2:** Owner GPU. The verification plan, full set, `lamp_pick=reuse`, radius 0. The wedge scene from S78Q (`shad-S78Q4`) is the picture to reshoot. Washed wedges fail the phase.

### Phase 5: Spatial radius

**Files:** `lighting_direct.comp` only, plus the lavapipe test that phase 4 reserved.

One random neighbour inside `lamp_pick_radius`, the gather's normal and plane tests, Algorithm 6, then the same single shadow ray. Radius 0 is byte-identical to phase 4. Default stays 0.

Stop condition from Q: if the shadow wedge on the owner GPU is softer than `off` by more than the p99 gate, the radius knob stays available and the experiment note says not to default it on. Do not add a second neighbour in this commit. Q4's "1 spatial" is the whole phase.

### Phase 6: Light tree, only if the counters ask

**Files:** new `lighting_light_tree.comp` or a CPU build uploaded next to the lamp tail, plus `pack.rs`.

Build a Conty–Kulla binary tree over the point lamps when the live lamp count is above 32. The walk replaces the cell scan as `p` for the systematic sample. The sun stays outside the tree. The pdf of the chosen leaf is the product of the child probabilities, and the RIS weight uses it.

Do not start this phase on a calendar. Start it when phase 2's walk counter, on `--lights 100 --layout first`, is a large share of the light pass after the ray count is already 1. If the walk is cheap, this phase is not written, and the ADR conflict note says the sampling tree was unnecessary at 100.

The deterministic tile cut from the design doc is not an optional step of this phase. It is a different feature.

---

## 6. Verification

Owner GPU for every millisecond and every picture. Lavapipe for the tests in the phases. "No quality loss" means all of the following, each compared to `lamp_pick=off` on the same binary and to the path-traced reference where one exists.

### Reference pictures

- Engine full effort: `lamp_pick=off`, preset `high`, which traces every lamp. This is the estimator's reference. An unbiased pick converges to it.
- Path tracer: `GENOS_REFERENCE_GPU=1`, the GPU reference in `crates/debug/src/reference.rs`, at its existing cap (0.05 noise or 2048 spp). Direct-only comparisons set `bounces=0` so the cache does not move the error. Full-lighting comparisons use the gi-check bounce count.
- Store the `off` frame and the picked frame from the same camera, same time of day, same lamp clock. The stress clock is deterministic. Do not compare a picked frame to an `off` frame from another day length.

S78Q's wedge shots (`shad-S78Q4/b`) are the picture to reshoot for spatial reuse. The moving-shadow smoke from the S77B postmortem is the picture to reshoot for lag.

### Mean and p99

Use `metrics::rel_err` (`examples/stress/src/gi_check/metrics.rs`) for the mean relative luminance. Add a p99 of the same per-pixel relative error, same denominator as `rel_base`, and print it beside the mean. Do not invent a limit before phase 0 has measured `off` against itself (quantisation, rgb9e5, and the residual of a still frame). The pick's p99 limit is that self-error plus the gi-check mean limit already in force for that row. A mean that moves and a p99 that grows is a firefly, which the mean will hide.

gi-check rows that must not regress: settled mean, bias, blotch, structure, lampret (lights move and return, limit 0.002), the turn run, and the fill run. Quick tier on every phase. Full tier on phase 2 and phase 4 before a default changes.

### Flicker on a still camera

Still camera, 600 frames, after a 60-frame lead-in, lamps frozen (`--dynamic 0`, sun frozen). Per pixel, the variance of linear luminance across the 600, then the mean of that variance and the p99. The existing 60-frame shimmer reads 0 at 8-bit and misses the drift Anthony has seen, so 600 is the window. `off` is the floor. `hits` must match that floor, because `u` does not depend on the frame. `pixels` without reuse must also match it, and the mean-error gate catches the bias of that stability. `reuse` may sit above the floor during the first `lamp_pick_history` frames; the score uses frames 60–600.

Pop keeps its definition: the share of pixel-frames that step more than 0.01 from their 9-frame average, frames 20–80 and 80–320.

### Unfilled surfaces

There is no symbol named "unfilled detector" in the repository. The gate that covers the same failure is the gi-check fill run (`examples/stress/scripts/gi_check/fill.rhai`): frames until the mean absolute luminance against the settled picture stays under 0.005, target 30, plus the pop share. S73A's failure was unfilled texels when turning or walking backwards. Phase 0's `fill` counter is the in-frame version: a pick that skips live fills will show up as a lower ray count and a worse fill-run score, not as a faster pass with a hole in it.

If the owner machine has a detector under another name, point phase 0 at it. Until then the fill run is the detector.

### The hell scene

```text
cargo run --release -p genos-stress -- --lights 100 --layout first --dynamic 50
```

1080p, owner GPU, `GENOS_GPU_TIMES=30`. Record `direct` and `hit_light` for `off`, `hits`, and, once it exists, `reuse`. S78Q's 47.9 → 35.2 ms is the number to beat, and the quality bar is the same mean and p99 against `off`, plus a still-camera variance pass. `make stress-big` (100 lamps spread, 16 sections) is a second row, not a substitute: most of those lamps are outside a 10 m range of the camera.

### The moving scene

The 6.7 ms figure is the owner moving stress scene at 1080p. `perf/moving.rhai` is cited in the S75 note and is not in this repository. The in-repo run is `genos-stress` at 1920×1080, `--lights 5 --dynamic 50`, sun running, boxes moving, 600 frames, frames 300–600 scored the way S78J was. Confirm the exact command against whatever produced 6.7 / 18 before calling the target met. gi-check turn, mix, and lampret cover the cases a single flyover misses.

### Timing

`GENOS_GPU_TIMES` buckets: `direct`, `hit_light`, `shadows` (the sum of those two when shadows are on), plus the phase-0 counters. Report still and moving. A win that shows up only on lavapipe does not count. A win that costs more than a couple of milliseconds on `off` does not land, even if `hits` is faster.

### What lavapipe is for

Run the CPU and picture tests in each phase. Do not tune `lamp_pick_rays` from lavapipe times. Do not accept a quality gate from a lavapipe screenshot of a shadow edge; the software tracer is the same tracer, so the edge position is meaningful, and the time is not.

---

## 7. Open questions for review

1. **ADR 0012.** Amend the deterministic tile cut, or keep it as a separate later path and let this plan ship stochastic sampling? The recommendation is to amend, because a cut of K rays cannot hit 2 ms and stay unbiased on equal lamps.
2. **The 6.7 ms command.** Which binary, which script, still or moving window, and which bucket (`direct`, `hit_light`, or the sum)? The partition in phase 0 is useless if it times a different run.
3. **The earlier split.** Were 4.6, 3.6, 2.3, 2.2, and 1.9 exclusive ablations, pieces of an older frame, or something else? Phase 0 replaces them either way. The mapping in section 1 is a hypothesis.
4. **Unfilled detector.** Is the fill run the right gate, or is there an owner-side check to call instead?
5. **Hell scene.** Is `--lights 100 --layout first` on the small building the room S78Q timed at 47.9 ms? `stress-big` will not match that number.
6. **Sun stratification.** Always trace the sun, and keep it out of the point-lamp pdf? The recommendation is yes. The alternative is to let the picker drop the sun on indoor hits, which is unbiased and will flicker in doorways.
7. **Unbiased default.** Algorithm 6 as the only reuse math, with the biased merge behind a knob, matches the answer given on Q4. If review would rather ship the biased estimator and an energy-loss budget, say so before phase 4. The knob's default should not change halfway through.
8. **Reservoir size.** 16 bytes × 2 screens (about 66 MiB at 1080p) versus S78Q's 8-byte pack (16.6 MiB). The small pack cannot store Algorithm 6's weight and `M` without quantising them.
9. **Phase 3's fate.** If `hits` reaches ~2 ms and the pixel picture is acceptable, do pixels stay exact forever? The recommendation is yes until a scene shows the pixel term in the partition.
10. **`LIGHTING_PROBE_PICKS`.** S60A measured no gain and the const is false. Phase 2 remeasures with the sun split and with rays = 1, on today's five-lamp scene. If it is still flat, the counters decide the next commit (empty fills, or the walk, or the cache lookup). Do not add a tree to explain a flat result.
11. **Motion vectors.** Worth a raster change so reuse can follow a moving box, or is the mover-bound reject enough? The recommendation is the reject, until a moving-box shot shows a pixel trail the reject missed.

---

## File map

| Path | Responsibility |
|---|---|
| `crates/scene/src/lighting.rs` | `LampPickSettings`, text, `set_key`, env overlay |
| `crates/scene/tests/lighting_settings.rs` | Round-trip and reject |
| `crates/render/src/lighting_dispatch.rs` | Control-block words, alloc key |
| `crates/render/src/lighting_gpu.rs` | Upload, reservoir allocation, counter print |
| `crates/render/src/lamp_pick.rs` | CPU systematic weights, for the unbiased-sum test |
| `crates/render/shaders/lighting_direct.comp` | The shade loop, the only trace site |
| `crates/render/shaders/scene_rays.glsl` | `lamps_at`, `lamp_light`, `lamps_light_only` |
| `crates/render/shaders/lighting_cache.glsl` | `gstats` fields, diag slots |
| `crates/render/shaders/lighting_cache_slots.glsl` | Retire `LIGHTING_PROBE_PICKS` once the uniform is the switch |
| `examples/stress/src/knobs.rs` | Debug menu |
| `docs/systems/lighting/toggles.md` | The three surfaces, documented |
| `docs/systems/lighting/README.md` | Deviation bullets when a phase departs from a paper |

## Self-review

- The 2 ms target is owned by phase 2, not by pixel ReSTIR. S78Q's 0.2 ms is why.
- `candidates` and `rays` stay separate through every phase.
- `off` changes no dispatch and no buffer.
- The sun split, the missing motion vectors, the retraced visibility, the unused tile cut, and the biased knob are the deviations. None of them are silent.
- No phase tells the implementer to copy S78Q's fused spatial pass. Spatial is one neighbour, default off, and it stops on the wedge.
