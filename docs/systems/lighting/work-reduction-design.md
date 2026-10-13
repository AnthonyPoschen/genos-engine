# Work reduction

The decisions below are the owner's answers and the judgments that close the open questions. Each phase lands in its own change and edits the lighting record in that same change. Until a phase lands, the current lighting record stays the description of the code.

The baseline is the owner's 1080p windowed moving run, about 14.5 ms a frame. Lighting is about 93% of it (about 13.5 ms). 200 fps (a 5.0 ms frame) is aspirational, not a gate. A measured cut of several times, with the picture inside the noise floor, is the result that counts. The planning sum below lands near 4.3 ms of lighting. With the other 1.0 ms left as it is, that is about 5.3 ms, about 190 fps, about 3.1× on the lighting share. The phase measurements on his RTX 4070 decide the rest. A cut that the picture can see fails, even if the frame is faster. Indirect light in a dark corner may lag a few frames after a lamp moves. Direct light and shadows stay on the same frame.

Every millisecond below that is not a measurement. It is the given pass split, with a-trous set to 3.0 ms so the passes add up to the stated lighting share. The a-trous pass on the moving scene is reported as 1.7–4.5 ms. Phases report the pass they touch, including a frame where a-trous is at the high end.

## The frame this design cuts

Planning split of the moving stress scene:

| Pass | ms | What the moving scene makes it do |
|---|---:|---|
| Light | 4.7 | Every lamp at every shaded point and every kept hit, one shadow ray each. A lamp move is a reason to relight kept hits (S78D). Pixel reuse is off once anything moves (S78J). |
| A-trous | 3.0 (range 1.7–4.5) | Layer 2 filters texels whose values moved. A light that writes the field trips the dense sweep (S78X2, dirty fraction above a quarter). |
| Compose | 2.1 | The camera or the lamps changed, so `still == 0` and compose covers the picture. Each pixel looks up four probes and, on the surface-cache path, finds, offers, and may parent-search four texels. |
| Trace | 1.9 | Probe strata and cache rays. The still scene's one check ray per held probe is about 0.3 ms of trace (S78K). The rest is the moving scene re-firing work. |
| Gather | 0.95 | Every probe bins its kept hits. |
| Filter | 0.87 | Three probe-space rounds, then the SH. A probe whose inputs moved runs them (S78Y). |
| Lighting | 13.5 | The rows above sum to 13.52. That is the stated 93% share. |
| Rest of the frame | 1.0 | Raster and present. Not a lighting pass. |
| Frame | 14.5 | About 69 fps. |

200 fps is 5.0 ms and is not the gate. The techniques below plan to about 4.3 ms of lighting.

## Rules this design keeps

- A cheap check ray that confirms the stored value keeps that area quiet (S78K, S78U). One check ray per held probe per frame stays. A pass does not relight, gather, or filter on a confirm.
- Update rate and ray count scale with the size of the change. The grade is A-SVGF λ, already used for texel rays (S74A, S76). A binary dirty bit is not the schedule.
- A real change wakes neighbours by impact on the texels a probe feeds, once per event, on a shrinking ring, under a per-frame cap. That is the approved wake ([WAKE](experiments/WAKE-neighbour-wake-design.md)). Probes are not woken by the shared averaged field.
- Inside a region, every texel refines together. A mixed-quality patch is worse than a uniform low grade (S72J, and the GI quality rule in the agent rubric).
- A static scene's GI cost approaches the check. Settled machinery adds no dispatch on top of the check the still scene already pays.
- The papers are the source. Every departure is listed in the technique and again in [Departures](#departures).
- Every frame is drawn from the current camera. The picture is composed from this frame's shaded points and the published lighting values. A moving camera already takes the full compose. This design does not drop a present and does not fill the picture by reprojecting last frame's color image. A settled lighting value may be stale. The frame that shows it is new. The held-camera 8×8 compose list ([QUIET](experiments/QUIET-quiet-paths-compose-filter-compact.md)) stays the held-camera path.

Stored light does not depend on the camera. The camera orders a queue when a cap is hit. It does not choose which value is published. [ADR 0012](../../adr/0012-gi-v2.md) already says this. The sparse set below is placed in the world from the light and the gradient.

[ADR 0008](../../adr/0008-lighting-uses-radiance-cascades.md) is superseded. The penumbra hypothesis is used as a spacing argument. The cascade tracer stays removed.

## Two regimes

**Full effort** is today's path. It runs for a shaded point or a probe that has no converged value, for a disocclusion (the probe moved, the held-hit check failed, a ray crosses a mover bound, a new face), and for a big change (λ at the top of the grade, or this frame's light motion can flip visibility). A failed check ray is a big change.

**Settled** is a place that already has a converged value and whose check ray agrees. A moving light updates a sparse subset of the probes it reaches. The others in the same region take that light's change by the blend compose already uses. They are not woken. Their stored stochastic residual (bounce past the first) is allowed to go stale, and the check is what brings it back.

A moving light is not a wake of every probe in range. The wake runs only after a check fails.

A region, for one moving light, is the settled texels and screen probes that share one λ bucket and that the compose blend connects, stopping at a visibility break (a face the blend already drops, or the shadow fringe). An 8×8 texel page is the smallest piece that can be scheduled. Pages the blend connects, in the same bucket, show the same grade in the same frame. The fringe is its own region and stays full effort.

Handover from a propagated value to a traced mean happens for the whole region in one frame. It happens when the traced mean sits inside the noise bound of the propagated value (the 3 SE test from S78U), or when the region has been full effort long enough that the traced mean is the converged one and every page adopts it together. A page does not step on its own.

## Where each millisecond goes

