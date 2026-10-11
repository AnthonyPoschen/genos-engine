# Lighting

How GI v2, the lighting engine, works: direct light, shadows, global illumination, the reference it is checked against, and the checks. Each component lists our method, the published method it compares to (Lumen and papers, with links), how and why ours differs, current scores and open issues. The plan and budgets are in [GI v2 design](gi-v2-design.md); the gate is in [Debugging](debugging.md). Update this file whenever a method changes, in the same commit as the code.

Scores are gi-check numbers (linear luminance, share of the light) against the GPU path-traced references, at the commit named in each section. "P6" is the gi-v2 default (6ce7fd4: screen probes, light cache in push mode, `GENOS_GI2_WORLD=0`). "SC" is the surface cache (`GENOS_GI2_WORLD=3`, branch `surface-cache`, flags in `GENOS_GI2_SC_FLAGS`).

## Sources

- [W21] D. Wright, "Radiance Caching for Real-time Global Illumination", SIGGRAPH 2021 Advances. https://advances.realtimerendering.com/s2021/index.html
- [W22] Wright, Narkowicz, Kelly, "Lumen: Real-time Global Illumination in Unreal Engine 5", SIGGRAPH 2022 Advances. https://advances.realtimerendering.com/s2022/SIGGRAPH2022-Advances-Lumen-Wright%20et%20al.pdf
- [KN] K. Narkowicz, "Journey to Lumen", 2022. https://knarkowicz.wordpress.com/2022/08/18/journey-to-lumen/
- [DOC] Lumen Technical Details (UE5 docs). https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-technical-details-in-unreal-engine
- [SRC] UE 5.4 cvar defaults (mirror). https://indxzero.github.io/ue544cvarwiki/
- [ReSTIR GI] Ouyang et al., "ReSTIR GI: Path Resampling for Real-Time Path Tracing", HPG 2021. https://research.nvidia.com/publication/2021-06_restir-gi-path-resampling-real-time-path-tracing
- [DDGI] Majercik et al., "Dynamic Diffuse Global Illumination with Ray-Traced Irradiance Fields", JCGT 8(2), 2019. https://jcgt.org/published/0008/02/01/
- [SHaRC] NVIDIA, "Spatially Hashed Radiance Cache" (RTXGI 2.x). https://github.com/NVIDIA-RTX/SHARC
- [NRC] Müller et al., "Real-time Neural Radiance Caching for Path Tracing", SIGGRAPH 2021. https://research.nvidia.com/publication/2021-06_real-time-neural-radiance-caching-path-tracing
- [SVGF] Schied et al., "Spatiotemporal Variance-Guided Filtering", HPG 2017. https://research.nvidia.com/publication/2017-07_spatiotemporal-variance-guided-filtering-real-time-reconstruction-path-traced
- [ReBLUR] D. Zhdan, "ReBLUR: A Hierarchical Recurrent Denoiser", Ray Tracing Gems II ch. 49, 2021. https://link.springer.com/chapter/10.1007/978-1-4842-7185-8_49
- [GIBS21] H. Halén et al., "Global Illumination Based on Surfels", SIGGRAPH 2021 Advances. https://advances.realtimerendering.com/s2021/index.html
- [GIBS24] Apers, "Shipping Dynamic GI in Frostbite", SIGGRAPH 2024 Advances. https://advances.realtimerendering.com/s2024/content/EA-GIBS2/Apers_Advances-s2024_Shipping-Dynamic-GI.pdf
- [MSME] T. Stachowiak, multi-scale mean estimator (kajiya renderer, the estimator GIBS cites). https://github.com/EmbarkStudios/kajiya
- [A-SVGF] Schied et al., "Gradient Estimation for Real-Time Adaptive Temporal Filtering", HPG 2018. https://cg.ivd.kit.edu/atf.php
- [NRD] NVIDIA Real-time Denoisers (ReBLUR, ReLAX). https://github.com/NVIDIA-RTX/NRD
- [IC] Křivánek et al., "Radiance Caching for Efficient Global Illumination Computation", IEEE TVCG 2005.
- [LT] Conty Estevez, Kulla, "Importance Sampling of Many Lights with Adaptive Tree Splitting", HPG 2018. https://doi.org/10.1145/3233305

