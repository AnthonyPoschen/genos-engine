# Fog and volumetric lighting

Status: **accepted**. ADR: [0018](../adr/0018-fog-and-volumetric-lighting.md), status Accepted. Anthony delegated design approval to the Engine bot, which accepted it. Terms follow [`docs/lighting/GLOSSARY.md`](../lighting/GLOSSARY.md). The metric harness is `fog-check` (`crates/fog-check`). It grades the analytic fixtures and each renderer row as that row's pass lands. `gi_inscatter_blur_rel` stays `NOT-IMPLEMENTED` until lighting phase 3 (`cursor/lighting-p3-capped-refresh`).

The realtime volume is a froxel grid. Height fog, local fog volumes, and shadowed shafts share that grid. Indirect in-scattering is a read of the world cache. The path tracer is extended so the picture can be graded. Nothing here is a screen-space blur.

## 1. Goals

1. A participating medium in the picture: height fog, bounded local volumes, and shadowed light shafts from the sun and from point lamps.
2. The grade is the path-traced reference once that tracer carries the medium. Until then the grade is the closed form (Beer-Lambert, exponential height density, slanted height-fog optical depth, Henyey-Greenstein and its sphere integral, the homogeneous integral) and the lighting-ref blurred relative luminance on a synthetic picture.
3. No flicker on a still camera. No smear and no pop on a yaw. Near-camera error is the tight gate.
4. The basic scene with density zero stays above 500 fps at 1440p on the RTX 4070 (frame under 2 ms). The stress scene at High stays near 100 fps (frame near 10 ms). Fog's own ceilings sit inside that frame.
5. Low runs on a GTX 1060 6 GB and an RX 580. Minimum VRAM is 4 GB. Ray queries are optional. macOS uses the software tracer.
6. Every failure mode and every tunable has a probe. The debug menu has no help text. Tools take arguments first.

## 2. Non-goals

- Multiple scattering. v1 is single scatter. Hillaire's isotropic multi-scatter lobe waits until the single-scatter grade is clean.
- Fog that writes the surface light cache or the world cache. The volume reads. It does not relight surfaces.
- Heterogeneous meshes as density (pyroclastic volumes, voxelized smoke assets). Local volumes are analytic boxes and spheres.
- Water, clouds, and the sky LUT. The sky miss stays Hillaire 2020, in the extras plan. It is not this volume.
- A radial-blur shaft, a billboard stack, or any other picture that is not the integral.
- Replacing GI. [ADR 0012](../adr/0012-gi.md) stays. Radiance cascades stay removed.

## 3. Where it sits

### Inventory

| Piece | What it does today | What this design does with it |
|---|---|---|
| `fog_along`, `height_fog_along`, `compose_fog` (`crates/render/src/particles.rs`) | Puff optical depth through at most eight spheres. Transmittance and the homogeneous in-scatter integral call `genos_math` (`crates/math/src/media.rs`). `height_fog_along` calls the slanted height-fog optical depth. `media.glsl` is the GLSL mirror. Lamps use `1 / (1 + dist² * 0.08)`. No shadows. | The composite equation stays. When the froxel is on, puff density is injected as local spheres and the puff composite is skipped, so a puff is not counted twice. Potato keeps `fog_along`. |
| Premultiplied fog blend (`crates/render/src/gpu.rs`) | `rgb = in-scattering`, `alpha = 1 - transmittance`, source factor one, destination `one_minus_src_alpha`. Result is `in-scattering + destination * transmittance`. | The upsample writes that target. The blend stays. |
| World cache and world radiance cache | Camera-independent surface light. Budgeted refresh ([ADR 0013](../adr/0013-lighting-invalidation-is-local.md)). | A refreshed froxel reads them for indirect incident light. No write. |
| Shadow cell cache, tracer | 12.5 cm shadow reuse. Software SDF or `VK_KHR_ray_query`. macOS is the software tracer ([ADR 0012](../adr/0012-gi.md)). | Shaft shadows use that tracer. Low and Medium may reuse the shadow cell. |
| Change records, priority, work caps | Per-preset caps. Near-camera error ranks first. No full-frame fallback. | In-scatter samples use the same scheduler. Extinction is not budgeted: it is analytic. |
| Path tracer (`crates/debug/src/reference.rs`) | Surfaces only. The file says fog is not traced. | Phase 7 adds the medium. The checker stays `blur_relative` in `genos-lighting-ref` (fail line 0.12). |
| Particle cards | A card with density is one fog card (`docs/systems/renderer.md`). | Cards stay for the flame. They are not the world volume. |

GI v2's non-goal line stands: the GI passes do not grow a volume. This design is the reader those passes left room for.

Presets are Potato, Low, Medium, and High, the same four as lighting. Potato is direct light and flat ambient. Its fog is off. Day-cycle and gi-1bounce do not gain a volume in this design.

### Fit to the frame

Order, after lighting compose and before the tone curve:

