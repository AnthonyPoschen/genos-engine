# GIBS: Global Illumination Based on Surfels (EA SEED / Frostbite), research note

> Research note copied from the shared research store on 2026-10-14. Video: <https://www.youtube.com/watch?v=h1ocYFrtsM4>. Slide frames are not committed; open the video at the listed timestamps (`[m:ss]` = `&t=<seconds>s`).


Source: SIGGRAPH 2021 Advances talk, Henrik Halén (SEED) + Kyle Hayward (Frostbite), with Andreas Brinck and a Ripple Effect colleague (captions say "Shangshan Bay"; spelling uncertain) [0:07–0:20]. Video: https://www.youtube.com/watch?v=h1ocYFrtsM4. MSME reference code: Ray Tracing Gems ch. 25 (see §6).
Transcript: `./gibs-transcript.txt` (timestamps below are `[m:ss]` from it). Auto-captions: "circles"/"servos" = surfels.
Written 2026-10-14 for genos-engine gi-v2 (tip 2abeb0dd). Verdicts: **NOW** = roll in during current work, **LATER** = after perf gate / when usage allows, **NO** = not for us (with reason).
Grounding: every line of the transcript was read in full (2026-10-14). Statements tagged with [m:ss] are what the talk says. Anything tagged *(not in talk)* is my background knowledge or inference. Confidence: talk content is from the transcript; related work is from public papers and my background knowledge. Slide frames were viewed on 2026-10-14 (§6). Slide content is tagged [slide m:ss]. Anything marked (unverified) needs a source check before anyone builds on it.

## 0. Lineage and later evolutions
| Work | Year | Relation |
|---|---|---|
| PICA PICA (SEED), primarily Tomasz Stachowiak [0:52–1:01] | 2018 | Original surfel GI with a uniform grid [8:17]. The talk refers to Tomasz's 2018 talk for spawning [4:20] and MSME [16:11]. Halcyon is *(not in talk)* |
| GIBS (this talk) | 2021 | Adds transform/bone attachment, non-linear grid, depth moments, 6x6 ray guiding, irradiance sharing, many-light (SLC/RIS), SH clipmap probes for transparents |
| Stachowiak's MSME code on GitHub (talk [16:17]) | 2018+ | Reference integrator. Look for the "multi-scale mean estimator" shadertoy/gist by h3r2tic (unverified URL) |
| Embark "kajiya" (Stachowiak) | 2021–22 | Open-source successor. Its irradiance cache is world-space clipmap probes plus ReSTIR GI, not surfels: he moved away from surfels |
| UE5 Lumen surface cache (Wright, SIGGRAPH 2022 "Lumen: Real-time GI in UE5") | 2022 | Mesh cards (captured surface atlases) instead of surfels. Same idea: cache radiance on surfaces, persistent, view-independent. Plus screen probes and a world radiance cache |
| GI-1.0 (Boissé et al., **AMD**, 2023, "GI-1.0: a fast scalable two-level radiance caching scheme") | 2023 | Correction to the brief: GI-1.0 is AMD (Capsaicin), not SEED. Screen probes + hash-grid world cache, fully dynamic, no surfels. Closest to gi-v2 |
| NVIDIA SHaRC / NRC; ReSTIR GI (Ouyang 2021) | 2021–23 | Hash-grid radiance caches; spatiotemporal reservoir reuse. Talk [27:28], [45:57] point this way as future work |

**Takeaway:** the industry moved from "surfels as the cache" to "screen probes + world hash/clipmap cache" (Lumen, GI-1.0, kajiya). gi-v2 is already on that branch, so we take GIBS's *integration/estimator/attachment* ideas, not its surfel data structure.

## 1. Technique-by-technique

Format: **ts**, how it works, gi-v2 mapping, **verdict**, deviation flags.

### 1.1 Surfel definition [2:57–3:19]
Position + radius + normal, approximating a small surface neighbourhood. Like a pixel, but on geometry.
gi-v2: our surface-cache entries/slots and world probes (1 m lattice, 4 m bricks, six-face cubes + hit distances) play this role. Ours are lattice/mesh-anchored, not disc-shaped.
**NO** as a new primitive (ADR 0012 already chose surface cache + screen probes). **NOW** as vocabulary: give cache slots an explicit normal + radius so the apply and sharing weights can use them (see 1.9, 1.11).

### 1.2 Persistence and opportunistic spawn from the G-buffer [1:53–2:26, 3:21–3:56]
Surfels spawn as geometry enters view and persist when it leaves, so the work isn't redone. No bakes, no UVs, no special meshes [0:42], [2:43].
gi-v2: matches our "probes never throw light away" (10-08) and persistent world probes. Pre-fill is our counterpart to "spawn fast enough that you never see gaps" [3:50].
**Already ours.** Deviation: GIBS starts empty and lets you watch it converge (3.5–10 s [40:31], [41:30]). Our rule says pre-fill is required, so we must keep pre-fill and not copy their cold start.