Research notes with the cvar values: `/workspace/research/lumen-digest.md` (agent box).

## Frame overview

One frame, passes in order (`crates/render/src/gi2_gpu.rs`, shaders `gi2_*.comp`); every pass has at most one trace site:

1. `gi2_place`: one screen probe per 16 px tile on the G-buffer.
2. `gi2_trace`: this frame's stratum of each probe's rays, re-traces of kept rays that cross a change, fills for newly seen surfaces, and the light cache's rays (async queue).
3. `gi2_light`: direct light with traced shadows at every pixel and every hit.
4. `gi2_gather`: each probe bins its kept rays by direction.
5. `gi2_filter`: three probe-space filter rounds, then SH.
6. `gi2_compact` / `gi2_cache`: the light cache (or surface cache) relights a batch.
7. `gi2_compose`: pixel = emitted + direct + interpolated probe (or cache) irradiance.

## Scene representation and tracer

- **Ours:** one software tracer over mesh SDF instances (`mesh_field.glsl`): sparse 8³ bricks of byte distances near the surface, one coarse byte per brick elsewhere, sphere-traced in object space after clipping to the instance bounds. All GI v2 rays (probe, cache, shadow) use it.
- **Reference:** Lumen software RT uses per-mesh SDFs near and a global SDF far; hardware RT uses a BVH [W22, KN].
- **Differences:** no far global SDF yet; the design wants world-anchored far bricks instead of camera clipmaps (clipmaps change with the camera). No hardware path yet.
- **Open:** far field (long rays) and hardware RT are not built.

## Direct light and shadows

- **Ours:** `gi2_light.comp` lights every lamp at every pixel and at every ray hit with one traced shadow ray each, so pixels, kept probe hits and cache hits share one loop and see this frame's light. No shadow maps, no light history.
- **Reference:** Lumen uses virtual shadow maps for pixels and shadow-map lookups plus traced rays for surface-cache texels, up to 8 lights per 8×8 card tile [W22]; MegaLights samples many lights stochastically and denoises. Many-light trees [LT].
- **Differences:** traced shadows match the reference tracer exactly and need no denoiser; cost grows with lamp count (fine for the stress scene, five lamps). The design's deterministic light-tree cut per tile and cached shadow masks are not built.
- **Scores (P6):** direct view matches a `bounces: 0` trace to about 2 %.
- **Open:** light tree for many lamps; area lights for small bright emitters.

## Screen probes (final gather)

- **Ours:** one probe per 16 px tile (smaller below 720p, about 45 rows), placed on the G-buffer at the tile centre or the first of four inner points that has a surface. 64 rays per frame per probe, in 8 strata (frame f traces stratum f mod 8), so a probe sees 512 directions. Pixels take the 4 nearest probes weighted by plane distance and normal (`gi2_compose`).
- **Reference:** Lumen places probes on G-buffer pixels on a 16 px grid, refines adaptively down to 4 px where interpolation fails, jitters placement and directions every frame, 8×8 octahedral rays, BRDF-guided product importance sampling (about 4×), and a 10-frame temporal filter [W21 s.33-52]. Irradiance caching [IC] is the origin of the adaptive refinement.
- **Differences:** fixed strata instead of per-frame jitter (no crawling noise; error is a still pattern). No importance sampling and no adaptive refinement yet.
- **Scores (P6):** settled mean 0.033-0.042 per view (limit 0.043 with GPU refs); room 48 px swim 0.0025 (target 0.0006, MISS); light structure 0.0143 (target 0.010, MISS); close-up 0.0106 PASS.
- **Open:** bounce swims while the camera turns (Anthony, 11:27) because the probe grid slides with the screen; adaptive probes; BRDF importance sampling.

