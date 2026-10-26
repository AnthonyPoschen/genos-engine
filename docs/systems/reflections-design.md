# Specular reflections, mirrors, portals, and water

Status: **accepted** (Engine bot, 2026-10-26). Anthony Poschen delegated design approval to the Engine bot, which accepted [ADR 0019](../adr/0019-reflections.md). Terms: [`docs/lighting/GLOSSARY.md`](../lighting/GLOSSARY.md).

This design adds the specular picture on top of the live GI path. The frame composite, the planar views, and the water quad are in the renderer. Glossy marches stay out of the GI passes. The measurement harness is `reflections-check`.

ADR 0019 is accepted. That lifts the specular-and-mirrors non-goal in the [GI design](gi-design.md) (ADR 0012) and the [extras-plan](rendering/extras-plan.md) sentence that said the screen probes are the water reflection. The rest of ADR 0012 stays in force: the world-space cache, no radiance history, ray queries rather than ray-tracing pipelines, the software tracer on macOS, Vulkan 1.3, and the 4 GB floor. Cost follows [ADR 0013](../adr/0013-lighting-invalidation-is-local.md): a work cap per preset, finished values only, and no full-frame fallback.

## Goals

1. A glossy surface, a planar mirror, a portal, and a water surface match a path-traced reference closely enough to pass the numeric gates below. Near the camera the match is the tighter gate.
2. The picture does not flicker and does not pop. A light change shows up in the specular term in the same frame the hit is valid. A value that is waiting is older or coarser, and it is finished.
3. The basic scene stays above 500 fps at 1440p on an RTX 4070. The stress scene stays near 100 fps (a 10 ms frame) at 1440p on that GPU. Low holds its cap on a GTX 1060 6 GB and an RX 580 at 1080p.
4. Low runs with zero reflection rays. `VK_KHR_ray_query` is optional. macOS uses the software tracer when a preset asks for rays. Low does not ask.
5. Every failure mode and every tunable has a probe. The P0 probe is the harness. The later probe is `debug/reflections` on the shared debug scaffolding.

## Non-goals

- Radiance cascades, hit lighting, and a second light cache. Caustics and mirror views read the surface cache that GI already owns.
- Stochastic multi-sample glossy rays, temporal radiance accumulation, and a denoiser that lags a step by more than two frames.
- Fog, volumetrics, and god rays.
- Walking through a portal. That is gameplay and waits on networking. This design draws the opening.
- FFT oceans, spray, and a second water simulation. The displacement is a fixed Gerstner sum.
- glTF specular extensions (`KHR_materials_specular`, iridescence, anisotropy, dispersion). They stay `loaded, unsupported`. The material is metallic-roughness plus an IOR.
- Ultra as its own reflection preset. Ultra uses the High caps.
- Copying Lengyel's OpenGL projection listing. Vulkan clip space gets its own derivation.

## Fit

The frame already has a G-buffer, a surface light cache, screen probes, a software SDF tracer, and an optional `VK_KHR_ray_query` tracer behind one interface (ADR 0012). Reflections use those. They do not add a tracer.

Roughness at or above `0.6` is the diffuse gather. Lumen reuses diffuse rays for rough reflections (Wright et al., SIGGRAPH 2022, "Reflection denoising": "reuse diffuse rays"). That band does not march, does not trace, and does not allocate a hit. The cutoff is a material boundary, which ADR 0013 allows.

Roughness below `0.6` and at or above `0.02` is the glossy band: one screen march, then a world-cache read, then a budgeted ray on Medium and High. Roughness below `0.02` on a surface marked planar is a planar mirror. The same roughness on a curved surface is one sharp ray on Medium and High, or a cache read on Low.

The specular direction is `reflect(I, N)` with `I` pointing at the surface, the GLSL form. It is fixed by the shading normal and the view. It does not rotate per frame. Stored cache values stay world-space. The camera only orders which hits, which views, and which caustic texels get this frame's budget.

The basic scene, as it stands, has no planar mirror and no water. Pixels at roughness ≥ 0.6 take the gather. The reflection classify then returns. Its ceiling is 0.05 ms, so the 2 ms basic frame (500 fps) still holds. A later scene that adds a mirror is remeasured with that view inside the cap. It does not get a second, uncapped pass.

Compose becomes, for a dielectric in the specular band,

```text
pixel = diffuse * (1 - F) + specular * F
```

and for a metal,

```text
pixel = specular * base_colour
```

with `F` the Fresnel weight. Today's Lambert compose stays until the phase that turns the lobe on. The lobe replaces that overlap. It is not added on top of a full Lambert term.

A reflection ray that hits reads the surface cache's finished value at that point. It does not evaluate the material (Lumen's hit-lighting mode). Lumen's default is the same cache read; hit lighting is the expensive option their docs tell you to turn on when you can spend it ([Lumen technical details](https://dev.epicgames.com/documentation/unreal-engine/lumen-technical-details-in-unreal-engine), "Wanting Highest Quality Mirror Reflections"). We do not offer that option.

Priority, when a budget is short, is the ADR 0013 score with the near-camera term first: projected error near the eye, then roughness (sharper first), then age. Ten percent of each reflection budget stays a starvation slice, as in that design. A miss that has no cache entry yet shows the ambient fill, which is the ADR 0013 last resort, not black.