1. If every density is zero, stop. No dispatch.
2. Inject extinction for the whole grid (height fog closed form, then local volumes).
3. Budgeted in-scatter: sun, then point lamps, then the world-cache read.
4. Fog history stays off. The reserved slot is not blended.
5. Integrate each column with Beer-Lambert.
6. Depth-aware upsample into the fog target and the existing blend.

A steady scene with density zero pays the check and approaches the non-fog frame time. A steady scene with fog pays the extinction inject, the starvation slice, the column integral, and the upsample. It does not retrace every froxel.

The basic-scene 500 fps gate is the room with density zero. A room that opts into height fog is a different scene and must still hold the High fog ceiling (1.20 ms). It is not claimed as 500 fps.

## 4. Alternatives

Three current methods. The pick is the first.

### Froxel grid (picked)

Sébastien Hillaire, "Physically Based and Unified Volumetric Rendering in Frostbite", SIGGRAPH 2015 Advances in Real-Time Rendering. <https://www.ea.com/frostbite/news/physically-based-unified-volumetric-rendering-in-frostbite>

Bartłomiej Wronski, "Volumetric Fog: Unified Compute Shader Based Solution to Atmospheric Scattering", SIGGRAPH 2014 Advances in Real-Time Rendering. <https://bartwronski.com/2014/08/13/volumetric-fog-siggraph-2014/>