## Kept hits (temporal reuse of visibility, not light)

- **Ours:** a probe keeps the hits of its last 8 strata. Each frame every kept hit is shaded again (`gi2_light`), so no light is carried over: a lamp that moves or switches shows in full next frame. A probe that slid on its surface takes strata from last frame's neighbours within 2.5 spacings on the same plane (parallax-corrected). Kept rays crossing the bounds of a moved object are re-traced; strata nobody traced (newly seen surfaces) are filled from the probe this frame.
- **Reference:** Lumen accumulates *radiance* over 10 frames with per-pixel depth rejection and a fast-update mode for pixels whose rays hit movers [W21 s.98-100]. ReSTIR GI [ReSTIR GI] reuses path samples (visibility and radiance) with reservoir resampling and validation rays. SVGF / ReBLUR [SVGF, ReBLUR] accumulate radiance with variance-guided rejection.
- **Differences:** we reuse visibility only, so light changes have no lag and no ghosting; the cost is fewer effective samples than a 10-frame radiance history while moving. Closest to ReSTIR GI's sample reuse with re-shading, without resampling weights.
- **Open:** importance resampling of kept hits (ReSTIR-style) could raise effective samples.

## Probe filter

- **Ours:** `gi2_filter.comp`, Lumen's recipe: 3 à-trous rounds (neighbours 2, 4, 8 probes apart) over 3×3, same direction, depth-only weight, neighbour ray used only if its hit seen from this probe is within 10° after clamping its distance to our own. Last round projects to SH (bands 0-2, cosine-convolved).
- **Reference:** Lumen: 3 passes of 3×3 neighbours, depth weight, 10° hit angle, hit-distance clamp [W21 s.57-63].
- **Differences:** à-trous spacing (2/4/8) instead of adjacent neighbours, to spread residual noise into blotches wider than the light varies (helps grid redraw and blotch).
- **Scores (P6):** blotch PASS (limit 0.023).

## Light cache (ray-hit irradiance, push mode)

- **Ours (P6 default, `GENOS_GI2_WORLD=0`):** a hashed cache of world patches (1M slots, 16-slot search) keyed by position, axis normal and size (25 cm × 2^level beyond 8 m from the eye). Ray hits read it to get the bounce (multi-bounce by feedback). Each round relights a batch of 16384 patches with 64 rays each, young patches first; each relight replaces the value (no running mean). Patches unread for 1800 frames are freed.
- **Reference:** SHaRC [SHaRC] (hashed world cells, accumulate then resolve); NRC [NRC]; Lumen's surface cache radiosity at hits [W22]; DDGI probes [DDGI].
- **Differences:** replace-on-relight keeps it noisy per patch but unbiased and lag-free; ray hits average over many patches so the noise mostly cancels.
- **Scores:** lights-only return (lampret, limit 0.002): hall 0.0002, room 0.0001 PASS.

## Surface cache (experimental, `GENOS_GI2_WORLD=3`)