| Pass | Planning ms | Cut by | Planning remainder |
|---|---:|---|---:|
| Light | 4.7 | [Confirmed-visibility shade](#1-confirmed-visibility-shade) | 1.5 |
| Trace | 1.9 | [Sparse probes](#2-sparse-probes-neighbours-take-the-change) then [first bounce](#5-first-bounce-delta-higher-bounces-stay-graded) | 0.5 |
| Gather | 0.95 | Sparse probes | 0.25 |
| Filter | 0.87 | Sparse probes | 0.25 |
| A-trous | 3.0 | [Stochastic-only a-trous](#3-a-trous-runs-where-new-samples-landed) | 0.6 |
| Compose | 2.1 | [Published read](#4-a-settled-pixel-reads-the-published-field) | 1.2 |
| Lighting | 13.5 | | 4.3 |

The [grade](#6-one-grade-for-the-region) does not have its own remainder. On a large move λ saturates and the frame honestly spends the full-effort path. The 4.3 ms sum is the moving stress scene with a fringe and a sparse set, not a scene that has gone quiet.

4.3 ms of lighting plus the 1.0 ms residual is a 5.3 ms frame, about 190 fps, a 3.1× cut of lighting. That measured cut is the result that counts. Clearing 5.0 ms would need the measured light pass and the measured compose on the low side of these figures, and it is not required.

## Ranked techniques

### 1. Confirmed-visibility shade

**Saved:** about 3.2 ms, all of it in the light pass (4.7 → 1.5).

**What changes.** A shaded point whose visibility to a lamp is confirmed recomputes that lamp's diffuse term from the lamp's current position, intensity, and the cached visibility bit:

```text
E = V * I * max(n · ω, 0) / r²
```

`V` is the bit. `ω` is the direction from the point to the lamp. The shadow ray does not run. The bit is the one S74A already stores per 12.5 cm cell, facing axis, and lamp, for hits. Pixels get the same cell. A lamp move does not bump the lamp epoch. The epoch moves when a mover bound crosses the segment, when this frame's lamp motion can change the depth order along the segment (the fringe), or when the check ray disagrees.

The fringe, the mover sweep, a cold point, and a failed check stay in the light dispatch and trace a real shadow ray. Confirmed points are absent from that dispatch. S77B reused about two thirds of the shadow rays and the light pass stayed at 8.6 ms, because the full grid still launched, and the moving shadow edge tore. The early-out inside a full dispatch is the failure this phase is not allowed to repeat. The analytic term is a write of `E`. It is a new shade from the current lamp, so the lit side and the umbra track the lamp in the same frame.

**Quality risk.** A fringe that is thinner than the lamp's motion leaves a one-frame-late shadow edge, which is the S77B tear. A thin occluder the cell never stored is a missed shadow until the check ray hits it. The check is one ray per held probe, plus a rotating share of confirmed pixels. A fail promotes the cell and the fringe.

**Wake and verify.** A confirm is quiet. A fail is a big change: the cell is full effort, and the wake ring runs from the probes that feed the texels the cell shades. The shared field does not wake anyone.

**Paper.** Laine, Saransaari, Kontkanen, Lehtinen, and Aila, *Incremental Instant Radiosity for Real-Time Indirect Illumination*, I3D 2007: a light that moved contributes a replacement, and the lights that did not move stay. Ulbrich, Novák, Rehfeld, and Dachsbacher, *Progressive Visibility Caching for Fast Indirect Illumination*, VMV 2013, and Clarberg and Akenine-Möller, *Exploiting Visibility Correlation in Direct Illumination*, EGSR 2008: nearby diffuse points share visibility, and the cache gets denser where visibility changes. S74A is that cache for hits.

**Departure.** Laine replaces VPLs. There is no VPL set here. The replacement is the closed-form diffuse point-light term times the cached bit. Ulbrich stores distance shadow maps and accepts an approximate secondary ray. The bit is the one a shadow ray already wrote. An approximate query does not shade the picture: a disagreeing check throws the bit away and traces. Pixel direct stays a per-pixel term. It is not read from the surface cache (the reason S74B was set aside). S74A's cell covers hits and refuses pixels. This phase uses the same cell for a pixel, and the pixel still traces whenever the cell is in the fringe or the check fails.

### 2. Sparse probes, neighbours take the change

**Saved:** about 2.3 ms. Trace 1.9 → 0.9 (1.0 ms), gather 0.95 → 0.25 (0.70 ms), filter 0.87 → 0.25 (0.62 ms). The trace remainder of 0.9 ms is the still-scene check (about 0.3 ms) plus a full bounce refresh of the sparse set. Technique 5 cuts that refresh.

**What changes.** The probes a moving lamp affects are not all updated. The updated set is stratified in the world. Its spacing follows the irradiance gradient: `R` proportional to `E / |∇E|` (Ward and Heckbert 1992), clamped, and tightened where the penumbra hypothesis says the field is sharp — next to the lamp and along the fringe — and widened where the field is smooth (Sannikov; Osborne and Sannikov, arXiv:2408.14425). The gradient of this lamp's diffuse term is analytic. The stochastic residual keeps the error bound the cache already uses.

The screen probe's SH is gathered indirect. The lamp's direct term stays in the pixel (technique 1). On a probe in the sparse set, each kept hit whose visibility to that lamp is confirmed updates the probe by the change in the hit's outgoing radiance, in the same cosine-convolved order-2 SH `lighting_probe_irradiance` evaluates:

```text
ΔL_hit = (albedo / π) * V * (E_new − E_old)
SH ← SH + ΔL_hit * Y(ray direction)
```

`E` is that lamp's diffuse irradiance at the hit. `Y` is the basis the gather already uses for that ray. This walks the hits of the sparse set and does not trace them. Phase 3 proves the neighbour blend first: the sparse set still gathers for real, and the neighbours take that gathered `ΔSH`. Phase 6 replaces the sparse set's gather with this hit projection. A probe outside the set does not walk its hits, does not gather, and does not filter. Its published SH adds the interpolated `ΔSH` in the same frame, using the weights compose already has: bilinear on the 16 px grid, plane distance, normal to the 8th power. A zero weight (the face test) does not donate.

The stored hit radiances of a propagated probe stay as they were. They are not read, because the probe does not gather. If a later check fails, the probe enters full effort, relights its strata, and gathers, which is the existing woken-probe path. The region adopts that gather together, under the handover rule. A right delta never takes that path, so the SH does not snap back to stale hits.

**Quality risk.** Interpolation across a face the weights fail to drop leaks light. A sparse set that is too coarse in a corner rounds the bounce. Both are check-ray failures, and the wake is the repair. S78Z relit probes and re-filtered them, spent about 0.6 ms of filter, and did not move the stuck-patch bias. A confirm must not enter the filter. The bias S78Z found is a surface-cache mean that depends on camera history. This design matches the full-effort picture, that bias included. It does not claim to heal it.

**Wake and verify.** The one check ray per held probe stays the quiet gate (S78U: a confirm is not a rewrite). The approved ring (WAKE: golden-angle rings at `r, 2r, 4r, …`, fill in only behind a failed test, radius scales with the change, once-per-event stamp, per-frame cap) runs on a fail. S78Z's "relight a stratum and filter" verify is not revived.

**Paper.** Ward and Heckbert, *Irradiance Gradients*, Eurographics Rendering Workshop 1992, and Křivánek, Gautron, Pattanaik, and Bouatouch, *Radiance Caching for Efficient Global Illumination Computation*, IEEE TVCG 2005: sample spacing from the gradient, interpolate between samples. McGuire, Mara, Nowrouzezahrai, and Luebke, *Real-Time Global Illumination using Precomputed Light Field Probes*, I3D 2017: interpolate probes with visibility so a probe behind a face does not donate. Majercik, Crassin, Shirley, and McGuire, *Dynamic Diffuse Global Illumination with Ray-Traced Irradiance Fields*, JCGT 8(2), 2019, and Majercik, Marrs, Spjut, and McGuire, *Scaling Probe-Based Real-Time Dynamic Global Illumination for Production*, JCGT 10(2), 2021, §5–6: update a subset, and classify probes so work that does not affect the picture is pruned. NVIDIA RTXGI's probe variability (coefficient of variation, `docs/DDGIVolume.md` in NVIDIAGameWorks/RTXGI-DDGI) is the published settle test. Ramamoorthi and Hanrahan, *An Efficient Representation for Irradiance Environment Maps*, SIGGRAPH 2001, and Sloan, Kautz, and Snyder, *Precomputed Radiance Transfer*, SIGGRAPH 2002: low-order SH irradiance, and a lighting change as a coefficient change. The per-lamp layer is already the accepted store in ADR 0012, inside that ADR's moving-light layer budget. The delta is how a move updates that layer. A phase that would pass the budget stops.

**Departure.**

- McGuire's probes are precomputed and static, and they store an octahedral light field plus depth. The interpolation here is of one lamp's dynamic delta, in the order-2 SH the filter already writes, with the compose weights. No octahedral depth is added.
- DDGI blends new rays with hysteresis (`probeHysteresis` about 0.97 in RTXGI) and classifies probes Off, Sleeping, Newly Awake, Newly Vigilant, Awake, or Vigilant, after relocating them out of static geometry. Screen probes stay where the placement pass puts them. A settled probe is skipped for trace, gather, and filter, and it still receives the delta in the same frame, so an on-screen probe is not left stale. Relocation is not used.
- RTXGI's variability can pause ray tracing for a whole volume. The pause here is per probe, after that probe's own check. One lamp does not keep every probe tracing, and one quiet room does not pause a probe the lamp still invalidates.
- Sloan's transfer is precomputed for distant lighting, and Ramamoorthi's lobe is the lamp's irradiance at the point that stores the SH. The screen probe stores gathered indirect, so the delta is the change in outgoing radiance at the confirmed hits, projected with the gather's basis. The lamp's direct lobe is not added to that SH. It stays in the per-pixel term, retraced on the fringe.
- The filter stamp (S78Y) today re-runs a round when a neighbour's SH changes. A propagated delta changes the SH and would restart the three rounds, which spends the 0.62 ms again. The filter stamp means "a stochastic gather wrote". A delta write does not set it.
- ADR 0012 forbids a radiance history. The layer is the lamp's current contribution, replaced outright. It is not a blend with last frame's radiance.

### 3. A-trous runs where new samples landed

**Saved:** about 2.4 ms (3.0 → 0.6). On a frame where a-trous is 4.5 ms, the same rule is what removes the dense sweep. The remainder is the full-effort regions.

**What changes.** A-trous reconstructs a smooth field from noisy traced texels (Schied et al. 2017; Dammertz et al. 2010, as S72V). A confirmed lamp delta has no new noise. Adding it to the published field does not mark the texel for a-trous. Full-effort pages still trace and still filter, and a whole region filters together.

**Quality risk.** A wrong delta is not blurred into its neighbours. The check is the gate, which is the point of S78U. A region that mixed delta texels and traced texels would be the patch S72J rejects, so a region is entirely one or the other for that lamp's change this frame.

**Wake and verify.** A texel whose check passes and whose only change is the delta is quiet, including layer 2. A texel the wake names is full effort and filters with its region.

**Paper.** Schied et al., *Spatiotemporal Variance-Guided Filtering*, HPG 2017: the filter exists to drive variance down. Schied, Peters, and Dachsbacher, *Gradient Estimation for Real-Time Adaptive Temporal Filtering*, HPG 2018, eq. 6–7: λ from the relative temporal change. Wright, Narkowicz, and Kelly, *Lumen*, SIGGRAPH 2022 Advances: card lighting is refreshed inside a budget.

**Departure.** SVGF filters every pixel every frame. Lumen spends its budget every frame. The budget here stays a ceiling (E19, already a recorded departure). The new departure is the dirty stamp. S78B and S78X mark a texel because its displayed value moved. A delta moves the displayed value and would pull the dense sweep back. The a-trous stamp means "a stochastic sample was committed". A delta does not set it. The 1-in-16 refresh floor (S78B) stays for the pull-push clamp, which has no reverse links.

### 4. A settled pixel reads the published field

**Saved:** about 0.9 ms (compose 2.1 → 1.2). Unmeasured. The figure is a planning allowance for skipping the search, the offer, and the parent walk on a settled corner. Phase 5 replaces it. If the 2.1 ms is bandwidth of the probe SH rather than that walk, the save is smaller and the frame misses 200 fps with the other remainders met.

**What changes.** Compose still runs for every pixel of a moving frame. A settled corner reads the published shown value and bilinear-blends it. A cold corner, a first reveal, and a full-effort page keep today's lookup, including the offer and the parent seed (S72M, S72N). The seed work stays, on the pixels that have nothing published.

**Quality risk.** A settled tag on a texel that has no published value shows a hole. The classifier has to treat "no published value" as full effort. The reveal strip stays on today's seed, so the method seam S72M fixed does not return.

**Wake and verify.** The compose path does not decide that a texel is quiet. It reads the tag the check wrote.

**Paper.** Lumen composites every pixel every frame from the screen probes, and the surface cache is read at ray hits. ADR 0012 keeps a full composite. QUIET already departs from that for a held camera.

**Departure.** The moving frame does not take QUIET's block skip. The departure from Lumen and from SVGF is the body of the shader: a settled pixel does not search or seed. Pixels still read the surface cache directly, which is the existing departure, restricted to the published value on the settled path.

### 5. First-bounce delta, higher bounces stay graded

**Saved:** about 0.4 ms, trace 0.9 → 0.5. This is the bounce refresh technique 2 left in the trace remainder. The reason to do it is the bounce view, not the 0.4 ms.

**What changes.** A kept hit's direct contribution from a lamp is the first bounce. Where the hit's visibility cell confirms, that contribution is the analytic `E` times the hit albedo, written as the same SH delta. The texel direct-light cache (S75) is the store it writes. Higher bounces are the hit reading the surface cache's indirect. They are not rewritten analytically. The sparse set traces them with a ray count scaled by λ, and every texel in the region uses that same count.

**Quality risk.** The lighting record's corner view is mostly bounce 2 and later (about 87% of that picture in the S35 split). A lamp that moves in a dark corner may show the higher bounces a few frames late. That lag is accepted. Direct light and the shadow stay on the same frame, so this phase does not spend the trace remainder to force the higher bounces onto the same frame. The bounce-only diff is still recorded. If the lag is more than a few frames after the lamp stops, the sparse set's higher-bounce ray count comes up, and the 0.4 ms is given back, before any other cut is touched.

**Wake and verify.** A higher-bounce change that passes the noise bound is quiet. One that exceeds it fails the check and wakes the region. The shared field is not a wake source (S78U, WAKE).

**Paper.** The visibility papers in technique 1, plus Lumen's cached direct lighting on cards, which S75 already stores for GI hits. Gautron, Bouatouch, and Pattanaik, *Temporal Radiance Caching*, IEEE TVCG 13(5), 2007: the lifespan of a record shortens as the incoming radiance changes faster.

**Departure.** Lumen retraces card direct on the budget. A confirmed cell is rewritten analytically. Gautron eases a record across frames with a temporal gradient so the value does not pop. The analytic first bounce is applied in the same frame. The longer lifespan applies only to the unresolved higher bounces, and only as a ray schedule. The presented frame is not an interpolation of last frame's picture. ADR 0012's rejection of radiance history stands for that picture.

### 6. One grade for the region

**Saved:** 0 to about 0.3 ms on the stress scene, from a tighter fringe and fewer residual rays when λ is small. A move large enough to saturate λ saves nothing here and spends full effort. The 4.3 ms planning sum does not depend on this 0.3 ms.

**What changes.** λ is A-SVGF eq. 6, the relative change of the lamp's term at the region. It sets the regime, the fringe thickness, the sparse-set spacing, and the higher-bounce ray count. Every page in the region gets that one number. E19 already scales texel rates this way. This extends the same grade to the fringe and the probe set.

**Quality risk.** The swim Anthony named against non-uniform updates (E19). The region rule is the mitigation. A grade that changes every frame inside one wall pops. The handover rule is the mitigation.

**Wake and verify.** λ nominates checks. It does not skip them (S74A: a hint nominates, the check decides). A confirm at low λ stays quiet. A confirm at high λ still means the analytic term is legal; the extra rays are for the stochastic residual, not a second copy of the direct term.

**Paper.** Schied, Peters, and Dachsbacher, HPG 2018, eq. 6–7. Ward, Rubinstein, and Clear, *A Ray Tracing Solution for Diffuse Interreflection*, SIGGRAPH 1988, for the error bound that says a sample is due.

**Departure.** A-SVGF has no ray-count rule. The lighting record already lists that departure (S74A, S76). This design uses the same departure for the fringe and the probe spacing. Eq. 7's history blend is not taken. The analytic term replaces the lamp's contribution in one frame.

### 7. Stochastic many-light sampling stays shelved

**Saved:** about 0 ms on the moving stress scene.

**What the papers do.** Bitterli, Wyman, Shirley, and others, *Spatiotemporal reservoir resampling for real-time ray tracing with dynamic direct lighting*, SIGGRAPH 2020, reuse a chosen light sample across pixels and across frames. Ouyang, Liu, Kettunen, Pharr, and Pantaleoni, *ReSTIR GI*, HPG 2021, do the same for paths. Narkowicz and Costa, *MegaLights: Stochastic Direct Lighting in Unreal Engine 5*, SIGGRAPH 2025 Advances, trace a fixed shadow budget against a stochastic subset and denoise.

**What was already measured.** S78Q (ReSTIR DI on pixels) was shelved by Anthony. The moving scene has about five lamps per pixel. Q4 left the light pass at 6.38 ms against 6.59 ms, raised noise, and cost 16.6 MiB. The 100-lamp room did drop (47.9 → 35.2 ms). MegaLights is the same shape of cut: it pays when the lamp count, not the shadow-ray grid, is the cost.

**Departure, if a later scene picks it up.** ADR 0012 rejects a radiance history and a per-frame-rotated sample pattern. MegaLights and ReSTIR both depend on temporal reuse and, for MegaLights, a denoiser. A later many-light phase would take a same-frame spatial share of visibility inside a region the check has called uniform, and would leave the temporal reservoir and the denoiser out. That phase is not part of this cut. The fixed budget as a ceiling is already E19.

## Departures

Collected so a phase can copy its row into the lighting record's deviation list when it lands. Rows 1–13 are not in the shade. Row 14 is: a small sun step amortises indirect light.

| # | Paper | What this design does |
|---|---|---|
| 1 | Laine et al. 2007, VPL replacement | Closed-form diffuse term times a cached visibility bit. No VPLs. |
| 2 | Ulbrich et al. 2013, approximate cached visibility. S74A stores the bit for hits and refuses pixels. | The bit is one a shadow ray wrote. A disagreeing check traces. A pixel may read the hit cell, and it traces on the fringe and on a failed check. |
| 3 | McGuire et al. 2017, static octahedral light-field probes | Dynamic per-lamp delta, order-2 SH, compose weights. |
| 4 | Majercik et al. 2021, six probe states, relocation, hysteresis | No relocation, no hysteresis blend. A settled on-screen probe still receives the delta the same frame. |
| 5 | RTXGI volume variability pause | Per-probe confirm. A volume is not paused as a whole. |
| 6 | Sloan et al. 2002 and Ramamoorthi and Hanrahan 2001, a lighting change as coefficients at the storing point | The delta is the change in outgoing radiance at confirmed hits, in the gather's basis. The lamp's direct lobe is not written into the probe SH. |
| 7 | S78Y / S78B stamps on any value change | Filter and a-trous stamps mean a stochastic write. A delta does not set them. |
| 8 | SVGF and Lumen filter or spend every frame | Settled regions with a noise-free delta do not run a-trous. Budget stays a ceiling. |
| 9 | Lumen retraces card direct; Gautron eases records across frames | Confirmed first bounce is analytic and same-frame. Higher bounces take a longer ray interval. The picture is not eased in from last frame. |
| 10 | A-SVGF eq. 7 history blend, and no ray-count rule | λ sets regime, fringe, spacing, and residual rays. The ray-count departure is already listed for texels. Eq. 7 is not taken. |
| 11 | MegaLights / ReSTIR temporal reuse and denoising | Not used. S78Q stays shelved. |
| 12 | Sannikov radiance cascades, as a tracer | Spacing only. ADR 0008 stays superseded. |
| 13 | ADR 0012 per-moving-light layer | The layer is the store the delta edits. A shortcut that patches the summed SH and drops the layer conflicts with the ADR and has to be named in that phase. |
| 14 | SVGF (Schied et al. 2017) and Lumen recompute every pixel every frame. Lumen relights surface-cache direct every frame. | A small directional-sun step is not a full dirty. Indirect light refreshes one band of probe rows and texel pages per frame, over 8 frames, while the step from the last full refresh stays inside 2° or 2% sun/sky irradiance. Direct sun light and its shadows are traced and shown every frame. |

Ward's `R_geo` term stays omitted, which S72V already records. This design does not add it.

## Phased plan

Each phase is one change, revertible on its own. The full-effort reference is today's path: a switch that marks every probe and every texel full effort. Until phase 1 that switch is unnecessary, because today is full effort.

The picture check, every phase after phase 0:

- Same script, same frame index, linear luminance of the lighting result before the tone curve.
- Three diffs against the full-effort render: the sum, the direct view, and the bounce view (debug views 1 and 2).
- Phase 0 records the noise floor by diffing two full-effort runs of that script. A later phase passes when each of its three mean `|ΔL|` values is at or under that floor.
- Pop is the existing fill-run metric: the share of pixel-frames whose linear luminance steps by more than 0.01 from their 9-frame average, at frames 20–80 and 80–320, reported as a percent. The candidate's pair does not exceed the full-effort pair from the same script.
- The moving-shadow smoke frames are written and looked at. A torn edge fails the phase even when the means pass (S77B).

A small headless diff under lavapipe can show the switch and the bounds. The milliseconds and the visual judgment are a short run on the owner's RTX 4070, left as a command. No phase is a full `gi-check` or a path trace.

A phase that misses its picture check is reverted. A phase that passes the picture and misses its millisecond check is reported and stopped on. The later phases do not stack on a cut that failed.

### Phase 0 — lock the reference

No behavior change. Record the moving-scene pass split and the still-scene pass split, and write the full-effort sum, direct, and bounce frames for the lamp move and the shadow walk. Diff two full-effort runs and store the three mean `|ΔL|` floors and the two pop percents.

**Pass:** the floors exist, and the three full-effort runs agree. The floor for a view is the largest of the three pairwise mean `|ΔL|` values. Each pair stays within three times the smallest pair, plus 5e-4. A later phase passes when its diffs against the full-effort runs sit in that same band, and its pop stays within half a percentage point of the full-effort pop. **Milliseconds:** none expected. The lavapipe harness is `crates/render/tests/lighting_work.rs`. It draws the corner scene at 320×180, records the three floors and the pop shares, and writes a contact sheet under `~/genos-perf/bounce-corner/`. The stress-building milliseconds stay on the owner's machine.

Lavapipe, 320×180, one prime run then three cold runs of the corner scene:

| View | Floor | Pop |
|---|---:|---:|
| Sum | 0.021431 | 46.8% |
| Direct | 0.000000 | 61.4% |
| Bounce | 0.004051 | 46.0% |

Direct matched across the three runs. The sum and the bounce stay inside the band above. The pop is the share of pixel-frames that step by more than 0.01 during the lamp move, so it is high on this short clip. The owner's 1080p window measures its own floor.

### Phase 1 — tag regimes, change no work

Mark each probe and each page full effort or settled, from the check and from λ. Keep running today's work for both. Counters: the settled fraction of probes, of pages, and of pixels, on the moving scene and the still scene. A debug view shows the tag.

The tag is read-only. A surface pixel is settled when its 12.5 cm texel already holds at least one set, its λ is under 0.25, and the pixel is outside the fringe. The fringe is a tube of radius equal to this frame's lamp motion around the segment from the point to the lamp. The walk ignores the floor, the roof, and an occluder that contains the point, and it starts 15 cm off the surface. A tube is wider than the swept triangle, so the tag stays full effort on a superset of the pixels whose visibility can flip. A probe is settled when the gather hold already skips it. A moving lamp dirties that hold, so probes stay full effort through the move. The pixel stop rule is the one that gates later phases. The shaded work is today's path for both tags. `GENOS_LIGHTING_FULL_EFFORT=1` forces every count to full effort. The default phase is 1.

**Pass:** picture diff inside the phase 0 floor, pop not up. **Stop rule:** after warmup, if fewer than half the pixels on the moving run are settled, the premise is wrong for this scene and the later phases are not started. **Milliseconds:** about 0.

Lavapipe, 320×180, measured in the same process as a fresh phase 0 floor: direct diff 0 against all three full-effort runs. Sum diffs 0.0205, 0.0243, 0.0173 sit in the pair band (floor 0.0239). Bounce diffs 0.0043, 0.0037, 0.0022 sit in its band (floor 0.0046). Pop stays within half a point. After 24 still frames and a lamp step, 56523 of 57600 pixels are settled (98.1%), 1077 are fringe, none are missing. The regime view matches those counts. The stop rule does not fire, so later phases start.

`lighting_moving_camera_keeps_its_brightness` misses its 0.03 gate on this lavapipe at the phase 0 commit (wiggle ratio about 1.04) and with full effort forced. That miss is older than the tag. `lighting_pipelines_build_quickly` counts the pre-existing `lighting gbuffer` timing name along with the nine lighting compute pipelines, so the same assertion fails on gi-v2.

### Sun step — amortise the indirect refresh

This lands before phase 2. PR #26 measured the moving stress scene at 1920×1080: the day-clock sun moves every frame, so every 8×8 compose block (32400/32400) and the a-trous dirty fraction (above 0.25) are dirty. The list is the whole picture. Skipping blocks cannot save the compose or the a-trous pass until a small sun step stops marking all of them.

A step is small when the sun's direction has moved less than 2° from the last full refresh and the global sun/sky light (`lighting_global_light`) has moved less than 2%. The camera, the meshes, and the point lamps have to be holding. A larger step, a sun appearing or disappearing, or a moved point lamp is a full refresh.

While the step is small:

- Every pixel traces the live sun. The direct view and the sun's shadow are this frame's. The compose list does not gain a block only because that packed direct changed.
- A direct patch, the same compose shader, adds that direct to the bounce stored last time the pixel was fully composed. A clean block does not keep last frame's sun.
- Indirect light uses the sun from the last full refresh, except one band of probe rows and one band of texel pages (`frame % 8`). That band is shaded with the live sun. Over 8 frames the bands cover the picture.
- The a-trous input keeps the committed sun/sky scale, so a tick does not move every filtered texel. The band uses the live scale. The band is a spatial page column. The hashed page id is scattered, and an a-trous neighbourhood of that set would fill the field.
- The step does not reset the surface cache's light-quiet counter. The sun direction is in the light hash, and taking the new hash would relight the room as a whole-scene light change.
- The sun's shadow epoch does not move. Hit visibility stays on the committed sun. Pixels do not read that cache.

`GENOS_LIGHTING_FULL_EFFORT=1` turns this off. The corner scene has no sun, so phase 1's picture is unchanged. `GENOS_LIGHTING_DEBUG=1` prints `lightingcompose` with the block count (`0xffffffff` is a full compose) and whether the sun path was on.

**Pass:** on a held camera and a sun step inside the tolerance, the compose list is under half the image and the a-trous dirty fraction stays at or under 0.25. The direct view stays inside the phase 0 floor against full effort. A 40° jump is a full compose. **Milliseconds:** not claimed. The owner's 1080p window is the measurement. Lavapipe does not time it.

### Phase 2 — confirmed-visibility shade

Technique 1. Wide fringe: every segment this frame's lamp motion or a mover bound could flip is traced. Confirmed points are off the light dispatch.

**Pass:** the three diffs and the shadow smoke. **Milliseconds:** light toward 1.5 ms. A list-build that costs more than a few tenths, or a light pass that stays near 4.7 ms, fails the time check the way S77B did.

Tried once and reverted. The list ran, and the analytic shade wrote a 12.5 cm cell bit that does not match a per-pixel shadow.

1. Claim: confirmed pixels can take the cached visibility and leave the trace.
2. Check: lavapipe 320×180 corner, direct view, mean `|ΔL|` against full effort. The floor is 0. The band is 5e-4.
3. Fault: the first moving frame's direct `|ΔL|` was 0.42. Traced and analytic counts were both large, so the miss is the shade, not a dead dispatch.
4. First seen: `lighting_work` on this branch, before the sun step.
5. Left reverted: the shade is not in the tree. A later attempt does not start from that cell bit.

### Phase 3 — sparse probes and propagation

Technique 2's probe skip, the same-frame blend of the delta, and the check-ray quiet gate. The sparse set still gathers for real. The wake ring runs only on a fail. Propagated probes do not gather or filter. The filter stamp ignores the delta.

**Pass:** the three diffs, plus a coverage view so a probe the blend reached with weight 0 and no other donor is visible. **Milliseconds:** trace toward 0.9 ms, gather toward 0.25 ms, filter toward 0.25 ms. Still-scene filter and gather do not rise (the S78Z regression).

### Phase 4 — a-trous stamp

Technique 3. The a-trous stamp ignores the delta. Full-effort regions still filter as a region.

**Pass:** the filtered-field diff as well as the picture, so a smooth wrong field fails. Pop, including a page edge inside one wall. **Milliseconds:** a-trous toward 0.6 ms on the planning frame, and the dense sweep stays off on a frame that today sits at the 4.5 ms end, unless that frame is genuinely full effort.

### Phase 5 — settled compose read

Technique 4. Every pixel is still shaded. A settled corner reads the published value.

**Pass:** the reveal strip against phase 0 (the S72M seam), and the three diffs. **Milliseconds:** compose, whatever it measures. The planning allowance is 1.2 ms. A compose that stays at 2.1 ms is a reported gap, not a silent pass.

### Phase 6 — first bounce and the grade

Techniques 5 and 6. The fringe may shrink only inside the phase 0 floor. Higher-bounce rays on the sparse set scale with λ, one count for the region.

**Pass:** direct light and the shadow stay inside the phase 0 floor on the same frame. The bounce view may miss that floor while the lamp moves and for a few frames after it stops. If the bounce is still outside the floor once those frames have passed, raise the residual ray count and spend the 0.4 ms back. **Milliseconds:** trace toward 0.5 ms. Still scene at or under the phase 0 still split.

### Phase 7 — still collapse

No new cut. The still stress scene runs the settled path. Light, trace, gather, filter, and a-trous for settled regions are empty apart from the one check ray per held probe. A still frame does not regress past phase 0.

**Pass:** still-scene picture inside its own floor, pop not up, and the pass split at or under phase 0's still split.

## Probes a phase adds

The debug standard is the inventory. Each phase that changes behavior adds its probe in that change. This document does not add them.

| Failure | What shows it |
|---|---|
| A cold texel tagged settled | Regime view. Counter: settled pages whose published value is missing. |
| Fringe thinner than the motion | Fringe mask. Direct-view diff. |
| Blend donated across a face | Propagated-delta view. The existing plane-weight rejection. |
| Two grades on one wall | Grade view. Pop at a page edge. |
| Wake cap saturated | The wake cap counter from the WAKE design. |
| A confirm restarted the filter or a-trous | Those passes' dirty fractions, next to the stochastic-write count. |
| Hot-path readback | No host-visible atomic in the confirm or the delta (S78W). |

Tunables, each with a counter: the λ that forces full effort, the fringe scale, the minimum sparse-set spacing, the wake cap, the check-ray count per held probe (default one).

## Networking

Lighting stays a picture on the machine that draws. The five lines, for the system record when a phase first changes behavior:

- Replicated state: none. Probe SH, visibility bits, layers, and deltas live on the GPU that draws. Lamp poses and meshes are the scene's replicated inputs.
- Authority: the drawing machine owns the caches. A later server does not simulate irradiance.
- Prediction / rollback: a rollback restores the scene and drops the caches. The next frame is full effort, the same as a cold start.
- Determinism: the full-effort path is the reference render. The settled path is allowed to be stale and is not required to match bit for bit across machines. The picture is not a replicated input.
- Headless server: a headless server runs no lighting passes. The headless debug path still runs them when it is scoring a picture.

## Decisions

1. **Baseline.** The line to beat is the owner's 1080p windowed moving run, about 14.5 ms. The older 18 ms row at 1920×1080 is not the line. Lavapipe does not time it. `GENOS_GPU_TIMES=1` on that window is the measurement.
2. **200 fps is aspirational.** A measured cut of several times, with the picture inside the phase 0 floor, counts. The planning sum, about 5.3 ms and 190 fps, is an acceptable result if the owner's run measures it. It is not claimed here.
3. **Bounce lag.** Indirect light in a dark corner may lag a few frames after a lamp moves. Direct light and shadows stay on the same frame. Phase 6 does not spend the trace remainder to pull the higher bounces onto that same frame. The owner judges the lag on `examples/stress/scripts/bounce_corner.rhai`.
4. **Moving lamps and layers.** The stress scene's default mix is 5 lamps with half of them moving, and a pixel sees about 5 lamps (S78Q). That moving set fits the Low layer cap. ADR 0012 and the GI v2 design give each moving light its own layer, with a cap of 8 at Low and 32 at Ultra (`docs/systems/gi-v2-design.md`, Light layers). This cut stores a layer for lamps that move this frame, up to 8. A lamp past that cap stays on the full-effort path. There is no per-lamp layer buffer in the tree today. Phase 3 keeps the delta in an 8-wide layer next to the probe SH and adds it back with a replace, so the summed SH is not patched in place and then left as the only store. Departure 13 names that the layer is this buffer, not the unbuilt card-atlas layer.
5. **Shadow edge.** A one-frame-late shadow outside the traced fringe is a failure even when the pop metric holds. Phase 2 traces every segment this frame's lamp motion or a mover bound could flip. Phase 6 may shrink that fringe only while the direct view stays inside the phase 0 floor.
6. **Atom.** The scheduling atom stays the 8×8 texel page. Pages one blend connects, in one λ bucket, show one grade. The atom is not a mesh and not a room.
7. **Camera-history bias.** S78Z's surface-cache bias is matched, not fixed. A texel-verify fix is not a precondition.
8. **Volume pause.** RTXGI's volume-wide pause stays out. A probe goes quiet on its own check.
9. **Many lights.** MegaLights and S78Q stay out, including the 100-lamp room. That room is a different task.
10. **Small sun.** A directional sun that moves a little is not a full dirty of compose or a-trous. Indirect light for that step refreshes over 8 frames, inside 2° or 2% irradiance of the last full refresh. Direct sun light and its shadows stay on this frame. Departure 14. A large step is a full refresh.

## Owner run

Lavapipe locks the picture. The milliseconds and the bounce lag are judged on the owner's machine, windowed, 1920×1080.

The corner scene is `examples/stress/scenes/bounce_corner.rhai`: two dark walls, no sun, no sky, one lamp. `examples/stress/scripts/bounce_corner.rhai` lets the picture settle, moves that lamp toward the corner, then stops it. Watch the corner, not the lamp bulb.

- The lit side of the wall and the shadow edge move with the lamp, on the same frame. A shadow that slides in a frame late fails the run.
- The bounce in the corner, the part no lamp reaches directly, may keep the old tint for a few frames after the lamp stops. It should then catch the new lamp. A corner that stays on the old tint fails the run.

```text
mkdir -p ~/genos-perf
cargo run --release -p genos-stress -- --scene examples/stress/scenes/bounce_corner.rhai --script examples/stress/scripts/bounce_corner.rhai --size 1920x1080 --no-panel --report ~/genos-perf/bounce-corner
GENOS_LIGHTING_FULL_EFFORT=1 cargo run --release -p genos-stress -- --scene examples/stress/scenes/bounce_corner.rhai --script examples/stress/scripts/bounce_corner.rhai --size 1920x1080 --no-panel --report ~/genos-perf/bounce-corner-full
GENOS_GPU_TIMES=1 cargo run --release -p genos-stress -- --size 1920x1080 --frames 120
GENOS_LIGHTING_FULL_EFFORT=1 GENOS_GPU_TIMES=1 cargo run --release -p genos-stress -- --size 1920x1080 --frames 120
```

The first command is the reduced path once the phases are on. The second is the full-effort picture. The last two are the moving stress scene, reduced and full effort. Shots from the script land in the report directory. The lavapipe check also writes a contact sheet under `~/genos-perf/bounce-corner/`.

`GENOS_LIGHTING_WORK_PHASE` (0–7) stops after that phase. Unset, the phase in the code is the one the last landed change turned on. `GENOS_LIGHTING_FULL_EFFORT=1` marks every probe and every texel full effort.

## Sources

- Majercik, Crassin, Shirley, McGuire. *Dynamic Diffuse Global Illumination with Ray-Traced Irradiance Fields.* JCGT 8(2), 2019. <https://jcgt.org/published/0008/02/01/>
- Majercik, Marrs, Spjut, McGuire. *Scaling Probe-Based Real-Time Dynamic Global Illumination for Production.* JCGT 10(2), 2021. <https://jcgt.org/published/0010/02/01/>
- NVIDIA RTXGI, probe variability. <https://github.com/NVIDIAGameWorks/RTXGI-DDGI/blob/main/docs/DDGIVolume.md>
- Wright, Narkowicz, Kelly. *Lumen: Real-time Global Illumination in Unreal Engine 5.* SIGGRAPH 2022 Advances. <https://advances.realtimerendering.com/s2022/SIGGRAPH2022-Advances-Lumen-Wright%20et%20al.pdf>
- Bitterli, Wyman, Shirley, et al. *Spatiotemporal reservoir resampling for real-time ray tracing with dynamic direct lighting.* SIGGRAPH 2020.
- Ouyang, Liu, Kettunen, Pharr, Pantaleoni. *ReSTIR GI: Path Resampling for Real-Time Path Tracing.* HPG 2021.
- Narkowicz, Costa. *MegaLights: Stochastic Direct Lighting in Unreal Engine 5.* SIGGRAPH 2025 Advances. <https://advances.realtimerendering.com/s2025/content/MegaLights_Stochastic_Direct_Lighting_2025.pdf>
- Sannikov. *Radiance Cascades.* And Osborne and Sannikov, arXiv:2408.14425. Links in [ADR 0008](../../adr/0008-lighting-uses-radiance-cascades.md).
- McGuire, Mara, Nowrouzezahrai, Luebke. *Real-Time Global Illumination using Precomputed Light Field Probes.* I3D 2017. <https://doi.org/10.1145/3023368.3023378>
- Ward, Rubinstein, Clear. *A Ray Tracing Solution for Diffuse Interreflection.* SIGGRAPH 1988.
- Ward and Heckbert. *Irradiance Gradients.* Eurographics Rendering Workshop 1992.
- Křivánek, Gautron, Pattanaik, Bouatouch. *Radiance Caching for Efficient Global Illumination Computation.* IEEE TVCG 2005.
- Ramamoorthi and Hanrahan. *An Efficient Representation for Irradiance Environment Maps.* SIGGRAPH 2001.
- Sloan, Kautz, Snyder. *Precomputed Radiance Transfer for Real-Time Rendering in Dynamic, Low-Frequency Lighting Environments.* SIGGRAPH 2002.
- Laine, Saransaari, Kontkanen, Lehtinen, Aila. *Incremental Instant Radiosity for Real-Time Indirect Illumination.* I3D 2007.
- Ulbrich, Novák, Rehfeld, Dachsbacher. *Progressive Visibility Caching for Fast Indirect Illumination.* VMV 2013.
- Clarberg and Akenine-Möller. *Exploiting Visibility Correlation in Direct Illumination.* EGSR 2008.
- Gautron, Bouatouch, Pattanaik. *Temporal Radiance Caching.* IEEE TVCG 13(5), 2007.
- Schied, Peters, Dachsbacher. *Gradient Estimation for Real-Time Adaptive Temporal Filtering.* HPG 2018.
- Schied et al. *Spatiotemporal Variance-Guided Filtering.* HPG 2017.
- Dammertz, Sewtz, Hanika, Lensch. *Edge-Avoiding À-Trous Wavelet Transform for Fast Global Illumination Filtering.* HPG 2010.