## Optics

These functions live in `genos-math` (`crates/math/src/optics.rs`). `crates/render/shaders/reflections.glsl` is the same formulas. The frame composite (`reflections_frame.comp`) and the water fragment include that file. `reflections_check.comp` compiles it as well and is the lavapipe parity dispatch, not a draw. A reflection pass includes the GLSL file when it calls one of them. Live rows grade the Rust functions against baked numbers. `reflections_check.comp` is also dispatched on lavapipe: every optics output in that buffer matches `genos_math::optics` within `1e-5`. A push constant that forgets to invert the below-water index ratio fails that check.

**Reflection direction.** `R = I - 2 dot(N, I) N`. A point `p` through a plane with unit normal `N` through the origin is `p - 2 dot(p, N) N`. The harness reads the reflected colour of its own screen patch (`mirror_reflect`) and the reflected point through the mirror window (`mirror_point_m`).

**Dielectric Fresnel.** Unpolarized exact Fresnel. `eta_t_over_eta_i` is `n_transmitted / n_incident`. From air that ratio is the material IOR. Water is 1.333. Glass defaults to 1.5. From under water the ratio is `1 / 1.333`, and a ray past the critical angle returns 1 (total internal reflection). Schlick's approximation (Schlick, Computer Graphics Forum 13(3), 1994; Karis, SIGGRAPH 2013) is the metal term only, with F0 equal to the base colour, because glTF does not store a complex IOR. On water, Schlick's max absolute error against the exact equations is 0.059, which misses the 0.02 gate. Gates: `fresnel_abs` ≤ 0.02 from air, `fresnel_below_abs` ≤ 0.02 from water. `water_f0` reads the flat normal-incidence patch. Reflection there is white and transmission is black, so the pixel is the Fresnel weight. The row is `|mean channel − 0.0204|`. `0.0204` is `((1.333 − 1) / (1.333 + 1))²` rounded to four decimals. The band is 0.005, so a glass F0 of 0.04 fails.

**Snell.** `eta = n_incident / n_transmitted`. Total internal reflection returns no transmitted direction; the pixel keeps the reflection. Gate: `snell_transmit` ≤ `1e-4` on the transmission patch, which does not share pixels with the F0 patch or the reflection patch. Walter et al. (EGSR 2007) supply the GGX NDF and the refraction microfacet. The runtime transmits along the single Snell direction of the shading normal. It does not draw a random BTDF sample. The Gerstner normal is what spreads water.

**GGX.** `D(h) = α² / (π (dot(n, h)² (α² − 1) + 1)²)` with `α = roughness²` (glTF). The projected integral of `D` is 1 (Walter et al.). Smith masking (Heitz, JCGT 2014) multiplies the lobe. The gate is the NDF integral, not a claim that the full BRDF integrates to 1. Gate: `ggx_energy` ≤ 0.03.

**Absorption.** `exp(-σ d)` along the transmitted path. Gate: `beer_rel` ≤ 0.02.

**Flat water.** `flat_water_transmittance` is `1 - F` at normal incidence. `caustic_rel` does not grade it. That row only tests the comparator.

## Alternatives

### Glossy reflections