- **Ours:** the light cache with fine texels that pixels read directly (bilinear over the 4 nearest texels on the pixel's axis plane): 6.25 cm within 2.5 m, 12.5 cm to 8 m, then 25 cm × 2^level; ray hits read 25 cm patches. Each texel accumulates 64-ray sets as a running mean up to a cap (64). Texels start from a pooled Gaussian of their lit neighbours and from the lit texel twice the size. Relight budget: one batch of 16384 per round. Best flags 239882: slanted surfaces (|n| not axis-aligned) use the probes; texels read keep relighting at the cap with rotating ray sets (multi-bounce keeps arriving); hit patches replaced each relight (push-style, removes the hit-cache bias).
- **Reference:** Lumen's surface cache: mesh cards fixed by geometry, resident pages sized by distance (5-20 cm), lighting updated within a fixed per-frame budget by priority = LastUsed − LastUpdated (direct 1/32, radiosity 1/64 of the atlas per frame), radiosity accumulated over at most 4 frames, lighting resampled when a page is reallocated, unused pages kept 256 frames [W22, SRC]. In Lumen pixels never read the cache directly; it is only read at ray hits, behind the screen-probe temporal filter.
- **Differences and why:**
  - Pixels read texels directly: this is what removes the screen-probe swim (texels are world-anchored), so texel noise and drift are on screen; Lumen hides cache noise behind the final gather.
  - Running mean of up to 64 sets instead of Lumen's 4-frame accumulation, because the texel is shown directly.
  - Camera only orders updates (flag 524288, S31, testing): once a texel has a set its lit-from point is frozen (no restart when a nearer reader appears), and a texel claimed at a new size (the eye crossed a size ring) starts from the light and count (up to 16 sets) of the lit texel at the next size up or down, as Lumen resamples lighting on realloc.
  - Per-texel estimator (flag 1048576, S32, testing), after GIBS [GIBS21] and Stachowiak's multi-scale mean estimator [MSME]: besides the long-term mean each texel keeps a short-term mean (blend 0.08) and short-term luminance variance. The long-term blend is a plain mean for the first 16 sets, then grows with how many standard deviations the short-term mean sits from it (with an inconsistency term and a variance-based reduction) and falls to 1/256 once they agree, so a converged texel neither drifts nor freezes and still follows real change. Fireflies above the short-term mean + 8σ are clipped. Converged texels whose sets agree (inconsistency < 0.3) go dormant: the sweep relights them one round in 8 to notice change, freeing budget for the rest (GIBS: dormant surfels fire only change-detection rays). Our ray count per relight stays 64; GIBS varies rays per surfel instead.
  - Texel size still depends on eye distance, as Lumen's resident pages do; the design wants size fixed per mesh. Resampling covers the ring crossings for now.
- **Scores (S41, flags 1812746 = 239882 + 524288 + 1048576, commit after a2fbf3e8):** settled room-a 0.0405 (bias -0.001), corner 0.0421 (+0.009), contact 0.0372 (+0.007), lamp-wall 0.0344 (+0.007), hall 0.0371 (+0.012), close 0.0204 (+0.006). Room: world swim 0.0019 PASS (settled-per-pose 0.0012 = the reference's own), grid redraw 0.0008 PASS, 48 px swim 0.0010 (target 0.0006), blotch 0.0151 PASS, close-up 0.0093 PASS, structure 0.0102 (target 0.010), end 0.0026 FAIL. Still shimmer 0 and 600-frame still drift 0. Turn: arrival 0.0027 / walking 0.0050 PASS, onset 0.0051 PASS, held 0.0050, return 0.0067, end 0.0073 FAIL. Lights-only return: hall 0.0032, room 0.0023 (limit 0.002; push 0.0002). Spin 0.1096 / flicker 0.0519 FAIL, mix 0.0116 / 0.0117 FAIL.
- **Earlier scores (S27, flags 239882):** settled room-a 0.0385 (bias +0.009), corner 0.0424 (+0.013), contact 0.0374, lamp-wall 0.0328 (-0.003), hall 0.0371 (+0.013), close 0.0212. Room: blotch 0.0110 PASS, structure 0.0079 PASS, close-up 0.0082 PASS, world swim 0.0042 FAIL (settled-per-pose 0.0056; was 0.0006 before relighting at the cap), grid redraw 0.0050 FAIL, end 0.0036 FAIL. Still shimmer (60 still frames) 0.0000. Lights-only return: hall 0.0020, room 0.0022 (limit 0.002). Spin 0.1075 / flicker 0.0529 FAIL; mix 0.0116 FAIL.
- **Camera trace (S30, 239882 vs S31 with 524288), per readback while turning:** claimed 542 / 602, offer restarts 234 / 0.6; still: 0 claimed, 0 restarted, relit 16k (budget always full).
- **Open:** slow drift of texels at the cap (continual relighting moves the mean; swim settled-per-pose 0.0056); holding converged texels (flag 262144) fixed the drift (0.0006) but raised blotch to 0.0207 and close-up to 0.0165. Next: Lumen-style priority (relight what was read and is oldest, LastUsed − LastUpdated) with converged texels relit only when their inputs change. Moving and spinning boxes (Lumen recaptures cards of movers and relies on screen traces for them). Bounce darkness was fixed by push-style hit patches; Lumen's equivalent is radiosity reading last frame's indirect at hits.

- **Read rule at ray hits (S40):** after SHaRC [SHaRC]: a hit closer to its ray's origin than size × √3 reads the finest lit texel that satisfies t > size × √3 and holds at least 4 sets; otherwise the 25 cm patch, which falls back to its lit neighbours, never black. SHaRC instead continues the path past a too-close cell; we cannot afford another bounce there, so we pick a finer cell.

## Radiosity and multi-bounce

- **Ours:** ray hits read the light cache's irradiance at the hit (last round's value), so each relight adds one bounce: infinite bounce by feedback. In SC, hit patches are 25 cm and replaced on each relight.
- **Reference:** Lumen radiosity: one 4×4 hemisphere probe (16 rays) per 4×4 card texels, jittered over 4 frames, plane and hit-distance weighted interpolation, accumulated over 4 frames; hits read this frame's direct plus last frame's indirect [W22]. DDGI gets multi-bounce the same way through probe feedback [DDGI].
- **Bounce split (S35, `GENOS_GI_CHECK_BOUNCES=1` references against a one-bounce engine, flag 2097152):** share of the picture from bounce 2 and later, reference / push engine: room-a 0.584 / 0.579, corner 0.873 / 0.857, contact 0.832 / 0.821, lamp-wall 0.661 / 0.661, hall 0.342 / 0.336, close 0.427 / 0.422. The first bounce is slightly bright in the dark corner and contact views (corner +15 % in push, +6 % in the surface cache; it is the gather, P6 has it too) and later bounces 1-1.6 points short there. The earlier 3-4 % darkness is gone.
- **Differences:** per-patch 64-ray hemispheres instead of shared 16-ray card probes; no spatial reuse between texels except pooling.
- **Open:** card-space probes for the bounce (Lumen's radiosity) would share rays between texels.

## Dynamic objects

- **Ours:** each frame lists the bounds of objects that moved, came or went (up to 64; more drops every kept stratum). Kept rays crossing them are re-traced; light-cache patches near a change (1 m) are relit first and run short means (2 sets) in SC.
- **Reference:** Lumen: movers are in the SDF/BVH, their cards are recaptured, pixels whose rays hit movers switch to fast temporal update with stronger spatial filtering [W21 s.99, W22]. ReSTIR GI validates reused samples with visibility rays.
- **Differences:** we invalidate by ray path (exact) rather than by hit velocity.
- **SC movers (S42-S49):** occluders that moved in the last 240 frames are movers; pixels inside a mover's bounds and within 1 m of this frame's changes show the probes, texels near a change are relit first with 2-set means. While only the light changes (lamps move), texels stay shown and lag a little, as Lumen's surface cache does (S46/S47; swapping every pixel to the probes swapped methods and was most of the mix error).
- **Scores:** P6 passes trail, mix, spin. SC (S49): mix 0.0084 / 0.0087 (target 0.0055), spin 0.0871 / flicker 0.0470 (the spinning green box is not caught as a mover; Anthony judges spin by eye).
- **Open:** object-local texels (GIBS: transform id + local position) are not built; texels on a moving box are world-keyed and re-claimed as it moves.

## Contact term

- **Ours:** none at full resolution; contact comes from probe rays (16 px) and the filter's hit-distance clamp.
- **Reference:** Lumen's full-res short-range AO / bent normal (screen traces of about 16 px), applied after the temporal filter [W21, W22].
- **Open:** contact rows pass but corner contact is the weakest settled view; a full-res contact term is planned (design rec. 3).

## Temporal and history

- **Ours:** no radiance history in the screen path (visibility only, see kept hits). The SC holds per-texel running means (that is a history, world-anchored, reset per texel on change or under changing light).
- **Reference:** Lumen's 10-frame full-res temporal filter, per-pixel rejection, no global reset [W21]; SVGF / ReBLUR.
- **Open:** SC lights-only return is at the limit (0.0020-0.0022): stale light lingers in texel means after lamps move and come back.

## Reference path tracer

- **Ours:** `crates/debug/src/reference.rs` (CPU) and the GPU reference (`GENOS_REFERENCE_GPU=1`): primary rays per pixel centre, shadowed direct light at each vertex, cosine bounce, Russian roulette after 3 bounces, traced to 0.05 noise or 2048 spp, cached by scene, pose and size. Furnace, closed room and inverse-square checks hold it honest.
- **Reference:** standard unidirectional path tracing (PBRT).
- **Differences:** the GPU refs read 0.002-0.004 higher error for the same engine than the old CPU refs (old corner leak), so the settled mean line is 0.043 with GPU refs.

## gi-check grades

- **Ours:** `genos-stress gi-check` (see [Debugging](debugging.md)): settled views against references (mean, bias, blob, contact, open wall), moving views, repro, camera, trail, flick (moving flicker and still shimmer), turn (arrival, onset, held, return), room (world swim, 48 px swim, grid redraw, blotch, structure, close-up, end), mix, spin, lampret (lights move and return; end against the soaked baseline, limit 0.002). Window titles name the run, GI mode, flags and test, plus REF while a reference renders. `GENOS_GI2_SC_TRACE=1` prints the surface cache's claims, restarts, frees and relights per readback.
- **Reference:** Lumen is checked visually against the path tracer; we gate numerically.
- **Open:** the still shimmer row (frame to frame, 60 frames) reads 0 for every build; the drift Anthony sees is slower than 60 frames at 8-bit, so a longer-window row (first vs last of ~600 still frames) is next.

## Fault ledger (short; the full ledger with first-seen points is in the work log SUMMARY.md)

Every failing grade of the surface cache, where it first appeared and which planned item should fix it. Re-checked before any push.

| Grade | Now / limit (S49, quick) | First seen | Expected fix |
|---|---|---|---|
| room end, soaked vs baseline | 0.0025 / 0.0020 | S26all, flags 239882, b1d7e956, 2026-10-11 | item 2 scheduler |
| turn held / return / end | 0.0052 / 0.0067 / 0.0073, limit 0.0020 | S26all 239882 (0.0045 / 0.0037 / 0.0043); longer settle (900) does not help | item 2 scheduler |
| lights-only return (lampret) | see SUMMARY (S47 0.0034 / 0.0025) / 0.0020 | S29lr239882, cf916c36 (0.0020 / 0.0022) | item 4 change detector |
| spin error / flicker / end | 0.0871 / 0.0470 / 0.0058, limits 0.027 / 0.020 / 0.002 | S26all 239882 (0.1075 / 0.0529) | item 4 object-local texels; judged by eye |
| mix still / turning / end | 0.0084 / 0.0087 / 0.0036, limit 0.0055 / 0.002 | S26all 239882 (0.0116) | item 4 |
| room light structure (target) | 0.0106 / 0.0100 | S34b estimator (239882 was 0.0079) | item 2 |
| room 48 px swim (target) | 0.0010 / 0.0006 | P6 itself 0.0025 | item 5 |

Fixed: camera regeneration (settled-per-pose 0.0056 -> 0.0012) and grid redraw (0.0050 -> 0.0008) by the estimator; offer restarts on camera moves (234 -> 0 per readback) by 524288.