A froxel is one cell of a grid aligned to the camera frustum. Slices are exponential in depth, so cells near the camera are thin. Each cell stores extinction and in-scattering. The view ray is integrated with the Beer-Lambert law. Local density is injected into the cells. Lights are sampled at cell centers. Those engines keep a short history to cut the noise. This design leaves that history off (§7). Frostbite named the cell (frustum voxel). Wronski shipped the same family a year earlier. Unreal Engine 5 volumetric fog is this family again, not a third algorithm (Epic, "Volumetric Fog in Unreal Engine", <https://dev.epicgames.com/documentation/en-us/unreal-engine/volumetric-fog-in-unreal-engine>).

Why this one: one grid serves height fog, local volumes, sun shafts, and point lamps. Cost follows the grid and the sample cap, not the pixel count, which is what a ceiling needs. The world-cache read is one extra term on a cell that already exists. The grid stays under the 2048 3D-texture edge (High is 160×90×48).

### Per-pixel ray march (rejected)

Balázs Tóth and Tamás Umenhoffer, "Real-time Volumetric Lighting in Participating Media", Eurographics 2009.

March the medium in the picture, one ray per pixel, sampling shadow maps along the ray. Quality can sit close to a path trace. Cost is pixels times steps. At 1440p and 48 steps that is about 1.8×10⁸ samples before the shadow ray. That does not fit a 1.20 ms High ceiling, and it does not fit a basic scene that must stay under 2 ms when fog is the only extra. The path tracer in phase 7 is this quality bar offline. It is not the frame path.

### Epipolar shafts (rejected)

Thomas Engelhardt and Carsten Dachsbacher, "Epipolar Sampling for Shadows and Crepuscular Rays in Participating Media with Single Scattering", I3D 2010. <https://doi.org/10.1145/1730804.1730823>

Samples are placed along epipolar lines of one directional light, then scattered to the picture. Single-scatter shafts are cheap and sharp. The method does not build a volume, so a local fog box, a point lamp, a height field, and a world-cache read do not share work. Shafts would be a second effect beside fog. We want one integral.

Screen-space radial blur is not in this list. It does not solve the integral and it is not graded.

## 5. Papers we follow, and the departures

The structure we follow is Hillaire 2015: extinction and scattering in a frustum grid, exponential slices, Henyey-Greenstein, Beer-Lambert along the view, local media injected into cells, temporal reuse of the scattering. Wronski 2014 is the second citation for a shipped grid below picture resolution. Where the two differ, Hillaire is the source.

| Source | What we take | Departure |
|---|---|---|
| Hillaire 2015 | Froxel grid, exponential slices, extinction and in-scattering, local injection, Beer-Lambert, Henyey-Greenstein | Listed below. Each one is a departure from that course. |
| Wronski 2014 | The volume is coarser than the picture, then applied up | We upsample a column integral. We do not march per pixel at picture size. |
| Engelhardt and Dachsbacher 2010 | Nothing from the algorithm | Rejected. Cited so the rejection is explicit. |
| Tóth and Umenhoffer 2009 | Nothing from the frame path | The offline tracer marches. The frame path does not. |
| Karis, "High Quality Temporal Supersampling", SIGGRAPH 2014 | Neighborhood clamp on history | Not applied. Fog history is off on every preset. Cited because enabling history is open, and a clamp would have to be graded then. |
| Novák, Georgiev, Hanika, Křivánek, and Jarosz, "Monte Carlo Methods for Physically Based Volume Rendering", SIGGRAPH 2018 course. <https://doi.org/10.1145/3214834.3214843> | Delta tracking in the reference tracer | The frame path does not path-trace the medium. |
| Kulla and Fajardo, "Importance Sampling Techniques for Path Tracing in Participating Media", EGSR 2012 | Equiangular sampling of point lamps in the reference | The frame path samples lamps at cell centers. It does not use equiangular samples. |
| Henyey and Greenstein, Astrophysical Journal 93, 1941 | The phase function | g is a setting. The integral of the function is 1. |
| [ADR 0013](../adr/0013-lighting-invalidation-is-local.md) | Work caps, priority, change records, shadow-cell reuse on Low and Medium, near-camera first | Hillaire refreshes the whole grid every frame. We do not. |

Departures from Hillaire 2015, by name:

1. **History stays off.** Hillaire blends a reprojected scattering sample every frame. [ADR 0012](../adr/0012-gi.md) stores no radiance history, so Potato, Low, Medium, and High leave fog history off. Enabling it is an open decision for Anthony. The evidence is `froxel_temporal_smear_px` and `fog_yaw_pop_rel`. This design does not choose that outcome and does not adopt a reject rule.
2. **No full-grid light refresh.** Extinction is filled every enabled frame. In-scatter samples stop at the cap. Hillaire updates every cell's lighting every frame. There is no full-volume fallback.
3. **Unshadowed closed form on an unsampled cell.** Hillaire would keep a reprojected sample. There is no history to keep. The stand-in is the analytic unshadowed integral of the height fog, plus local extinction with no lamp. It is finished, it is not black, and it is biased bright inside a shaft until a sample arrives. Near slices are first in the cap, so that bias is not the near-camera error.
4. **Column integral and a depth-aware upsample,** not a per-pixel march through the 3D texture. A march at 1440p does not fit the ceiling.
5. **Traced shadows,** not shadow maps. One shadow ray, or a shadow-cell read, per refreshed cell per selected lamp. The sun is always selected. Point lamps use the existing lamp weights.
6. **Height fog uses the closed form per slice** (`σ(y) = σ0 exp(-(y - y0) / H)`, and `σ0 * H * exp(-y0 / H) * (1 - exp(-dy / H)) / cos θ` along a slanted ray). Hillaire samples density. Those functions live in `genos_math::media`. `fog-check` grades that module.
7. **Single scatter only.** The multi-scatter lobe in the Frostbite course is out of v1.
8. **World-cache in-scattering.** The 2015 course has no world-space GI cache. A refreshed cell adds `σ_s * p(ω) * L_cache`. That read is the GI feed. It is not in the paper.
9. **Grid sizes are preset caps,** not a fixed 160×90×64. High is 160×90×48 so the allocation stays under 12 MB. Slice counts: Low 24, Medium 32, High 48. Near 0.10 m, far 64 m, exponential.

`RGBA16F` is the texture format. It does not require `shaderFloat16`. Shaders do not assume a subgroup size.

## 6. Data and passes

### Grid

Exponential slice edges, for slice index `i` in `0 .. n` and `n` slices:

```text
z(i) = z_near * (z_far / z_near) ^ (i / n)
```

`z_near = 0.10 m`, `z_far = 64 m`. Cell `i` covers `[z(i), z(i+1)]`. High uses 48 slices, so the first slices are centimetres and the last are metres. The tail past 64 m is the closed-form height-fog integral along the remaining ray. Local volumes past the last slice are ignored.

Stored per cell, 16 bytes:

| bytes | contents |
|---|---|
| 8 | RGBA16F: in-scattering rgb, extinction in a |
| 8 | RGBA16F history: previous in-scattering rgb, accept weight in a |

The column target is RGBA16F, 8 bytes per column: integrated in-scattering rgb and transmittance in a. Potato allocates neither.

### Extinction inject (every enabled frame)

Height fog at a sample height `y`:

```text
σ_t(y) = density * exp(-(y - height) / thickness)
```

`density`, `height`, and `thickness` are settings. The integral of `σ_t` across a slice has a closed form because the ray's `y` is linear in distance and `σ_t` is exponential in `y`. Absorption and scattering share `σ_t`. Single-scatter albedo is the `albedo` setting, so `σ_s = albedo * σ_t` and `σ_a = σ_t - σ_s`. Albedo is clamped to `[0, 1]`.

A local fog volume is a box or a sphere in world space, with its own density, albedo, and falloff in metres. A cell whose center is inside contributes `density * falloff(distance to the bound)`. Volumes add. They do not replace the height field.

Puff spheres from the particle step are local spheres of radius `SMOKE_RADIUS` (0.42 m) and density `SMOKE_DENSITY` (1.7) when the froxel is on. The puff composite is then skipped.

This pass has no rays. It is inside the millisecond ceiling. It is not inside the sample cap.

### In-scatter (budgeted)

For a refreshed cell at world position `p`, the source added to the ray is:

```text
S = σ_s * p_hg(g, cos θ) * L_i * V
```

`p_hg` is Henyey-Greenstein:

```text
p_hg(g, μ) = (1 - g²) / (4π (1 + g² - 2 g μ) ^ 1.5)
```

`μ` is the cosine between the view direction and the direction toward the lamp. `g` is `phase_g` (default 0.4). `V` is 1 or 0 from a shadow ray, or the shadow-cell value. `L_i` for a point lamp is the lamp colour times the existing lamp falloff evaluated at `p`. The sun is the directional colour, unattenuated by distance. The world-cache term uses the cache radiance in place of `L_i * V` and the same phase, and only when `fog.gi` is on (Medium and High).

The sun takes the first sample on every refreshed cell. Point lamps take what the cap has left, strongest first, using the lamp-pick weights. The sun is not lamp-picked. Low refreshes the sun only. Medium refreshes up to four point lamps. High refreshes up to eight.

Sample caps per frame: Potato 0, Low 4 000, Medium 12 000, High 32 000. Ten percent of each cap is the starvation slice, oldest cells first, matching [ADR 0013](../adr/0013-lighting-invalidation-is-local.md). The rest is priority: change-record magnitude, near-camera slice (the nearest quarter of the slices), age, then a newly visible column. A histogram of 64 bins picks the threshold. No sort.

Low and Medium may take `V` from the shadow cell cache. High traces a fresh ray for a near-slice cell and may reuse the cache farther out. The ray goes through the software tracer or through `VK_KHR_ray_query` when that extension is present. macOS uses the software tracer. A slower GPU keeps the same millisecond ceiling: the controller lowers the sample cap, sun last. Nothing requires a minimum ray count.

### Column integral

One thread per column. Across slices, with `T` starting at 1:

```text
T_next = T * exp(-σ_t * Δs)
L     += T * (S / σ_t) * (1 - exp(-σ_t * Δs))    when σ_t > 0
```

`S` is the cell's stored in-scattering source (already including phase and shadow). This is the piecewise exact form of

```text
L(D) = L0 * exp(-τ) + ∫ S(s) exp(-τ(s)) ds
```

For constant `σ_t` and `S` it matches `(S / σ_t) * (1 - exp(-σ_t D))`. That match is the `homogeneous_inscatter_max_abs` metric. The tail past `z_far` adds the closed-form height term and no local volume.

The closed forms are `transmittance`, `height_fog_density`, `height_optical_depth`, `phase_hg`, and `homogeneous_inscatter` in `crates/math/src/media.rs`. `fog_along` and `height_fog_along` call the `f32` wrappers. `crates/render/shaders/media.glsl` is the GLSL mirror, compiled by `media_check.comp`. `fog-check` calls the `f64` functions on the good fixture and compares them to an oracle that does not call the module. The broken fixture is a wrong formula in the harness.

### Upsample

The column target is 160×90 on High, not 2560×1440. The composite samples it with a depth-aware weight: neighbors whose picture depth falls in a different slice get weight zero. The fog target is full resolution. The existing blend applies. Edges stay on the depth discontinuity instead of smearing across it.

## 7. Fog history

Fog history is off on Potato, Low, Medium, and High. [ADR 0012](../adr/0012-gi.md) stores no radiance history. The grid layout reserves a history slot so a later decision can turn the setting on without a second allocation. While the setting is off the pass does not read that slot and does not blend a previous frame. An unsampled cell uses the unshadowed closed form (departure 3).

Enabling fog history is an open decision for Anthony. This document does not choose it. The evidence is `froxel_temporal_smear_px` (centroid lag of a static shaft during a 90° yaw, threshold 0.50 px at 1440p) and `fog_yaw_pop_rel` (frame-to-frame relative luminance of the fog term on that yaw, threshold 0.05).

Extinction is recomputed every frame.

## 8. Light shafts and the world cache

A light shaft is the picture of `V` changing across cells that share a lamp. It is not a sprite and not a radial blur. The sun extrudes change records the way [ADR 0013](../adr/0013-lighting-invalidation-is-local.md) extrudes a mover for surface shadows: cells in the swept hull get priority, they do not get a second uncapped pass.

Point lamps are the same, with the hull clipped to the lamp's reach. A lamp that moves, switches, or dims is a change record for surface light. Fog history is off, so that edit does not blend a previous fog sample.

The world-cache term is indirect incident light at `p`. The cache stays camera-independent. The froxel is view-dependent and is not a cache of surface light. Medium reads the coarse world level. High reads the level the surface point would use. Potato and Low do not read it (`fog.gi` off). A cache miss contributes zero indirect light, not a second ambient. The direct sun term still stands.

Fog does not darken the cache. A surface under a thick fog is still lit as GI lights it, and the volume then composites in front. That is a departure from full transport and it is a non-goal of v1, stated here so the grade does not treat it as a bug. The path tracer's surface vertices also ignore the medium until a later decision says otherwise. Phase 7's reference matches v1: the medium is on the camera ray only.

## 9. Path tracer

`crates/debug/src/reference.rs` gains a medium that uses the same settings as the froxel: height fog, local boxes and spheres, albedo, `phase_g`. It does not read the world cache, the froxel, or `fog_along`.

- Homogeneous and exponential height segments use the closed-form transmittance.
- Overlapping local volumes use delta tracking as stated by Novák et al., SIGGRAPH 2018.
- A point lamp on a volume vertex uses equiangular sampling (Kulla and Fajardo, EGSR 2012) plus the transmittance to the lamp.
- The sun uses the closed-form transmittance along the shadow segment through the height field, and delta tracking where a local volume overlaps that segment.
- The camera ray is the only ray that carries the medium in v1. Surface bounces stay as they are.

The grade reuses `blur_relative` (`genos-lighting-ref`): `|Yc - Yr| / max(Yr, 0.25 * mean(Yr), 0.02)`, box blur, fail line 0.12. FLIP is reported and is not the gate, same as a bounced lighting picture. The near-camera mask (nearest quarter of the slices) fails at 0.08. Lavapipe does not judge this picture. The owner GPU does. `fog-check` does not launch the tracer.

## 10. Preset cost

VRAM is the grid plus the column target. 16 bytes per froxel, 8 bytes per column. Figures are decimal MB (1e6 bytes). Milliseconds are ceilings at 1440p on an RTX 4070, to calibrate in the first implementing phase, not measurements. A pass that measures over its ceiling loses samples first, then far slices. Near slices stay.

| | Potato | Low | Medium | High |
|---|---|---|---|---|
| Grid | none | 64×36×24 | 96×54×32 | 160×90×48 |
| Froxels | 0 | 55 296 | 165 888 | 691 200 |
| Bytes | 0 | 903 168 | 2 695 680 | 11 174 400 |
| VRAM | 0 | 0.903 MB | 2.696 MB | 11.174 MB |
| VRAM ceiling | 0.05 MB | 1.5 MB | 4 MB | 12 MB |
| In-scatter samples / frame | 0 | 4 000 | 12 000 | 32 000 |
| Point lamps | 0 | 0 | 4 | 8 |
| Local volumes | 0 | 0 | 8 | 32 |
| World-cache read | off | off | coarse | on |
| Fog history | off | off | off | off |
| ms ceiling, 4070, 1440p | 0.05 | 0.40 | 0.80 | 1.20 |

High fog at the ceiling is 11.2 MB. [ADR 0013](../adr/0013-lighting-invalidation-is-local.md) puts High lighting near 140 MB of the ~200 MB soft budget. 152 MB still fits a 4 GB card. Low on a GTX 1060 at 1080p uses the same 0.903 MB grid. Its millisecond ceiling on that card is 1.00 ms, so the 60 fps Low frame (16.7 ms) keeps a margin. The 4070 Low ceiling stays 0.40 ms. The controller moves the sample cap, not the grid, when a card misses its ceiling.

Work that scales with the picture is the upsample. It is one full-resolution gather of a few taps, gated by density. It is inside the ceiling, not added on top.

## 11. Success metrics

Lower is better. A value at the threshold passes. A negative value fails. `fog-check` prints one line per metric:

```text
<name> <value> <threshold> <PASS|FAIL|NOT-IMPLEMENTED>
```

A missing feature prints `-` for the value and `NOT-IMPLEMENTED`. It does not print a zero. JSON is `<out>/metrics.json`. The default directory is `./genos-debug/fog/`. `--out` overrides it. `GENOS_FOG_OUT` applies only when `--out` was omitted.

| Metric | Threshold | What it measures | This PR |
|---|---|---|---|
| `beer_lambert_max_abs` | 1e-4 | Max abs error of transmittance against a 64-step product, σ = 0.5, distances 0, 0.5, 1, 2, 4, 8 m | Graded. The good fixture calls `transmittance`. Broken fixture is `max(0, 1 - σ d)`. |
| `height_fog_density_max_abs` | 1e-4 | Max abs error of `σ0 exp(-(y-y0)/H)` at six heights, σ0 = 0.08, H = 40 | Graded. The good fixture calls `height_fog_density`. The oracle is forward Euler. Broken fixture is a linear ramp. |
| `height_ray_optical_depth_max_abs` | 1e-4 | Max abs error of `σ0 * H * exp(-y0/H) * (1 - exp(-dy/H)) / cos θ` on slanted rays, a downward ray, a ray whose base height is not 0, and a horizontal ray (`σ(y0) * distance` when `|cos θ| < 1e-4`) | Graded. The good fixture calls `height_optical_depth`. The oracle is a midpoint integral. Broken fixture omits `/ cos θ`. |
| `phase_hg_max_abs` | 1e-6 | Max abs error of Henyey-Greenstein against locked samples, including `1/(4π)` at g = 0 | Graded. The good fixture calls `phase_hg`. Broken fixture returns 1. |
| `phase_hg_integral_abs` | 1e-4 | Max abs error of the sphere integral of Henyey-Greenstein against 1, Simpson in μ times `2π`, at g = 0, 0.4, and 0.7 | Graded. The good fixture integrates `phase_hg`. Broken fixture drops the `4π`. |
| `homogeneous_inscatter_max_abs` | 1e-3 | Max abs error of `(S/σ)(1-exp(-σ D))` against piecewise steps, σ = 0.2, S = 0.15 | Graded. The good fixture calls `homogeneous_inscatter`. Broken fixture adds `S*D` with no transmittance. |
| `image_grade_blur_rel` | 0.12 | `blur_relative` of a synthetic fog picture against itself. The lighting-ref fail line. | Graded. Broken fixture scales luminance by 1.4. |
| `froxel_temporal_smear_px` | 0.50 | Centroid lag, in picture pixels at 1440p, of a static shaft during a 90° yaw | Graded from lavapipe column readback at High 160×90×48, every one of the 320 yaw frames. History is off. Holding the first frame fails. |
| `fog_yaw_pop_rel` | 0.05 | Max frame-to-frame relative luminance of the fog term on that yaw, after frame 8 | Graded on the CPU High grid. History is off. Scaling odd frames by 1.4 fails. |
| `local_volume_leak` | 1e-4 | Peak extinction outside a local box, divided by the peak inside | Graded. Height density is zero and one box sits on the view axis. A write outside that box fails the gate. |
| `sphere_volume_leak` | 1e-4 | Peak extinction outside a local sphere, divided by the peak inside | Graded from the lavapipe extinction image. Ignoring the sphere bound fails the gate. |
| `shaft_shadow_blur_rel` | 0.12 | Blurred relative luminance of the shadowed GPU columns against the CPU shadowed integral | Graded. The CPU model is the reference. Skipping the shadow lookup fails. `--fixture broken` compares the unshadowed CPU integral with the shadowed one. |
| `gi_inscatter_blur_rel` | 0.12 | Same grade where the only light in the medium is the world cache | `NOT-IMPLEMENTED` until lighting phase 3 |
| `media_path_blur_rel` | 0.12 | Full picture against a local closed form, not `genos_math` | Graded from lavapipe columns. The broken fixture scales the CPU picture by 1.4. |
| `near_fog_blur_rel` | 0.08 | The same grade on the nearest quarter of the slices | Graded from lavapipe columns against that segment's local closed form. |
| `fog_still_flicker` | 0.02 | Mean frame-to-frame relative luminance of the fog term, camera still | Graded from two lavapipe frames. History is off. Scaling the second frame by 1.4 fails. |
| `frame_column_abs` | 1e-3 | Peak abs error of the frame's own column buffer against the CPU integral of that frame | Graded from the Low frame after the depth prepass. An empty buffer fails. A separate `run_fog` dispatch is not this row. |
| `frame_picture_unchanged` | 0 | 0 when fog changes the composed picture, 1 when the fog-on and fog-off pictures match | Graded from two Low frames. The compose header is written on the device only inside the frame dispatch, so a missing dispatch and a compose that ignores fog both score 1. |
| `froxel_vram_potato_mb` | 0.05 | Allocated fog VRAM | Graded. Lavapipe allocation, decimal MB. Potato is 0. Doubling the High grid fails 12 MB. |
| `froxel_vram_low_mb` | 1.5 | Allocated fog VRAM | Graded. Lavapipe allocation, decimal MB. |
| `froxel_vram_medium_mb` | 4 | Allocated fog VRAM | Graded. Lavapipe allocation, decimal MB. |
| `froxel_vram_high_mb` | 12 | Allocated fog VRAM | Graded. Lavapipe allocation, decimal MB. |
| `froxel_ms_potato` | 0.05 | Fog pass time, 4070, 1440p | Graded when a lavapipe window exists. Potato does not dispatch, so the time is 0. The figure is a lavapipe timestamp, not an RTX 4070 measurement. |
| `froxel_ms_low` | 0.40 | Fog pass time, 4070, 1440p | Graded on lavapipe, or `NOT-IMPLEMENTED` with no window. Samples drop before far slices. |
| `froxel_ms_medium` | 0.80 | Fog pass time, 4070, 1440p | Graded on lavapipe, or `NOT-IMPLEMENTED` with no window. |
| `froxel_ms_high` | 1.20 | Fog pass time, 4070, 1440p | Graded on lavapipe, or `NOT-IMPLEMENTED` with no window. An unreduced cap that exceeds the ceiling fails. |

Exit status: 0 when no selected metric failed, including a run whose rows are `NOT-IMPLEMENTED`. 1 when any metric failed. 2 when an argument is bad. `--suite analytic` is the closed forms plus the synthetic picture. `--suite feature` is the renderer metrics. `--suite all` is both. `--fixture good` grades lavapipe readback against the CPU model and the local closed form. `--fixture broken` runs the `*_broken` helpers. The millisecond ceilings are RTX 4070 figures. A lavapipe timestamp is labelled as lavapipe and can miss those ceilings. That miss is a fail, not a pass.

The good analytic run exits 0. The broken analytic run exits 1. A measured 0 on a feature that is absent is still `NOT-IMPLEMENTED`. `gi_inscatter_blur_rel` stays absent.

Phase 0 on this tree (`cargo run -p genos-fog-check --offline --locked`):

| Metric | Good | Broken |
|---|---|---|
| `beer_lambert_max_abs` | 9.992007e-16 PASS | 3.678794e-1 FAIL |
| `height_fog_density_max_abs` | 3.678833e-7 PASS | 2.942999e-2 FAIL |
| `height_ray_optical_depth_max_abs` | 7.793025e-8 PASS | 1.428326e0 FAIL |
| `phase_hg_max_abs` | 0 PASS | 9.756395e-1 FAIL |
| `phase_hg_integral_abs` | 9.264753e-6 PASS | 1.156649e1 FAIL |
| `homogeneous_inscatter_max_abs` | 7.216450e-16 PASS | 6.014224e-1 FAIL |
| `image_grade_blur_rel` | 0 PASS | 4.000000e-1 FAIL |

Good exits 0. Broken exits 1. That table is the phase-0 analytic run. The feature suite now grades the froxel. JSON is `genos-debug/fog/<fixture>/metrics.json`.

## 12. Debug tool and tests

Fog is a lighting feature. Later phases extend `debug/lighting` (`genos-lighting`). They do not add `debug/fog`. Scenes stay in `debug/scenes/`. The menu page is `Fog`. Controls are the setting keys: sliders for density, height, thickness, albedo, and phase g; checkboxes for enabled, gi, and history; a preset dropdown. No help text.

Views, with no help text, labelled by the key: `extinction`, `inscatter`, `transmittance`, `history` (accept weight, zero while the setting is off), `age`, `budget`, `volumes` (box edges). Counters: dispatches, samples used, samples capped, history accepts, history rejects, VRAM bytes, pass milliseconds. Accepts and rejects stay at zero while history is off. MCP: `get_fog_settings`, `set_fog_settings`, `get_fog_stats`. The catalogue entry is `crates/debug/src/commands.rs`, same JSON for MCP, `POST /cmd`, and Rhai.

| Failure | Probe |
|---|---|
| Density zero still dispatches | `fog_dispatches` is 0 |
| Shaft smears on a yaw | `froxel_temporal_smear_px` |
| Yaw pops | `fog_yaw_pop_rel` |
| Density outside a box | `local_volume_leak` |
| Density outside a sphere | `sphere_volume_leak` |
| Shaft ignores a wall | `shaft_shadow_blur_rel` |
| Cache light missing in the medium | `gi_inscatter_blur_rel` |
| Picture misses the reference | `media_path_blur_rel`, `near_fog_blur_rel` |
| Still camera flickers | `fog_still_flicker` |
| Over the ceiling | `froxel_ms_*`, `froxel_vram_*` |
| History smears or pops, if a later decision enables it | `froxel_temporal_smear_px`, `fog_yaw_pop_rel` |

Rhai gates, added with the phase that can run them, under `debug/lighting/scripts/gates/`:

| Script | Phase | Pass |
|---|---|---|
| `fog-still.rhai` | only if history is enabled | `fog_still_flicker` ≤ 0.02 |
| `fog-yaw.rhai` | only if history is enabled | smear ≤ 0.50 px and yaw pop ≤ 0.05 |
| `fog-height.rhai` | height fog | `height_fog_density_max_abs` ≤ 1e-4 and image grade ≤ 0.12 |
| `fog-local.rhai` | local volumes | leak ≤ 1e-4 |
| `fog-shaft.rhai` | shafts | shaft grade ≤ 0.12 |
| `fog-gi.rhai` | world-cache read | GI grade ≤ 0.12 |

Those scripts are not in this change. The harness that runs today:

```sh
cargo run -p genos-fog-check -- --suite analytic --fixture good --out genos-debug/fog/good
cargo run -p genos-fog-check -- --suite analytic --fixture broken --out genos-debug/fog/broken
cargo run -p genos-fog-check -- --suite feature --out genos-debug/fog/feature
```

`cargo test -p genos-fog-check` grades the feature rows on lavapipe. The analytic rows do not need a GPU. The millisecond ceilings stay the 4070 figures. A lavapipe time is not that measurement.

## 13. Settings and arguments

`FogSettings` is a public `EngineSettings` value on the scene, beside lighting. Keys: `enabled`, `density`, `height`, `thickness`, `albedo`, `phase_g`, `gi`, `history`. Preset applies the table in §10. A game may map its own names onto these keys.

Arguments on `genos-lighting`, later phases: `--fog on|off`, `--fog-density`, `--fog-height`, `--fog-thickness`, `--fog-albedo`, `--fog-phase`, `--fog-gi on|off`, `--fog-history on|off`. Mirrors, only when the argument was omitted: `GENOS_FOG`, `GENOS_FOG_DENSITY`, `GENOS_FOG_HEIGHT`, `GENOS_FOG_THICKNESS`, `GENOS_FOG_ALBEDO`, `GENOS_FOG_PHASE`, `GENOS_FOG_GI`, `GENOS_FOG_HISTORY`.

`fog-check` arguments: `--suite analytic|feature|all`, `--fixture good|broken`, `--out DIR`, `--help`. Mirrors: `GENOS_FOG_SUITE`, `GENOS_FOG_FIXTURE`, `GENOS_FOG_OUT`. The argument wins. Output is `./genos-debug/fog/` unless `--out` is set. The tool does not write under `$HOME`.

## 14. Networking

- Replicated state: the fog settings and the local-volume list (shape, position, radius or extents, density, albedo, phase g). Not the grid, not the history, not the column target.
- Authority: the machine that owns the scene writes the settings. Each client builds its own grid.
- Prediction / rollback: none for the grid. Settings follow the scene snapshot. A wrong volume is corrected by the snapshot. The grid is not rolled back.
- Determinism: extinction is determined by the settings and the camera. Shadow rays follow the tracer. History, if a later decision enables it, is per client and is not in the snapshot. The default is off.
- Headless server: no grid and no picture. It replicates the settings only.

The implementing commit copies these five lines into the lighting record.

## 15. Phases

Each phase is its own change, with an A/B against the metric it names. A miss is dropped: the note records the claim, the check, the fault, the first bad frame, and what was reverted.

| Phase | What lands | Gate |
|---|---|---|
| 0 | This design, the shared closed forms, and `fog-check`. No froxel draw. | Analytic good exits 0. Analytic broken exits 1. Feature exits 0. A bad argument exits 2. |
| 1 | Height fog, column integral, upsample, composite. Density zero skips. | `beer_lambert_max_abs`, `height_fog_density_max_abs`, `image_grade_blur_rel`. VRAM and ms recorded, not yet capped by a measured pass. |
| 2 | Local volumes and puff injection. | `local_volume_leak` |
| 3 | Sun shafts, traced, budgeted. | `shaft_shadow_blur_rel` on the sun |
| 4 | Point lamps inside the cap. | The shaft grade with point lamps on |
| 5 | Held. Fog history stays off until Anthony decides. | If that decision enables history, the evidence is `froxel_temporal_smear_px`, `fog_yaw_pop_rel`, and `fog_still_flicker`. |
| 6 | World-cache read. | `gi_inscatter_blur_rel` |
| 7 | Media in the path tracer. Camera ray only. | `media_path_blur_rel`, `near_fog_blur_rel` |
| 8 | Ceilings enforced. Sample cap shrinks before far slices. | `froxel_vram_*`, `froxel_ms_*` on the 4070. The 1060 Low 1.00 ms check is an owner-machine command. |

## 16. Open questions

1. Multi-scatter lobe: phase and albedo scale, after phase 7 is green. Not in v1.
2. Whether to enable fog history on any preset. The default is off on all four, because [ADR 0012](../adr/0012-gi.md) stores no radiance history. The evidence is `froxel_temporal_smear_px` and `fog_yaw_pop_rel` in §7. This document does not choose an outcome and does not adopt a reject rule.
3. Far plane 64 m against the stress building. The tail is analytic height fog. If a checkpoint looks past 64 m into a local volume, raise `z_far` and drop far slices, not near slices.
4. Whether Low on a 1060 can afford one local volume inside 1.00 ms. The table says no until that measurement.

## 17. Glossary

New entries, one meaning each, added to [`docs/lighting/GLOSSARY.md`](../lighting/GLOSSARY.md) in this change:

| Term | Role | Meaning |
|---|---|---|
| Participating medium | OUT | A volume that absorbs and scatters along a ray. |
| Extinction | STORE | Light removed per metre (`σ_t`). |
| Optical depth | STORE | Extinction integrated along a path. |
| Transmittance | OUT | Fraction of light that survives a path. Beer-Lambert. |
| In-scattering | OUT | Light the medium adds to a ray. |
| Phase function | SAMPLE | Directional pattern of one scatter. Henyey-Greenstein. |
| Froxel | STORE | One cell of the frustum-aligned volume. |
| Froxel grid | STORE | The 3D grid of froxels. Preset-sized, not picture-sized. |
| Height fog | STORE | Extinction that depends only on world height. |
| Local fog volume | STORE | A box or sphere of analytic density added into the grid. |
| Light shaft | LIGHTS | A beam made visible by shadowed direct light in the medium. |
| Fog history | TEMPORAL | A reserved slot for the previous froxel's in-scattering. Off on every preset. Enabling it is open. |

Comparison against the glossary roles: the grid is new storage (no cache entry does this job). The sample cap extends work caps. The phase function is new sampling. Fog history is new temporal state and does not replace MSME. The shaft shadow extends the shadow cell cache. The composite extends compose. `fog-check` extends the reference tester.

## 18. Networking copy

- Replicated state: fog settings and the local-volume list. Not the grid.
- Authority: the scene owner writes settings. Clients build grids.
- Prediction / rollback: none for the grid.
- Determinism: extinction follows the settings and the camera. History stays off unless a later decision enables it, and then it stays on the client.
- Headless server: settings only. No grid.