### 1.3 Spawning: 16x16 tile gap fill [3:57–4:27]
The screen is split into 16x16 tiles. Each tile finds its least-covered pixel, and if that coverage passes a *randomised* threshold, spawns one surfel from G-buffer data. It stops once the tile is covered.
gi-v2: our equivalent is the smart-wake / allocation of cache slots for visible surfaces. Useful idea: a **per-tile coverage measure with a randomised threshold** limits allocation per frame (≤1 per tile per frame), which avoids spikes when a lot is revealed at once.
**LATER** (for PR #37/#39 if they touch slot allocation). Make it a public config/debug/MCP setting: `--cache-spawn-tile 16`.

### 1.4 Recycling: GPU free-stack + probabilistic heuristic [6:27–8:10]
Fixed pool allocated up front. A free-list stack uses an atomic counter (decrement to spawn, increment to recycle). The recycle probability combines live count, time since the surfel last contributed, and distance, and is compared against a uniform random number.
gi-v2: our slots have a cap and switch dead probes off. The **"last contributed to the image" age plus stochastic eviction under pressure** is better than a hard LRU/distance cut, because it doesn't cause eviction waves.
**LATER**: slot eviction score = f(pool fill, frames since last applied, distance) vs rand. Fits the "view coverage × likely error, with aging" single score from 10-09. Flag: stochastic eviction is non-deterministic, so tests need a fixed seed.

### 1.5 Attach to transform / bone ID [4:27–5:45]
The G-buffer stores a per-object transform ID, which the surfel stores at spawn along with its *object-relative* position. Each frame, world pos = globalTransforms[id] × localPos. Skinned meshes write the ID of their **highest-weight bone**, giving one-bone skinning of the cache; this is slightly off, but the apply step is forgiving.
gi-v2: this is the root cause of **PR #31 moving-box lag**: our cache entries for movers are world-anchored, so lighting smears or lags while the object moves and must re-converge. Attaching mover cache entries to the object's transform ID makes their gathered irradiance travel with the object; only the *incoming* light changes, which the adaptive blend (1.8) handles.
**NOW, top pick**. Fits our 10-09 layered design ("moving objects carry their own record, never dirty probes"). Needs: object/transform ID in the G-buffer (or an existing instance ID), a GPU transform buffer (current + previous), and local-space storage for mover entries. Flag: GIBS attaches *all* geometry. We do it for movers only, because static stays world-lattice per ADR 0012. Note this deviation.

### 1.6 Screen-relative size [5:50–6:25]
Radius scales so the screen projection is about constant. Approaching an object shrinks surfels and spawns more; backing away grows them and recycles the extras. This gives constant quality and constant cost at any distance.
gi-v2: our near 0.5/1 m cells plus 1 m world lattice plus coarse far volume is a discrete version of this.
**NO** for the lattice (it would break world-fixed probes, which fixed camera-move perf on 10-07). **LATER**: cache-slot LOD picked by projected size is a valid idea for the surface cache.

### 1.7 Non-linear trapezoid acceleration grid [8:10–11:05]: **LATER, big-world track**
**What the talk says.** PICA PICA used a uniform grid [8:17–9:05]: each frame every surfel is inserted into its cell plus any neighbouring cell its radius overlaps, with radius ≤ cell side, so only the 26 neighbours ever matter. That breaks at game scale [9:08–9:29]. Screen-relative radii make far surfels huge, so a uniform cell sized for them is far too coarse near the camera. GIBS's answer [9:41–10:13]: a **uniform cube grid around the camera** plus **six trapezoidal grids, one per ±X/±Y/±Z face** (frustum-shaped pyramids reaching out from the cube faces). Slice thickness grows with distance from the centre. Each pyramid is a regular grid behind a non-linear transform, so insert and lookup are still O(1) [10:07]. On screen, cells look the same size near and far ("optical illusion" [10:30–10:44]). Debug views: cell colours per region, and a surfels-per-cell heatmap that is denser where geometry is complex [10:48–11:05].

**What the talk does and doesn't specify.**
- Stated: a uniform grid "for an area close to the camera" [9:43–9:47], and trapezoidal grids "along each principal axis" [9:47–9:49] whose "thickness of the slices … increased with distance from the center of the structure" [9:51–9:55].
- Stated: it has properties "similar to the projection transform", meaning more resolution near the camera [9:30–9:34], and it "perfectly fits the way circles grow with distance" [10:04–10:05].
- Stated: the trapezoidal grids keep a constant on-screen cell size regardless of distance [10:30–10:36]. The grey central grid "grows linearly" [10:21–10:24]. That wording is ambiguous: it may mean the central grid is linear/uniform.
- Stated: every surfel is re-inserted each frame, plus neighbour overlap, with radius ≤ cell side [8:33–8:56]. That was described for the PICA PICA uniform grid. For the new grid the talk only says lookups and insertions are "still very fast" [10:13].
- **Not stated:** the slice-thickness law, cell counts, the cube size, the per-face lateral resolution, memory, and timing.
**My reconstruction (inference, not from the talk).**
- The structure is camera-centred and rebuilt every frame (all surfels re-inserted [8:33]), so it never "moves". There is no toroidal shift, unlike clipmaps.
- Cube: side `2·R0`, cell `c0` (uniform, about the near surfel diameter).
- For a point p = x − cam outside the cube, the dominant axis `a = argmax|p_i|` picks the pyramid. Depth `d = |p_a|`, lateral coords `u = p_b/d`, `v = p_c/d` lie in [−1,1], so lateral cell count is constant per slice. That is the "trapezoid".
- Slice law, my choice: thickness ∝ d, to match surfel radius ∝ distance [10:04]. Depth slice `k = floor(log(d/R0)/log(1+s))`, with `s ≈ c0/R0`, gives geometric thickness `t_k ≈ s·d`. Cell world size then grows linearly with distance, like the screen-constant surfel radius [5:50–5:58]. If instead thickness grew by a fixed step per slice (t_k = t0 + k·Δ), the slice count would grow as about √D rather than log D. The talk doesn't say which.
- Cells needed out to distance D: `6 · N_lat² · log(D/R0)/log(1+s)` + `(2R0/c0)³`. **Logarithmic in view distance**, vs `(2D/c)³` for a uniform grid.
- Example (illustrative): R0 = 16 m, c0 = 0.5 m gives cube 64³ ≈ 262k cells. N_lat = 64, s = 1/32 makes about 220 slices to 16 km, so 6·64²·220 ≈ 5.4M. Shrinking N_lat to 32 or s to 1/16 gives about 0.7M. A uniform 0.5 m grid to 1 km would be about 6.4×10¹⁰ cells, which is impossible. The trapezoid makes horizon distance almost free.

**Memory (inference; not in the talk).** Per cell, a count/offset (8 B) in a compact prefix-sum layout, plus a flat index list of total `Σ surfels × cells-overlapped` (typically 1–4, ×4 B). With about 0.5–1M cells (about 4–8 MB) plus a 2M-entry list (8 MB), the total is about 12–16 MB, independent of world size, because it depends only on view-distance log and live surfel count. The pool itself is fixed-size [6:35].

**Cost (inference; the talk gives no grid timing).** Per frame: clear counts O(cells), count pass O(surfels), prefix sum O(cells), scatter O(surfels). The lookup is a few ALU ops (abs/max/div/log) plus 1 indirect read. Expected cost is about 0.1–0.3 ms on a mid GPU for about 1M surfels (estimate, not from the talk). Seams: the cube↔pyramid boundary and the pyramid↔pyramid diagonal edges are where neighbour queries need care. Overlap insertion handles it if the radius ≤ the local cell size.

**gi-v2 mapping.** Today our world probes are a fixed 1 m lattice to 50 m plus a coarse far volume, with no per-frame rebuild. In a big world (km-scale), the 50 m lattice + far volume is the part that will strain, as either a seam or memory. Two possible uses: (a) as a **camera-relative far-field index** replacing the coarse far volume, holding sparse far cache entries with log-distance cells, or (b) if we ever adopt disc-like cache entries, as their neighbour structure for sharing (§1.14) and ray binning (§1.15). It does not replace the world-fixed near lattice: that fixed camera-move perf on 10-07, and a camera-relative structure changes cell membership as the camera moves. That is fine for an index rebuilt each frame, but wrong for storing radiance.
**Deviation flags.** (1) A camera-relative rebuild every frame vs our world-fixed persistent storage: index only, never storage. (2) It is not in our papers or ADR 0012, so it needs an ADR addendum if adopted. (3) Slice law, sizes, memory and cost figures above are all my inference. The talk gives none of them [9:41–11:05].

**Proposed big-world test scene: `bigworld_valley`** (procedural, deterministic seed, no baking)
- A 4×4 km heightfield, with hills to block and reveal like the talk's hilltop shot [6:02].
- 2,000 instanced rocks/trees at density falling with distance.
- A village cluster with 100 lamps (reuse the PR #43 stress lamps) 1.5 km from start.
- An enclosed building (dark-room test at scale).
- 20 moving boxes.
- Sun + sky.
- Scripted 60 s camera path: walk → hilltop reveal of horizon → fast fly 2 km to the village → enter building.
- CLI `--scene bigworld_valley --path fly60`. Output to ./genos-debug/bigworld/ with --out.

**Metrics (perf-gate style compact pass/fail):**
1. fps p50/p1 still + along the path (200+ target on the moving scene, checked on zanven-pc only).
2. GI GPU ms split: index build / lookup / gather.
3. GPU memory for GI structures vs view distance (500 m, 2 km, 8 km). Pass if growth is ≤ log, i.e. < 1.5× from 2→8 km.
4. Far-field picture error vs a high-sample reference at 3 distance bands (near / 200 m / 2 km). Pass if p99 is within the cross-process noise floor ×2.
5. Seam metric: max luminance step across near↔far and cube↔pyramid boundaries on a flat plane.
6. Reveal convergence: time-to-90% after the hilltop reveal (pre-fill must hold, so no visible empty frame).
7. Pops: count of frames with > X% screen luminance jump not caused by a light change.
A/B: current lattice + far volume vs trapezoid far index, same seed and path.
**Status: not launched (per Anthony). Build the scene first: it is useful on its own, because big worlds are untested.**

### 1.8 Multi-scale mean estimator (MSME): adaptive blend [15:16–16:26]
**Slide [15:40]:** "Modified exponential moving average estimator [BarréBrisebois2019]", with code at github.com/Apress/ray-tracing-gems/…/Ch_25_Hybrid_Rendering_for_Real-Time_Ray_Tracing/MultiscaleMeanEstimator.hlsl (saved here). **The actual formula from that code:**
- Firefly clamp: `y ≤ 0.1 + shortMean + 8σ`. This is biased by design, so flag it.
- `shortMean = lerp(shortMean, y, 0.08)`.
- `variance = lerp(variance, (y−shortMeanOld)(y−shortMeanNew), 0.04)`.
- `relDiff = luma(|mean−shortMean|/σ)`, and `inconsistency = lerp(inconsistency, relDiff, 0.08)`.
- `vbbr = clamp(luma(0.5·shortMean/σ), 1/32, 1)` (smoothed 0.1). This means noisy signals blend more slowly.
- `alpha = clamp(smoothstep(0, 1, relDiff·max(0.02, inconsistency−0.2)), 1/256, 1)·vbbr`, then `mean = lerp(mean, y, alpha)`.
- State: 3+3+3+1+1 = 11 floats per entry. Open question 1 is answered.

A plain EMA forces a choice between reactive and converged. MSME keeps a long-term mean plus a **short-term mean and a short-term variance**. When the short-term mean departs from the long-term mean by more than the short-term variance can explain, the long-term blend factor rises (react fast). Otherwise it decays toward 1/N (converge to noise-free). The integrator is reused for the SH probes [34:35].
gi-v2: this directly fixes the **slow dark-room fade** (fixed 100–200 ms hysteresis plus "snap on big change", which also caused the 10-10 sun-4x fade-then-pop bug). Replace both with one MSME blend per cache entry/probe. It also removes the pop, because there is no separate snap rule.
**NOW, top pick (paired with 1.5)**. Storage: +2 values per channel (short mean, short variance) per entry, packed in fp16. Flag deviation: our papers/ADR specify hysteresis + snap. MSME replaces that, so record it in the ADR. Must keep "only finished values are published": MSME output is still the published long-term mean.

### 1.9 Applying surfels to pixels [11:09–11:40]
Reconstruct world pos, look up the grid cell, and loop over its surfels as virtual point lights, weighted by orientation (normal dot) and distance.
gi-v2: screen probes / surface-cache gather already do this. **Already ours.** Add a normal-similarity weight if missing.

### 1.10 Depth moments + Chebyshev leak test [11:40–13:36]
**Slide [12:58] "Radial Gaussian Depth":** E(x)≈Z1=Σz/n, E(x²)≈Z2=Σz²/n, σ²=Z2−Z1², and P(x≥t) ≤ σ²/(σ²+(t−μ)²). It cites [Donnelly2006] VSM, [Majercik2019] DDGI and [Chebyshev1867]. "Values are filterable!" 4x4 texels per hemisphere.
Surfels leak light, e.g. outdoor surfels lighting indoor walls (blotches). Each surfel keeps a **4x4 hemispherical depth map of moving-average depth and depth²** (initialised to the radius, updated from its own GI rays that hit within the radius). At apply time, a Chebyshev inequality gives a soft visibility weight, like VSM/DDGI. Only depth inside the radius and only the hemisphere matter, so 4x4 is enough.
gi-v2: our world probes already store hit distances (DDGI-style), so check that we use **mean + mean²** and Chebyshev rather than a hard compare. For surface-cache entries and the **box-foot contact strip / dark-hall / bright-corner leaks**, a tiny per-entry moment map is cheap.
**NOW (if PR #39/#37 is the leak work) or LATER**. Flag: it is a DDGI deviation only in resolution (4x4 hemi vs 8x8/16x16 octahedral full sphere). This is acceptable and should be noted.

### 1.11 Lighting integration: rays, sky, shadow rays, infinite bounce via cache [13:40–15:12]
Random rays: a miss reads the sky, and a hit evaluates direct light with shadow rays plus the **cached irradiance of surfels at the hit point**, which gives infinite bounce over time.
gi-v2: same as our world-cache feedback. Our 3–6 bounce slider question: GIBS shows that cache feedback makes the bounce count implicit. **Already ours**. Keep the bounces slider as a cap on feedback weight.

### 1.12 Variance-driven ray counts, dormant state, global budget [16:26–18:19]
Rays per surfel per frame scale with its MSME variance: high on change or new spawn, near-dormant (one ray every few frames) when converged, but still enough to *detect* change. A global user ray budget scales all requests to give predictable cost.
gi-v2: this is our **smart wakes**, but driven by a measured statistic instead of event heuristics. Also our "probes that never finish" bug (10-10): the variance gives a finish criterion. The debug view is a blue→red variance heatmap [16:58].
**NOW (pairs with 1.8, same data)**: expose `--gi-ray-budget`, `--gi-dormant-rays`, and a variance debug view. Deviation flag: a stochastic dormant ray every few frames is **not** reusing or holding a frame. It is a fresh trace, so it is allowed. Pre-fill still gives the initial value.

### 1.13 Cosine sampling → 6x6 ray guiding [18:19–25:14]
**What the talk says, in order:**
- [18:20–18:43] Uniform hemisphere rays are the naive option. The BRDF is Lambertian, so they importance-sample the cosine lobe.
- [18:43–19:26] This fails when the light isn't along the cosine lobe. Example: two surfels with the *same number of rays in the same directions*, where one hits the sunlit wall twice as often. That shows up as noise.
- [19:41–19:54] Sending many more rays is too expensive, so the goal is to send more rays where the light comes from.
- [19:56–20:37] It is inspired by Müller et al. "Practical Path Guiding" and "Probability Trees" [McCool1997]. **Correction: the caption "mikula dal" is \"McCool et al.\"** [slide 20:20]. Müller's method: a spatial binary tree, with a directional quadtree per node subdivided until leaves hold equal radiance, then sampled with McCool's technique.
- [20:39–20:58] They don't build a quadtree per surfel per frame. They map the hemisphere to a quad (the slide cites Cigolle2014's unit-vector survey and shows octahedral-style mappings [slide 20:50]) and track irradiance per direction in it ("updating their irradiance maps").
- [21:01–21:16] The size is configurable, usually **6x6**. It is **8 bits per component plus a single 16-bit scale per surfel**, and **the entire function is normalised after each iteration**.
- [slide 21:10] More precise than the speech: "Every surfel produces a **moving average 6x6 luminance map** over its hemisphere". It is **one channel (luminance), 8 bits per texel**, "stored in a single 4K texture (could support 7x7 for all surfels)", and the function is normalised **every frame**. So the update *is* a moving average (rate not given), and storage is 36 B + 2 B.
- [21:29–22:47] Sampling: multiply a uniform random number by the sum, walk the cells accumulating until the running sum reaches it (the start cell doesn't matter), get a uv, then apply the inverse hemisphere→quad transform.
- [22:50–23:11] It is hierarchical: a linear-filter tap at pixel intersections, a 3x3 walk then a 2x2 walk.
- [23:14–23:25] "If we account for the PDF … at each step", and the PDF equals the function value, then it is "unbiased".
- [23:28–24:33] HDR-map testbed, plotting relative error vs samples. Guided "after a number of initial samples … learns where to send rays" and converges much faster than cosine.
- [24:35–25:14] There is no gain for uniform light. Small bright sources are "too low resolution" for the guide, but analytic lights are sampled directly.
- Future work: share guiding across surfels [45:20], use ReSTIR-like methods to augment it [45:27], add guiding to the probes [46:19].

**Not said anywhere in the talk:**
- The update/blend rate of the map (the slide only says "moving average").
- Any mixing with cosine or uniform rays.
- Determinism or seeds.
- How new lights are caught.
- Interaction with MSME or variance ray counts.
- Time cost. Only storage is given: 36 B + 2 B per surfel.

**Earlier cons checked against the talk:**
1. *Determinism / reference tests.* **Not addressed.** Everything in GIBS is stochastic ([4:08], [8:03], [14:16]). Guiding also adds history dependence (the ray set depends on past hits). Mitigation *(not in talk)*: a seeded RNG plus statistical checks against our cross-process noise floor.
2. *Bias / lock-in on new light.* **Partly addressed.** Unbiasedness [23:14] requires a nonzero PDF wherever there is radiance. But 8-bit cells normalised to the map [21:07–21:16] can round dim directions to 0, which is biased and never re-sampled. The talk names no floor or mixing. New *analytic* lights are covered by direct light sampling [25:08], since the guide only steers indirect rays. A newly lit indirect surface in a zero cell would be missed. MSME/variance [16:26–17:44] react only to what rays actually see. Mitigation *(not in talk)*: an ε floor plus a cosine fraction (about 25%) with a one-sample-MIS PDF.
3. *PDF correctness.* **Addressed in principle** [23:14–23:25] (the per-level PDF in the 3x3→2x2 hierarchy). Missing: the hemisphere→quad Jacobian (the octahedral-style mapping [slide 20:50] is not equal-area, so the Jacobian is needed), the bilinear-tap PDF, and the zero cells. We need a unit test (integrates to 1) and a copy of their HDR testbed [23:28].
4. *Overlap with adaptive blend and variance rays.* **The talk treats them as separate and complementary.** MSME decides how fast to trust samples [15:28–16:11], variance decides how many rays [16:32–17:01], and guiding decides where they go [19:48]. No interaction is discussed. Risk *(not in talk)*: guiding lowers variance, so rays drop and the surfel goes dormant while locked onto old directions. The cosine fraction guards against that.

**Updated pro/con:**
- Pro: a big win for off-normal light (sunlit wall, bright corner, dark hall) [18:54–19:26], [23:28–24:33]. Tiny storage, unbiased if done right, no frame reuse.
- Con:
  - It breaks the "ray patterns are fixed" rule.
  - It needs a warm-up [24:07], which conflicts with pre-fill unless the map starts as cosine.
  - No gain for uniform light or small sources.
  - The lock-in risk isn't covered by the talk.
  - Reference tests become statistical.
  - No time cost is given.
- **Recommendation: LATER, opt-in `--gi-ray-guide off|on` (default off).** Do it after MSME and variance rays land. Our deviations from the talk: a cosine fraction plus an ε floor, the map initialised to cosine (pre-fill), and a seeded RNG. It needs Anthony's call on the fixed-pattern rule, and a PDF testbed first.

### 1.14 Irradiance sharing between neighbours [25:08–26:23]
Surfels access others "in the same grid cell" via the acceleration structure and apply "weighted irradiance sharing" [25:25–25:43]. The weights are **not given**. Normal, distance and moment gating are my assumption. After 64 samples, it looks far less blotchy.
gi-v2: Anthony's "**splotchy** / blotchy light detail" complaint (10-10) is this exact symptom. Our equivalent is a spatial filter over surface-cache entries **in cache space** (not screen space, so no frame reuse). It probably overlaps with what PR #40's denoiser does in screen space.
**NOW as a queued roll-in to PR #40** (option: pre-filter in cache space, which is cheaper than screen denoise iterations). Flag: it introduces bias (light blurs across edges) unless gated by normal and moments.

### 1.15 Ray binning (sort rays by cell + direction) [26:37–27:31]
[slide 27:30] BF5 approach, **16-bit bin index: 12 bits spatial (cell location hash) + 4 bits orientation (ray direction)**. This corrects the earlier reading, which said ray index: the bin is by *direction*, and the reorder uses ray index plus bin offsets. Speech: bin key = the surfel's cell coordinate converted to 1D (the dominant part) + "the rays shot from the circle" as the other part [27:04–27:15]. That's a per-surfel ray index, not explicitly a direction. Binning is "based on their position and orientation" [26:59–27:01]. There are 2 passes for counts/offsets plus 1 reorder pass (Battlefield V approach). This improves BVH cache coherence.
gi-v2: our hardware ray queries for the world cache. With lavapipe in the cloud you can't measure it; it needs zanven-pc. Our rays come from a lattice, so they are already fairly coherent.
**LATER** (perf-gate candidate. Measure on the real GPU first. It may be a win on Sponza, which drops to 9–19 fps on high).

### 1.16 Many lights: stochastic lightcuts vs RIS/ReSTIR [27:34–33:23]
Brute-force random N lights doesn't scale. **Stochastic Lightcuts** (Yuksel, HPG 2019): Morton-sorted view-space lights, a bottom-up tree, a cut of 2–8 nodes minimising error, then a stochastic descent. It converges in about 5 s on 1,400 lights but is expensive on console. **Reservoir/RIS** (from ReSTIR): sample 4–8 candidate lights weighted by distance and intensity, keep 1, normalise the PDF, one shadow ray. It's cheap but noisier. Full spatiotemporal resampling is future work (reservoirs stored per cell/surfel [45:57]).
gi-v2: PR #43 `--lamp-pick hits|reuse`: `hits` is close to their RIS-without-reuse, and `reuse` (experimental, noisy without a denoiser) is the temporal-reservoir step they were also still struggling with.
**LATER**: a light tree (SLC) as a third `--lamp-pick tree` mode for the 100-lamp stress scene (moving 17 fps). Storing reservoirs **per cache entry** rather than per pixel is their proposed fix for the storage problem, and it's also our cleanest path to make `reuse` work without a pixel denoiser. Queue it.

### 1.17 Transparents: SH probes in a volume clipmap [33:23–38:30]
G-buffer spawn can't cover transparents, so ray-traced probes in nested clipmap levels (4 levels shown [37:21–37:41]; 3 in the perf test [44:07]) are projected to SH [33:59] (order not stated), use the same MSME integrator and variance ray count, and shift toward the camera when it leaves a level's centre grid. Still-valid probes are copied to their new coordinate rather than discarded [35:24–36:07]. That is not explicitly toroidal. New probes are initialised from the coarser level (that's their "pre-fill"). Levels update out of sync. Level borders get a blend band, or **blue-noise dither** between levels (one fetch, TAA cleans it). Cost about 0.2 ms full-screen sampling at 900p.
gi-v2: our world probes are world-fixed out to 50 m plus a far volume. **Initialise new probes from the coarser level** is a pre-fill technique we should check we already do. **Blue-noise dither** at the near/world/far seams could replace dual-fetch blends.
**LATER** for transparents (we don't have a transparent GI path yet). **NOW-cheap check**: coarse→fine pre-fill on probe activation. Flag: dither relies on TAA. If we have no TAA in the debug path, dithering will show.

### 1.18 Skinned meshes [5:09–5:45, 1:08]
Covered by 1.5 (the highest-weight bone ID). Skinned geometry casts and receives like static.
gi-v2: no skinning yet (animation is the next roadmap priority). **LATER**: design the transform-ID buffer in 1.5 so bone IDs slot in without changes.

### 1.19 Frame structure and perf [38:52–44:36]
**Slide [39:10] full pass list:**
- Persistent: positional update, recycle, grid allocation, ray sorting, raytrace, clipmap update, probe trace.
- Spawn: geometric normal reconstruction, gap fill, ray sorting, raytrace, write to persistent storage, write to probe volumes.
- **Filtering: spatial denoise + temporal denoise.**
- Apply: inject newly spawned, **lighting apply at quarter area resolution**, lighting upsample, clipmap sampling.
- **So GIBS also runs a screen denoiser.** That matters for PR #40: surfel caching alone did not remove the need for one.
**Slide [39:50] stress settings:** PS5, 4K output, infinite far plane, **100K ray budget** (the default; the 1M figure [41:26] is the boosted test), 8-sample reservoir, solving from zero.
Four phases: persistent surfel work (RT dominated), spawned work (RT or normal reconstruction or gap fill), filtering (about 0.2 ms), and apply (lighting apply dominates). PS5, 4K output, 1080p internal, no distance cull, 8-sample RIS:
- PvZ town, sun only: converges in about 3.5 s, about 7 ms
- 1,400 lights: about 11 ms with a 1M ray budget, about 10 s to converge
- Constrained view: about 7 s
- 4-sample SLC: about 5 s, 11.3 ms
- Internal demo, 140 lights: about 5.5 ms
- Free roam: 5.4 ms, or 3.4 ms at 1800p checkerboard
- 3-level clipmap solved round-robin: cheap
gi-v2 comparison: our target is 200+ fps moving (5 ms *whole frame*), so GIBS's 3.4–7 ms GI alone is above our budget. That supports keeping the lattice + screen-probe design and taking only the cheap estimator ideas. Round-robin solving of clipmap levels matches our budgeted updates.

### 1.20 Limitations and future work [44:36–46:31]
- Procedural/deforming geometry continuously respawns, because it isn't tracked.
- High-detail geometry over-spawns. The planned fix is to combine with SSGI where it suffices.
- Ideas they listed: share guiding across surfels, ReSTIR to augment guiding, store light visibility per cell to cut dead lights, share lights between binned rays (spatial RIS), reservoirs per cell/surfel, optimise the SLC traversal, move shadow rays to a later pass, and add ray guiding + specular to the probes.
gi-v2: our screen-space near field already handles the SSGI half. **Per-cell light visibility** is a strong LATER idea for the 100-lamp scenes (pairs with PR #43). The deferred shadow-ray pass is a perf-gate candidate.

## 2. Open questions (things we don't know)
1. ~~Exact MSME formulas~~ Answered (§1.8, saved code). Originally: (the short-window length, the variance→alpha map). Get Stachowiak's published code before implementing. The agent must cite it.
2. How depth moments interact with sharing (is sharing gated by Chebyshev?). Not stated.
3. The exact apply kernel weights ("among other things" [11:40]).
4. How the global ray budget redistributes (proportional scaling vs priority).
5. Recycle heuristic weights. Not given.
6. Whether the transform-ID buffer keeps previous transforms for motion. Probably yes (Frostbite), but not said.
7. Whether guided-ray direction maps are shared at all in 2021 (it's listed as future work).
8. ~~16-bit scale per surfel or per channel?~~ Answered: "a single 16-bit scaling value per circle" [21:09–21:11].
9. ~~Probability trees citation~~ Answered: McCool1997 [slide 20:20].
12. The guiding-map update rule, defensive mixing, and the hemisphere→quad mapping/Jacobian are not in the talk [20:39–23:25].
13. Trapezoid slice law and sizes are not in the talk [9:41–10:13].
10. Has GIBS shipped in a Frostbite title since 2021? Not found in this pass. Check later (lean).
11. Video frames not inspected (the heatmaps, grid views, and 4–5 m ID visualisation). Optional: look at them with yt-dlp clips if Anthony wants visual detail.

## 3. Verdict summary
- NOW: 1.5 transform-ID attachment for movers (PR #31 lag) + 1.8 MSME adaptive blend (dark-room fade, sun pop) + 1.12 variance-driven rays/budget/dormant (smart wakes, never-finishing probes). These share one data path, so they go in one agent.
- NOW-queued: 1.14 cache-space irradiance sharing (PR #40 splotch), 1.10 Chebyshev moments for leaks (PR #39/#37), 1.17 coarse→fine probe init check.
- LATER: 1.3 tile spawn throttle, 1.4 stochastic eviction, 1.13 6x6 guiding (breaks the fixed-pattern rule, so it needs Anthony), 1.15 ray binning (GPU-measure), 1.16 light tree + per-entry reservoirs (PR #43 follow-up), 1.17 transparents clipmap, 1.18 bone IDs, 1.20 per-cell light visibility.
- LATER, big-world track: 1.7 trapezoid grid as a camera-relative far-field index + `bigworld_valley` test scene.
- NO: 1.1 surfel primitive, 1.6 screen-relative lattice, GIBS cold-start convergence (conflicts with pre-fill).

## 4. Launch spec for the one new cloud agent (medium effort)
Repo AnthonyPoschen/genos-engine, base `gi-v2` (2abeb0dd). Branch `gibs-mover-attach-msme`.
> Implement two GIBS (EA SEED, SIGGRAPH 2021; research note GIBS.md §1.5, §1.8, §1.12) ideas in gi-v2, one commit per change, normal push only, then open a PR to gi-v2.
> (1) Attach surface-cache entries/probes that belong to moving objects to the object's transform ID: store object-local position + normal at allocation; each frame transform them with the object's current transform from a GPU transform buffer; static geometry stays world-lattice (ADR 0012). Flag this deviation from GIBS (they attach everything) in the PR. Target: PR #31 moving-box lighting lag.
> (2) Replace the fixed hysteresis + snap-on-big-change blend with a multi-scale mean estimator (long mean + short mean + short variance driving the long-term blend factor), following the published MSME at ./MultiscaleMeanEstimator.hlsl (Barré-Brisebois et al., Ray Tracing Gems ch. 25). Flag its 8σ firefly clamp as a bias, and make the clamp a setting. Keep "only finished values are published". Target: slow dark-room fade and the sun-4x fade-then-pop.
> (3) If clean, make per-entry ray count follow the MSME variance with a dormant floor and a global ray budget (smart wakes). Otherwise leave it for a follow-up and say so.
> Rules: follow the papers and flag every deviation in the PR; settings live in the public engine config, debug menu and MCP, with CLI args (`--gi-attach-movers on|off`, `--gi-blend msme|hysteresis`, `--gi-ray-budget N`, `--gi-dormant-rays F`); env vars only as a mirror; debug output to ./genos-debug/<kind>/ with --out override; add a variance heatmap debug view; never reuse or hold a whole frame; pre-fill required; no baking; lavapipe-only checks in the cloud; PR must include screenshots and numbers (moving-box lag before/after, dark-room fade time-to-90% before/after, still/moving fps on lavapipe vs gi-v2, picture diff vs cross-process noise floor). Keep the old path selectable for A/B.

## 5. Queued roll-ins (deliver when Chief says usage is back on pace)
**PR #31: bc-50806d5f-4ba6-5b15-9d2d-bf242470ac40**
> Heads-up before you resume: a separate agent is landing transform-ID attachment of mover cache entries plus the MSME adaptive blend on gi-v2 (GIBS §1.5/§1.8). Once that merges, rebase onto gi-v2 and re-measure the moving-box lag with `--gi-attach-movers on`. Drop any of your own lag workarounds the attachment makes redundant (list what you removed and why), and keep only the parts that still show a measured win. Screenshots + numbers vs gi-v2, deviations flagged, one commit per change, normal push.

**PR #37: bc-5925cb44-de65-5d19-8e97-0a14339da764**
> GIBS roll-in (§1.10, §1.17): for leak/contact problems (dark hall, bright corner, box-foot strip), check that probe/cache visibility uses depth *moments* (mean and mean²) with a Chebyshev soft test rather than a hard distance compare; GIBS gets away with a 4x4 hemisphere moment map per entry. Also check that newly activated probes are pre-filled from the coarser level (coarse→fine init) rather than starting at zero/sky. Apply only if it fits your PR's scope; otherwise list it as follow-up. Flag the DDGI resolution deviation. Lavapipe checks, screenshots + numbers.

**PR #39: bc-8164ccdf-b207-5fd3-8a65-c55275637623**
> GIBS roll-in (§1.3/§1.4/§1.12): if your PR touches slot allocation or wakes, consider (a) at most one allocation per 16x16 screen tile per frame with a randomised coverage threshold to smooth reveal spikes, (b) eviction by a stochastic score of pool fill × frames-since-last-applied × distance (fixed seed for tests), and (c) wakes driven by per-entry variance once the MSME agent lands. Don't duplicate (c) before that merges. Settings via config/debug/MCP/CLI, deviations flagged.

**PR #40: bc-db15359b-d5d3-51ed-8fb4-30da1035d009** (running, so do not interrupt; send after its current round)
> GIBS §1.14 idea for the splotch: GIBS fixes blotchy low-sample irradiance by sharing irradiance between neighbouring cache entries in *cache space* (normal + distance weights, gated by depth moments), not by screen denoising. If your pictures stay beyond the cross-process floor, try a cheap cache-space pre-filter as an opt-in (`--gi-cache-share on|off`) and compare it to adding denoise iterations: fps + p99 diff vs floor. Bias flag: blur across edges unless gated.

**PR #43 lamp picking (merged; new follow-up when usage allows, not a running agent)**
> GIBS §1.16: add an experimental `--lamp-pick tree` (stochastic lightcuts, Yuksel 2019), and test storing reservoirs per cache entry instead of per pixel to make `reuse` work without a pixel denoiser. 100-lamp stress scene numbers.

## 6. References shown on screen (from slide frames, 2026-10-14)
| Slide ts | On-screen cite | Reference | Link (checked) |
|---|---|---|---|
| 4:05 | [Stachowiak2018] | T. Stachowiak, "Stochastic All The Things: Raytracing in Hybrid Real-Time Rendering", Digital Dragons 2018 | https://www.ea.com/seed/news/seed-dd18-presentation-slides-raytracing (200) |
| 12:58 | [Donnelly2006] | W. Donnelly, A. Lauritzen, "Variance Shadow Maps", I3D 2006 | https://dl.acm.org/doi/10.1145/1111411.1111440 (403 to bot; DOI valid) |
| 12:58 | [Majercik2019] | Z. Majercik et al., "Dynamic Diffuse GI with Ray-Traced Irradiance Fields" (DDGI), JCGT 8(2) 2019 | https://jcgt.org/published/0008/02/01/ (200) |
| 12:58 | [Chebyshev1867] | Chebyshev's inequality (one-sided/Cantelli form used) | n/a |
| 15:40 | [BarréBrisebois2019] | C. Barré-Brisebois et al., "Hybrid Rendering for Real-Time Ray Tracing", Ray Tracing Gems ch. 25, 2019 | https://link.springer.com/chapter/10.1007/978-1-4842-4427-2_25 (200); code https://github.com/Apress/ray-tracing-gems/blob/master/Ch_25_Hybrid_Rendering_for_Real-Time_Ray_Tracing/MultiscaleMeanEstimator.hlsl (fetched) |
| 20:20 | [Müller2017] | T. Müller, M. Gross, J. Novák, "Practical Path Guiding for Efficient Light-Transport Simulation", EGSR 2017 | https://tom94.net/data/publications/mueller17practical/mueller17practical.pdf (200) |
| 20:20 | [McCool1997] | M. McCool, P. Harwood, "Probability Trees", Graphics Interface 1997 | https://graphicsinterface.org/proceedings/gi1997/gi1997-5/ (403 to bot; URL pattern unverified) |
| 20:50 | [Cigolle2014] | Z. Cigolle et al., "A Survey of Efficient Representations for Independent Unit Vectors", JCGT 3(2) 2014 | https://jcgt.org/published/0003/02/01/ (200) |
| 24:00 | HDRI credit | Sergej Majboroda, "Abandoned Workshop", Poly Haven (testbed env map) | https://polyhaven.com/a/abandoned_workshop (200) |
| 27:30 | "BF5 approach" | J. Deligiannis, J. Schmid, "It Just Works: Ray-Traced Reflections in Battlefield V", GDC 2019 | https://www.gdcvault.com/play/1026282/It-Just-Works-Ray-Traced (from search) |
| 29:05 | [Yuksel2019] | C. Yuksel, "Stochastic Lightcuts", HPG 2019 | https://www.cemyuksel.com/research/stochasticlightcuts/ (search) |
| 29:05 | [Lin2020] | D. Lin, C. Yuksel, "Real-Time Stochastic Lightcuts", I3D 2020 (new cite, not mentioned in speech) | http://dqlin.xyz/pubs/2020-i3d-SLC/ (search) |
| 31:40 | [Bitterli2020] | B. Bitterli et al., "Spatiotemporal Reservoir Resampling (ReSTIR)", SIGGRAPH 2020 | https://research.nvidia.com/publication/2020-07_spatiotemporal-reservoir-resampling-real-time-ray-tracing-dynamic-direct (200) |
| 35:00 | [Stachowiak2018] | Adaptive integrator reused for probes | as above |

**Key frames** (open the video at these times): spawn tiles [4:05](https://www.youtube.com/watch?v=h1ocYFrtsM4&t=245s), transform IDs [4:45](https://www.youtube.com/watch?v=h1ocYFrtsM4&t=285s), free stack [7:00](https://www.youtube.com/watch?v=h1ocYFrtsM4&t=420s), **trapezoid grid [10:00](https://www.youtube.com/watch?v=h1ocYFrtsM4&t=600s)** (a central cube with a frustum slab on each face, which confirms the 6-pyramid reconstruction's topology; no slice law is shown), **depth moments [12:58](https://www.youtube.com/watch?v=h1ocYFrtsM4&t=778s)**, **MSME [15:40](https://www.youtube.com/watch?v=h1ocYFrtsM4&t=940s)**, guiding refs/mapping [20:20](https://www.youtube.com/watch?v=h1ocYFrtsM4&t=1220s), **6x6 luminance map [21:10](https://www.youtube.com/watch?v=h1ocYFrtsM4&t=1270s)**, hierarchy [23:00](https://www.youtube.com/watch?v=h1ocYFrtsM4&t=1380s), error plot [24:00](https://www.youtube.com/watch?v=h1ocYFrtsM4&t=1440s), ray sorting [27:30](https://www.youtube.com/watch?v=h1ocYFrtsM4&t=1650s), lightcuts [29:05](https://www.youtube.com/watch?v=h1ocYFrtsM4&t=1745s), reservoir algorithm [31:40](https://www.youtube.com/watch?v=h1ocYFrtsM4&t=1900s), probe clipmap [35:00](https://www.youtube.com/watch?v=h1ocYFrtsM4&t=2100s), **frame overview [39:10](https://www.youtube.com/watch?v=h1ocYFrtsM4&t=2350s)**, stress settings [39:50](https://www.youtube.com/watch?v=h1ocYFrtsM4&t=2390s), future work [45:40](https://www.youtube.com/watch?v=h1ocYFrtsM4&t=2740s).

**Corrections from the slides:**
- "mikula dal" = McCool1997.
- The guide map is single-channel luminance, a moving average, normalised every frame, held in one 4K atlas.
- The MSME formula is known (and includes an 8σ firefly clamp).
- Ray bins are 12 spatial + 4 *direction* bits.
- GIBS has spatial + temporal denoise passes and quarter-res apply.
- The default ray budget is 100K.
- Lin2020 is cited.
- The trapezoid diagram matches the cube + 6-frustum topology.

## Test scene principles from the many-light town (Anthony, 14 Oct 2026)
Reference frame: the many-light town at [29:00](https://www.youtube.com/watch?v=h1ocYFrtsM4&t=1740s) (the stress test with about 1,400 lights, just before 29:21). We don't copy the scene; we take its principles for the `bigworld_valley` / many-light test:
- Hundreds to thousands of small lights spread through a complex landscape at mixed scales (street lamps, windows, signs, string lights), not a grid.
- Dense occlusion and varied geometry (buildings, trees, arches, overhangs), so most lights are irrelevant to any given pixel.
- Long view distances showing near and far detail together, to exercise the light tree, the far-field grid and distance error bands.
- Moving content throughout: moving lights, moving occluders and objects, and a moving camera fly path.
- Use it to test the light tree (`--lamp-pick tree`), the trapezoid grid, attached movers and the adaptive blend together.
- Include an elevated, partly top-down camera view, like the reference frame, that shows a large share of the landscape (but not all of it) at once. Many lights and surfaces are visible at very different distances, with edges leaving the view, to stress culling, far-field accuracy and how lights enter and leave the frame.