| | Approach | Why it loses or wins |
|---|---|---|
| A | Stochastic screen-space reflections only (Stachowiak, SIGGRAPH 2015) plus a temporal accumulator | Off-screen mirrors disappear. The accumulator is the radiance history ADR 0012 refuses. |
| B | A reflection ray for every glossy pixel, hardware only, denoised with SVGF (Schied et al., HPG 2017) | About a quarter of users and macOS have no ray queries (ADR 0012). The 1060 cannot spend that many rays. SVGF's temporal pass lags. |
| C | **Pick.** Screen march, then the world cache, then an optional ray on the shared tracer (Wright et al., SIGGRAPH 2022; [Lumen technical details](https://dev.epicgames.com/documentation/unreal-engine/lumen-technical-details-in-unreal-engine)) | The stack already exists for GI. Low drops the ray and still has a finished colour. One direction per pixel, so the noise does not crawl. |

Departure from Lumen: Low schedules zero reflection rays, where Lumen still traces a software ray on a screen miss. Departure from Stachowiak: one direction, not a random set. Departure from Lumen hit lighting: the ray reads the cache.

### Denoise

| | Approach | Why it loses or wins |
|---|---|---|
| A | SVGF temporal accumulation plus à-trous (Schied et al., HPG 2017) | A step rides the history. The engine already refused this for GI. |
| B | A-SVGF gradient history (Schied et al., HPG 2018) or NVIDIA ReBLUR | Both shorten a radiance history. They still keep one. |
| C | **Pick.** Store the hit. Re-shade it from the cache in the same frame. One spatial à-trous pass, variance-guided, on a budgeted list | A step reaches 90 % on the frame the hit is valid. The lag gate is ≤ 2 frames, which leaves one frame for the change record. An exponential blend at 0.08 fails that gate (~28 frames). |

Lumen's reflection slide says "spatial and temporal reuse" (Wright et al., SIGGRAPH 2022). We take the spatial reuse and the diffuse-ray reuse for roughness ≥ 0.6. We do not take the temporal reuse. The à-trous kernel is the one the GI denoiser already runs. S79 refused a half-resolution à-trous for the GI picture. The reflection *trace* on every preset is half resolution; the GI à-trous is unchanged. Near pixels (closer than 6 m) are the first claim on the ray budget, which is how half-resolution tracing still answers the near-camera rule. If `near_weighted_rel` misses 0.03 once pictures exist, the next phase spends the mirror-target bytes on a full-resolution near band inside the same 32 MB cap.

### Mirrors, portals, water

| | Approach | Why it loses or wins |
|---|---|---|
| A | Ray-query mirrors only | Low has no rays. A mirror of a mirror becomes a ray tree with no cap. |
| B | Screen-space colour copied into the mirror | The reflected room is wrong as soon as it leaves the screen. |
| C | **Pick.** One extra raster per view, camera reflected through the plane (Diefenbach, PhD, Pennsylvania, 1996) or mapped through the portal (Luebke and Georges, I3D 1995), clipped by an oblique near plane (Lengyel, Journal of Game Development 1(2), 2005, pp. 5–16). Valve's Portal and Unreal planar reflections ship the same budgeted recursion. | The view is a real G-buffer plus the existing lighting. The cap is a hard ceiling. A view that does not fit shows the world-cache reflection, which is finished. |

Water, same shape of choice:

| | Approach | Why it loses or wins |
|---|---|---|
| A | Two scrolling normal maps and a looping caustic texture | The path tracer cannot match a texture that is not the flux. |
| B | Tessendorf's FFT ocean (SIGGRAPH 2001 course) plus a volumetric caustic | Too much for a 1060, and a room-scale shore does not need an FFT. |
| C | **Pick.** A fixed Gerstner sum (Finch, GPU Gems, chapter 1, 2004), exact Fresnel, one Snell refraction of the opaque colour, Beer–Lambert, an image-space caustic map (Shah, Konttinen, and Pattanaik, IEEE TVCG 13(2), 2007, pp. 272–280) written into the surface cache, and a shoreline clamp. | The displacement is world-anchored, so the cache key does not follow the camera. Flat water has an analytic flux the harness grades. |

Departure from Lengyel: the paper's listing edits an OpenGL projection (clip Z in −1..1). Vulkan clip Z is 0..1. The implementation derives the row. The harness gates the predicate (a point behind the plane is not drawn), not the matrix coefficients. Departure from Luebke and Georges: they use portals as a visibility walk of the whole scene. We fill the opening and stop at the view cap. Departure from Shah: the caustic map is stored in the world cache, so a camera turn does not drop it. Departure from Tessendorf: no FFT. Jacobian foam (the horizontal Jacobian below 0.5) is the foam term we do take from those notes. Departure from Miche (1944): the usual breaking limit is wave height / depth ≈ 0.78. Our cap is 0.75, which is stricter and exact in binary floating point at the depths the harness samples.

## Data

G-buffer channels this design adds, beside the roughness and metallic ADR 0012 already stores:

| Channel | Holds |
|---|---|
| IOR | transmitted index, default 1.5, water 1.333 |
| kind | opaque, glossy, planar mirror, portal, water |
| ids | mirror plane id or portal link id |

**Specular hit record.** World position reconstructed from depth, packed normal, source (screen, cache, trace, miss), age, variance. Eight bytes. Radiance is not in the record. Half resolution: 960×540 on 1080p (4.15 MB), 1280×720 on 1440p (7.37 MB).

**Hi-Z.** Min-depth pyramid at half resolution, R16. About 1.3 MB at 1080p and 2.5 MB at 1440p. Built only when the classify finds a glossy, mirror, portal, or water pixel.

**Mirror target.** One colour image and one depth, reused for every extra view. Potato has none. Low is quarter resolution. Medium and High are half resolution. A recursive view samples the previous view when the cap still has a slot. Otherwise it samples the world cache.

**Portal record.** Two scene handles, two planes, the rigid map. Author-placed. The map is

```text
p' = origin_b + basis_b * flip_z * basis_a⁻¹ * (p - origin_a)
```

`flip_z` reverses the source forward axis so the view looks out of the destination. The harness grades a quarter-turn about Y into a quarter-turn about X, origin B at `(10, 0, 0)`, forward flipped. The expected points are baked by hand. A map that applies `from_basis` instead of its inverse misses them. Gate: `portal_point_m` ≤ `1e-5`.

**Water.** A mesh plus a uniform block of Gerstner components (direction, amplitude, wavelength, speed, steepness). The count is 0 / 4 / 6 / 8 for Potato / Low / Medium / High. Time is the scene clock. A frozen clock does not move the surface. The mesh writes the G-buffer (kind = water) and claims surface-cache texels the way any mesh does, so the room can pick up the water's colour. That replaces the extras-plan rule that water does not write the G-buffer, once this ADR is accepted.

**Caustic cache.** A small world-space hash, sun only, same finished-value rule as the surface cache. Medium writes the image-space map into it (1 MB). High may spend reflection rays to confirm texels; those rays count against the reflection ray cap, not a second cap. Potato and Low leave it unallocated.

## Passes

Each pass is a compute or a raster with one trace site, the GI rule. A cheap check skips the pass when its input count is zero. The check always runs.

1. **Classify.** Count glossy, mirror, portal, and water pixels. All zero: stop. Ceiling 0.05 ms.
2. **Hi-Z.** Min-depth pyramid when the classify is non-zero.
3. **Mirror and portal views.** Near the camera first, up to the view cap. Each view is a G-buffer and the existing lighting into the reused target. The oblique near plane rejects geometry behind the plane (`portal_leak` = 0). A requested view past the cap is not drawn.
4. **Screen march.** One direction, half resolution, step cap 16 / 32 / 48 (Low / Medium / High). Thickness is one cache cell, 25 cm in world space, converted to view depth, so a hit behind a thin wall is rejected. McGuire and Mara (JCGT 2014) march until the ray leaves the frustum. We stop at the step cap and fall through to the cache.
5. **Cache fallback.** A cone read of finished cache entries along the miss direction. No new ray. This is the Low fallback and the colour of a view the cap refused.
6. **Reflection rays.** Medium up to 32 k, High up to 128 k, Potato and Low zero. Software SDF, or `VK_KHR_ray_query` when the device has it. The interface is the GI tracer. Misses stay on the cache value.
7. **Shade.** Read the cache at the hit. Metals tint by base colour. Dielectrics apply `F`.
8. **Spatial filter.** One à-trous iteration on a budgeted list (4 k / 8 k / 16 k items). Variance already under the threshold skips the item.
9. **Water.** Fresnel blend of the reflection and the refracted opaque colour, times `exp(-σ d)`. Foam coverage from the shoreline clamp and the Jacobian. Caustic add is a cache read.
10. **Compose.** The formula in [Fit](#fit). One blend.

Change records from ADR 0013 drop a hit whose surface moved. The next shade uses the new cache value. `denoise_lag_frames` stays `NOT-IMPLEMENTED` on every fixture until that reflection filter exists. Copying the new sample is not a passing case. The GI à-trous is a different filter and is not a stand-in.

## Mirrors

A planar mirror reflects the camera through its plane and draws one view. A portal applies the rigid map instead of the reflection. Both share `mirror_views`.

| Preset | Extra views |
|---|---|
| Potato | 0 |
| Low | 1 |
| Medium | 2 |
| High | 3 |

A further view is emitted when the running product of Fresnel reflectances stays at least 0.02 and the cap still has room. `min(request, cap)` is the broken scheduler: a water chain (F ≈ 0.020 then 0.060) emits one view, and clamping a request of 2 to the Medium cap emits two. Gate: `mirror_view_frac` ≤ 0, the max over Potato, Low, Medium, and High of `|emitted − fresnel_budget| / cap`. A cap of 0 passes only when that preset emits 0. The count is `optics::mirror_views`.

The oblique clip is Lengyel's construction with a Vulkan depth row. The third row is the scaled plane `c` (scale 1), so the plane sits on clip `z/w = 0`. Lengyel's listing uses scale 2 and `c − row3` because that lands on the OpenGL near plane `z/w = −1`. `sgn(0)` is +1. `portal_leak` is the fraction of six reflected-camera samples where `oblique_draws` disagrees with `plane_in_front`. Gate: `portal_leak` ≤ 0. Drawing every sample is the broken case.

## Water

The surface normal comes from the Gerstner sum. The glossy stack is the reflection once that pass exists. The CPU picture in this phase reflects a flat plane along the mirrored ray. Refraction samples the opaque colour along the Snell direction. A refracted miss keeps the opaque sample, darkened by Beer–Lambert over the water depth. It does not invent a tint, and it does not read the world cache. The cache waits with the screen march.

Shoreline depth is the opaque depth minus the water surface, in metres. Wave height is `min(deep_height, 0.75 × depth)`. Foam coverage is 1 where the clamp is active or the horizontal Jacobian drops below 0.5, and 0 elsewhere, with a smooth step between. The path tracer does not model foam yet ([reference check](lighting/reference-check.md) blind spots). Foam's picture grade stays `NOT-IMPLEMENTED` until the reference grows that term. The harness grades the clamp against an independent Gerstner sample at `(2, 0)`, time 0, amplitude 0.4, wavelength 8, steepness 0.5. The oracle writes the cap as the literal `0.75`. Depths are 0.0625, 0.125, 0.25, 0.5, 1, 2, and 4 metres. `shoreline_ratio` is `|engine − oracle|` of the max crest/depth, and `shoreline_deep_ratio` is the same error at 4 m. The gate is `1e-4`: larger than an f32 sine residual near this phase (the f32 sine of π/2 is 1, and an f32 ulp near 0.75 is about `6e-8`) and far below the unclamped gap (about 6) and the forced-cap gap (about 0.65). Skipping the clamp or forcing it in deep water fails.

Caustics:

| Mode | Presets | What it does |
|---|---|---|
| off | Potato, Low | no map, no rays |
| image | Medium, High default | one sun, image-space map (Shah et al.), written into the caustic cache |
| cache | High optional | reflection rays confirm cache texels, inside the ray cap |

Flat water under a vertical sun transmits `1 − F`. For IOR 1.333 that is 0.979626812158. That number is `flat_water_transmittance` in the shared module. `caustic_rel` does not call it. Good feeds a uniform field of the baked number and bad feeds a 0/1 checker, so the row only tests the comparator. Live leaves it `NOT-IMPLEMENTED`. Gate, once a caustic map exists: `caustic_rel` ≤ 0.05. Curved-surface caustics use the same name against the path tracer.

## Budgets

Ceilings on the named GPU at the named resolution. The millisecond cap is what a later controller holds by cutting work units, the same rule as ADR 0013. These numbers are the gates. They are not a measurement of a 4070; phase 0 of the feature records the real times. The harness's good fixture feeds the estimates in the VRAM notes (under the cap). The bad fixture feeds a High frame at 2.5 ms and 80 MB.

| | Potato | Low | Medium | High |
|---|---|---|---|---|
| GPU the cap is for | RTX 4070 | GTX 1060 6 GB, RX 580 | RTX 2060 class | RTX 4070 |
| Resolution | 1440p | 1080p | 1080p | 1440p |
| Reflection ms | 0.05 | 0.40 | 0.70 | 1.00 |
| Reflection VRAM (MB) | 0 | 8 | 16 | 32 |
| SSR steps | 0 | 16 | 32 | 48 |
| Reflection rays | 0 | 0 | 32 000 | 128 000 |
| Tracer | off | off | software, or ray query if present | ray query if present, else software |
| Extra mirror views | 0 | 1 | 2 | 3 |
| Denoise items × iterations | 0 | 4 000 × 1 | 8 000 × 1 | 16 000 × 1 |
| Caustics | off | off | image | image; cache optional inside the ray cap |
| Gerstner components | 0 | 4 | 6 | 8 |

High VRAM estimate, inside the 32 MB cap: hit records 7.37 MB, Hi-Z 2.5 MB, half-resolution mirror target (RGBA16F + D16) 9.2 MB, caustic cache 2 MB. Sum 21.1 MB. Low estimate: hits 4.15, Hi-Z 1.3, quarter-resolution target 0.8. Sum 6.2 MB. Medium estimate: hits 4.15, Hi-Z 1.3, half-resolution RGBA8 target 3.1, caustic image 1. Sum 9.5 MB.

The reflection caps sit inside the ~200 MB lighting soft budget. High GI in ADR 0013 is about 140 MB; 21 MB of reflections leaves the sum under 200. On a 4 GB card the Low reflection cost is 8 MB.

Frame targets:

- Basic scene, no glossy pixels, no mirror, no water: reflection time ≤ 0.05 ms. The 500 fps target is a 2 ms frame.
- Stress scene, High, 1440p: reflection time ≤ 1.00 ms, and lighting time plus reflection time ≤ 10 ms. Reflection work yields first when the sum would pass 10 ms. ADR 0013's 8 ms lighting cap and this 1 ms cap are both ceilings. They do not stack past the frame target.

Gates: `ms_frac` ≤ 1 and `vram_frac` ≤ 1, each the max of used/cap across the four presets, once a GPU auto-tune exists. Both rows are `NOT-IMPLEMENTED`. Lavapipe timestamps are not the 4070 or 1060 cap. The pass still reports `gpu_ns` and `vram_bytes` through `get_reflection_stats` for that later tune. `low_reflection_rays` ≤ 0 is the atomic the Low preset's frame shader increments.

## Metrics

Every gate is `value ≤ threshold`.

Live and good read the presented frame. The picture is the grade window: a mirror, a portal whose bases are a quarter-turn about Y into a quarter-turn about X, and water from the incident side and from the transmitted side. `reflections-check` opens that window, reads the swapchain, and compares it with `crates/lighting-ref/src/reflections_trace.rs`. The tracer is f64. It does not call `genos_math::optics`, it does not read that module's constants, and it does not call the renderer. The scene literals are copied into the tracer.

A channel that matches the quantised trace contributes 0. A one-code miss contributes 0 only when the f64 colour sits within 0.02 of a rounding boundary, which is the f32 store of that shade. Any larger miss is the row's value. The source label is `presented frame`. Nothing is labeled `GPU frame readback`.

- `mirror_reflect` is the red reflection patch, x 0..32, y 20..44.
- `fresnel_abs` is the incident-side water at x 8..80, y 0..16, clear of the GGX swatch.
- `beer_rel` is x 40..80, y 20..44.
- `water_f0` is the flat patch x 2..14, y 48..58. `snell_transmit` is x 40..72, y 48..58.
- `fresnel_below_abs` is the transmitted-side water, past the critical angle.
- `mirror_point_m` is the mirror window. `portal_point_m` is the portal window. `portal_leak` is the fraction of mirror pixels that show the plane behind the mirror.
- `mirror_view_frac` is the fraction of the third window that is not the surface albedo. Medium's cap is 2, so that window is refused.
- `ggx_energy` is `|presented[2,2].r − 1|` on the projected-NDF swatch.
- `near_weighted_rel` is the depth-weighted relative error of the whole frame. Weight is `1 / max(depth, 0.25)` on the tracer depth. Per pixel the error is the mean of `|a − b| / max(|b|, 1e-3)` over the three channels.
- `flicker_reversal` is the max mean-channel change across three still Medium frames.
- `low_reflection_rays` is the atomic the frame shader increments on Low. The source label is `GPU dispatch count`. The shader calls `reflection_ray_cap`. A one-group launch still runs when the cap is 0, and threads past the cap do not increment.
- `shoreline_ratio` and `shoreline_deep_ratio` are `|engine − oracle|` against the wave sample above. The source label is `wave oracle`. The gate is `1e-4`.

`ssr_depth_rel`, `denoise_lag_frames`, `caustic_rel`, `ms_frac`, and `vram_frac` are `NOT-IMPLEMENTED` on live, good, and bad. The value is `na`. Lavapipe time is not `ms_frac`. `get_reflection_stats` reports `gpu_ns` and `vram_bytes` without turning them into those fractions.

`denoise_lag_frames` is `NOT-IMPLEMENTED` on live, good, and bad. There is no reflection denoiser to run, and copying the sample is not a result.

| Metric | Threshold | What it measures |
|---|---|---|
| `mirror_reflect` | 1e-4 | max channel error of the reflected-colour patch |
| `mirror_point_m` | 1e-5 | error of a point reflected through a plane, metres |
| `fresnel_abs` | 0.02 | max absolute error against the Fresnel equations, from air |
| `fresnel_below_abs` | 0.02 | the same error from under water, including total internal reflection |
| `water_f0` | 0.005 | `|mean channel − 0.0204|` on the flat normal-incidence patch. Glass F0 0.04 fails |
| `snell_transmit` | 1e-4 | max channel error of the transmission patch |
| `ggx_energy` | 0.03 | absolute error of the GGX projected integral against 1 |
| `ssr_depth_rel` | 0.02 | relative depth error of a screen march on a synthetic buffer |
| `denoise_lag_frames` | 2 | frames after a step until the shaded value reaches 90 % |
| `mirror_view_frac` | 0 | max `|emitted − fresnel_budget| / cap` across Potato, Low, Medium, High |
| `portal_leak` | 0 | fraction of plane samples drawn on the wrong side |
| `portal_point_m` | 1e-5 | error of the portal rigid map, metres |
| `beer_rel` | 0.02 | relative error of transmittance against `exp(-σ d)` |
| `shoreline_ratio` | 1e-4 | `|engine − oracle|` of max crest / depth. The oracle's cap literal is 0.75 |
| `shoreline_deep_ratio` | 1e-4 | `|engine − oracle|` of the same crest at 4 m |
| `caustic_rel` | 0.05 | mean relative error against flat-water transmittance |
| `near_weighted_rel` | 0.03 | depth-weighted relative error, weight `1 / max(depth, 0.25)` |
| `flicker_reversal` | 0.002 | mean size of a frame-to-frame reversal on a still pixel |
| `low_reflection_rays` | 0 | reflection rays the Low preset schedules |
| `ms_frac` | 1 | max of used milliseconds / cap |
| `vram_frac` | 1 | max of used megabytes / cap |

`near_weighted_rel` is the near-camera rule in one number. A large error at 20 m and a zero error at 0.5 m passes. The same error at 0.5 m fails. `flicker_reversal` is the still-camera flicker measure from the lighting reference check, on one pixel. An alternating 0.5 / 0.3 fails. A constant passes.

Picture grades, once the feature exists, fill these same names from the path tracer. They do not get a second, looser threshold. The blurred relative luminance in [the reference check](lighting/reference-check.md) stays the GI gate. Specular views add `near_weighted_rel` and `flicker_reversal`. A caustic texture that is not the flux fails `caustic_rel`. There is no visual override and no per-scene nudge.

The harness:

```sh
cargo run -p genos-lighting-ref --bin reflections-check -- --case live
cargo run -p genos-lighting-ref --bin reflections-check -- --fixture good --out genos-debug/reflections/good
cargo run -p genos-lighting-ref --bin reflections-check -- --fixture bad --out genos-debug/reflections/bad
```

`--case` is an alias of `--fixture`. The later flag wins when both are set. Stdout is one line per metric: `name value=… threshold=… PASS|FAIL|NOT-IMPLEMENTED`. The JSON is `<out>/metrics.json`. The default directory is `./genos-debug/reflections`. `--out` wins. `GENOS_REFLECTIONS_OUT` applies only when `--out` was omitted. Exit 0 when no metric failed. Exit 1 when any metric failed. Exit 2 on a bad argument. `NOT-IMPLEMENTED` is not a failure.

`cargo test -p genos-lighting-ref reflections` runs the good, bad, and live fixtures and the `--out` precedence. Good and live open a proof window on lavapipe. The numeric picture gate is that readback against the f64 trace. Timing on lavapipe is not the 4070 or 1060 budget.

Rhai gates land with `debug/reflections`, on the same line format, under `debug/reflections/scripts/gates/`:

| Script | Scene | Gate |
|---|---|---|
| `idle.rhai` | basic room, no glossy pixels | reflection ms ≤ 0.05, rays = 0 |
| `glossy.rhai` | a metal sphere and a dielectric sphere, still camera | `near_weighted_rel`, `flicker_reversal`, `fresnel_abs` |
| `mirror.rhai` | a planar mirror of a lit room | `mirror_reflect` against a second path-traced camera, `mirror_view_frac` |
| `portal.rhai` | two linked openings, a graph that wants more views than the cap | `portal_leak`, `portal_point_m`, `mirror_view_frac` |
| `water.rhai` | a pool, a shore, a vertical sun | `snell_transmit`, `beer_rel`, `shoreline_ratio`, `caustic_rel` |
| `low.rhai` | the pool on the Low preset, software tracer forced | `low_reflection_rays` = 0 |

The presented frame grades the mirror, the portal, Fresnel, F0, Snell, Beer, GGX, flicker, the view cap, and the near-weighted error. The shoreline rows are the wave oracle. `low_reflection_rays` is the GPU dispatch count. The Rhai scenes and `debug/reflections` still wait. Glossy screen marches, the world-cache read, the `ray_query` fallback, and the reflection denoiser wait on lighting phase 3 (`cursor/lighting-p3-capped-refresh`). That next phase does not edit `lighting_schedule.rs` and does not edit the GI passes. `ssr_depth_rel` stays `NOT-IMPLEMENTED` until the march exists. Foam and `caustic_rel` stay `NOT-IMPLEMENTED` until P9.

## Debug and tests

`debug/reflections` is the binary, on the scaffolding in [the debug standard](lighting/debug-standard.md): scene picker, checkpoints, reset, replay, screenshots, metrics, sliders, a guided-review panel, and a headless CLI. It is not in this change. This change's tool is `reflections-check`.

The debug menu edits `ReflectionSettings` through `set_key`. Rows are the keys. There is no help text. Sliders: `rough_cutoff`, `mirror_cutoff`, `mirror_views`, `rays`, `shore_cap`. Dropdowns: `caustics` (`off`, `image`, `cache`), `tracer` (`off`, `software`, `ray_query`). Checkboxes: `enabled`, `water`.

Arguments, each with an environment mirror that applies only when the argument was omitted:

| Argument | Mirror | Meaning |
|---|---|---|
| `--reflections on\|off` | `GENOS_REFLECTIONS` | master switch |
| `--reflection-rays N` | `GENOS_REFLECTION_RAYS` | ray cap for this run |
| `--mirror-views N` | `GENOS_MIRROR_VIEWS` | extra-view cap |
| `--water on\|off` | `GENOS_WATER` | water pass |
| `--caustics off\|image\|cache` | `GENOS_CAUSTICS` | caustic mode |
| `--reflection-tracer off\|software\|ray_query` | `GENOS_REFLECTION_TRACER` | tracer |
| `--out DIR` | `GENOS_REFLECTIONS_OUT` | report directory |

`reflections-check` reads `GENOS_REFLECTIONS_OUT`. The picture settings read the other six, and the command line wins when the argument is present.

`ReflectionSettings` implements `EngineSettings`, lives on the scene, and is on the MCP catalogue (`get_reflection_settings`, `set_reflection_settings`). The menu, the CLI, and MCP write that struct.

Overlays, drawn by the debug binary, with no legend paragraph: source colour, recursion index, water depth, caustic energy, hit age, variance. Counters: rays used / cap, views used / cap, march misses, cache fallbacks, denoise items, reflection milliseconds, reflection megabytes.

| Failure | Probe |
|---|---|
| Reflection direction wrong | `mirror_reflect`, source overlay |
| Fresnel bias | `fresnel_abs`, `fresnel_below_abs` |
| Refraction bend | `snell_transmit` |
| Lobe energy | `ggx_energy` |
| Screen march accepts the wrong depth | `ssr_depth_rel`, hit age |
| Denoise lags a step | `denoise_lag_frames` |
| Too many mirror views | `mirror_view_frac`, recursion overlay |
| Portal draws the back side | `portal_leak` |
| Portal map jumps | `portal_point_m` |
| Water absorption | `beer_rel` |
| Waves at the shore | `shoreline_ratio`, water-depth overlay |
| Caustic does not match the flux | `caustic_rel`, caustic overlay |
| A far error hides a near miss | `near_weighted_rel` |
| Flicker on a still pixel | `flicker_reversal` |
| Low traces a ray | `low_reflection_rays` |
| Frame over the cap | `ms_frac` |
| VRAM over the cap | `vram_frac` |

Snapshot and restore: the hit records, the view cap's emitted ids, and the caustic cache generation. A failed frame restores that snapshot and replays the same clock.

## Phases

Each phase is one change. A phase that misses its gate is dropped: a five-line postmortem, then the branch goes.

| Phase | Change | Gate |
|---|---|---|
| P0 | This harness and `genos-math` optics. No picture change. | good exits 0, bad exits 1, live passes the module rows and leaves the comparator rows `NOT-IMPLEMENTED` |
| P1 | `ReflectionSettings`, classify, counters. Picture unchanged. | idle ms ≤ 0.05, rays = 0 |
| P2 | Screen march and cache fallback. No rays. | `ssr_depth_rel`, `mirror_reflect`, `fresnel_abs` on the spheres |
| P3 | One spatial filter. | `denoise_lag_frames` ≤ 2, `flicker_reversal` ≤ 0.002 |
| P4 | Optional rays. Low stays at 0. | `low_reflection_rays`, `near_weighted_rel` ≤ 0.03, `ms_frac` |
| P5 | Planar mirrors. | `mirror_view_frac`, `portal_leak`, `mirror_point_m` |
| P6 | Portals on the same cap. | `portal_point_m`, `portal_leak` |
| P7 | Water shade, Snell, Beer–Lambert. | `snell_transmit`, `beer_rel` |
| P8 | Shoreline clamp and Jacobian foam. | `shoreline_ratio` ≤ 1e-4, `shoreline_deep_ratio` ≤ 1e-4 against the wave oracle |
| P9 | Caustic modes. | `caustic_rel` ≤ 0.05, `vram_frac` ≤ 1 |

P4 is where half-resolution tracing is confirmed or the near band moves to full resolution inside 32 MB. P9 does not add a ray cap of its own.

The CPU picture landed the parts that do not need that lighting phase: planar mirrors and portals (P5, P6) with the Fresnel view cap, water shading and refraction (P7) on a flat plane, the shoreline clamp (P8; Jacobian foam still waits), and the Low ray count from P4. The frame runs those passes after lighting when the world lists a mirror, a portal, or water. Potato and Low store a ray count of 0 and do not march rays. Mirror and portal views are planar cameras with the oblique near clip, capped by `reflection_view_cap`. Water quads are the translucent pass and call `reflection_water_shade`. A scene with none of those surfaces skips the passes, so today's Lambert picture stays valid. `ReflectionSettings` is on the scene, the debug menu, and MCP. Classify counters and the idle-millisecond probe wait on `debug/reflections`.

Next, after lighting phase 3: the screen march, the world-cache or `ray_query` fallback, and one spatial reflection filter. Those three stay out of `lighting_schedule.rs` and out of the GI passes. They turn `ssr_depth_rel` and `denoise_lag_frames` live under the same thresholds.

## Networking

- Replicated state: roughness, metallic, IOR, surface kind, portal link ids, Gerstner amplitudes, and the water clock. Hit records, Hi-Z, mirror targets, and the caustic cache are derived and are not replicated.
- Authority: the server owns material factors, portal links, and the water clock. A client does not spawn a portal.
- Prediction / rollback: the picture is recomputed from the camera and is not rolled back. Walking through a portal waits on the networking plan. A client predicts the Gerstner displacement from the server clock.
- Determinism: Fresnel, Snell, the mirror matrix, the portal matrix, and the Gerstner sum are pure functions of their inputs. Specular directions are fixed by the normal and the view. The software tracer and ray queries may disagree at a silhouette; both are graded at the same threshold.
- Headless server: the server does not draw mirrors, water, or reflections. It evaluates the portal matrix and the Gerstner height on the CPU when gameplay needs the surface. The reflection passes are client-only.

## Open questions

1. ADR 0013's game-side importance hint is not in this design. A mirror view is ordered by distance to the camera. Whether that hint may reserve a view waits until the hint exists.
2. Metals use Schlick because glTF stores F0 as the base colour. A metal reference scene with a measured complex IOR would replace that term. The dielectric term stays exact.
3. Gerstner counts (4 / 6 / 8) are the starting point. Phase 7 measures them against the 0.40 ms Low cap and drops components until the cap holds.
4. High's optional caustic confirmation spends the reflection ray cap. It does not get a second cap. Phase 9 checks that a sun-plus-water stress frame still holds `ms_frac` ≤ 1.

## Departures

Collected from the sections above, for the pull request that implements each one.

| Source | What we take | Departure |
|---|---|---|
| Lumen, Wright et al., SIGGRAPH 2022, and the Lumen technical details | Screen trace, then a ray, shaded from the surface cache. Rough reflections reuse the diffuse gather. | No temporal reflection reuse. No hit lighting. Low traces no reflection ray. The march stops at a step cap. |
| Stachowiak, SIGGRAPH 2015 | Screen-space glossy reflections | One direction per pixel, fixed by the normal and the view. |
| Schied et al., HPG 2017 (SVGF) | The spatial à-trous kernel | The temporal accumulation is not used. One iteration, on a budgeted list. |
| Schied et al., HPG 2018 (A-SVGF) | A fast response to a change | Change records drop the hit. There is no gradient-driven history. |
| Schlick 1994, Karis, SIGGRAPH 2013 | Schlick Fresnel | Dielectrics use the exact equations. Schlick stays on metals. |
| Walter et al., EGSR 2007 | GGX NDF, `α = roughness²` | Transmission is one Snell direction, not a random BTDF sample. |
| McGuire and Mara, JCGT 2014 | Hi-Z screen march | Step cap. Thickness is 25 cm in world space. A miss reads the cache. |
| Diefenbach 1996 | Reflected camera for a planar mirror | View cap. A refused view reads the cache. |
| Lengyel, JGD 1(2), 2005 | Oblique near plane on the mirror or portal | Vulkan clip Z is 0..1, so the third row is `c` and the scale is 1. The listing subtracts the fourth row and uses 2, for OpenGL `z/w = −1`. `sgn(0)` is +1. |
| Diefenbach 1996, this picture | Reflected camera, graded by a Whitted tracer from the hit | The shaded water hit is a flat plane. Gerstner feeds the shore clamp, not that hit. A refused view and a refraction miss keep the opaque sample. The world cache is not read. |
| Luebke and Georges, I3D 1995 | A portal opening | We fill the opening. We do not walk the scene as a PVS. |
| Finch, GPU Gems ch. 1, 2004 | Gerstner sum | Component count is a cap, not a fixed ocean. |
| Tessendorf, SIGGRAPH 2001 | Jacobian foam | No FFT. |
| Miche 1944 | Wave height limited by depth | Cap is 0.75, stricter than 0.78. |
| Shah, Konttinen, and Pattanaik, TVCG 2007 | Image-space caustic map | The map is written into the world cache. |
| S79 (half-resolution à-trous) | The GI picture stays full resolution | The reflection trace buffer is half resolution. The GI à-trous is unchanged. |
