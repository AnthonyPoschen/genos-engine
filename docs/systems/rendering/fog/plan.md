# Fog and volumetrics on lighting-v3

Decision: [ADR 0026](../../../adr/0026-fog-on-lighting-v3.md). Earlier design: [fog-and-volumetric-lighting-design.md](../../fog-and-volumetric-lighting-design.md) ([ADR 0018](../../../adr/0018-fog-and-volumetric-lighting.md)). Presets: [settings-presets.md](../settings-presets.md).

## Goal

Photoreal fog and light shafts that match the path-traced reference, with no flicker or pops, a light-switch frame inside the frame budget, the basic scene at ≥ 500 fps at 1440p on the 4070, and a Low preset that fits a GTX 1060.

## Pipeline per frame

1. **Density pass.** Every froxel gets extinction and albedo from height fog (analytic) and local volumes. Cost: one write per froxel.
2. **Direct in-scatter.** For each froxel: pick lamp candidates from the ReSTIR light tree, resample to one reservoir, spatial reuse from 4 neighbours in the slice, then 1–2 shadow rays (ray query, or the software tracer). Phase function: Henyey-Greenstein.
3. **Indirect in-scatter.** One probe read per froxel (irradiance + Chebyshev visibility).
4. **Integrate.** Front-to-back along slices: transmittance and accumulated in-scatter, closed-form per slice (energy-conserving integral of Hillaire 2015).
5. **Compose.** Tricubic read of the integrated grid at each pixel depth: `background * transmittance + in_scatter`.

No pass keeps state between frames except the fixed blue-noise jitter table.

## Cost model (1440p, 4070, High)

| Pass | Froxels | Work | Modelled ms |
| --- | --- | --- | --- |
| Density | 921,600 | ALU | 0.03 |
| Direct | 921,600 | 2 rays + reservoir | 0.25 |
| Indirect | 921,600 | 1 probe read | 0.05 |
| Integrate | 14,400 columns | 64 steps | 0.04 |
| Compose | 3.7 M pixels | 1 tricubic read | 0.08 |
| **Total** | | | **0.45** |

Low on the GTX 1060 at 1080p: 73,728 froxels, 2 rays each, modelled 0.9 ms (0.6 ms with 1 ray; the direct pass is about half the fog cost, so a second ray adds about 0.3 ms); or height fog only in compose at about 0.05 ms. These numbers are models until the GPU half of stage F2 measures them. The CPU reference does not time the pass.

## Stages

| Stage | What | Target branch | Gate |
| --- | --- | --- | --- |
| F0 | Path tracer participating media + `fog-reference` comparison test | master | Closed forms match the path tracer within 1% on 5 scenes; test runs on CPU |
| F1 | CPU froxel model graded against F0 | master | `fog_ref_err_near` ≤ 0.05 on the model |
| F2 | CPU volumetric integration reference, then GPU density + integrate + compose | master for the reference; lighting-v3 for the GPU | reference: `fog_ref_err_near` ≤ 0.05 vs the ray march; Low shaft margin reported. GPU: matches that reference within 1%; ms ≤ table |
| F3 | ReSTIR direct in-scatter | lighting-v3 (after stage C) | `fog_switch_frames_to_ref` ≤ 1; `fog_flicker_max` under threshold |
| F4 | Probe indirect in-scatter | lighting-v3 (after stage E) | `fog_ref_err_near` ≤ 0.05; no wall leak in fog |
| F5 | Preset tables + Low height-fog-only | lighting-v3 | each preset in its ms and VRAM ceiling; basic scene ≥ 500 fps at High |

A stage that misses its gate is reworked or dropped, never merged on a promise.

## Reference comparison test (lands first)

`crates/debug/tests/fog_reference.rs`, CPU only, deterministic seeds. Scenes:

1. Homogeneous fog box, sun only (checks Beer-Lambert and HG integral).
2. Height fog over a plane, low sun (checks slanted optical depth).
3. Point lamp in fog (checks equiangular sampling against delta tracking).
4. Lamp switching on in a dark foggy room (frame 0 dark, frame 1 lit, no lag).
5. Shaft through a window (shadowed in-scatter).

Each scene compares the closed form or CPU froxel model with the path tracer at fixed sample counts and reports `fog_ref_err_near`. Renderer rows report `NOT-IMPLEMENTED` until the GPU half of F2. The CPU integration reference is `crates/debug/tests/fog_integrate.rs`.

## F1 results (CPU froxel model)

`crates/fog-ref/src/froxel.rs` rebuilds the grid for each preset every frame: closed-form optical depth per froxel, 2 stratified shadow rays per froxel on Low, High and Ultra (1 on Medium) at fixed blue-noise jitter, light picks resampled by unshadowed contribution, reuse of the 4 slice neighbours, then the F2 front-to-back integral, and a trilinear read in compose. `froxel_frame` is that injection, the disabled reprojection, and the integral. `crates/debug/tests/fog_froxel.rs` grades it against the F0 reference on 5 pixels per scene ray.

| `fog_ref_err_near` | Low | Medium | High | Ultra |
| --- | --- | --- | --- | --- |
| 1 homogeneous sun | 0.0027 | 0.0016 | 0.0010 | 0.0004 |
| 2 height fog low sun | 0.0078 | 0.0036 | 0.0015 | 0.0005 |
| 3 point lamp | 0.0323 | 0.0108 | 0.0051 | 0.0030 |
| 4 lamp switch dark room | 0.0070 | 0.0044 | 0.0013 | 0.0004 |
| 5 window shaft | 0.0483 | 0.0450 | 0.0388 | 0.0228 |

`fog_switch_frames_to_ref` is 1 on every preset (frame 0 exactly black, frame 1 within the gate), and `fog_flicker_max` is 0 on a still camera.

### Decision: Low casts 2 shadow rays per froxel

With 1 ray Low scored 0.147 on the window shaft, while 64 rays on the same grid scored 0.007. The miss was visibility noise, not grid layout. The options were tried in order: 2 rays first, then 2 rays with 3×3 reuse, then height-fog-only. 2 rays reach 0.048, inside the 0.05 gate, so Low keeps the froxel grid at about +0.3 ms modelled (0.9 ms on the 1060). The margin is thin. The CPU integration path rechecked it at 0.0483 (margin 0.0017 under 0.05). The GPU half of F2 must recheck the shaft again; if it slips, the next step is 3×3 reuse, then height-fog-only. Decided by the engine owner on Anthony's delegated authority, for his later review.

## F2 CPU reference (volumetric integration)

`crates/fog-ref/src/froxel.rs` follows Hillaire 2015's order on the CPU. `inject_froxels` writes the closed-form optical depth and the in-scattering source (the F1 light picks, including the 4-neighbour reuse). `reproject` is the identity: a previous volume is not read, and `history_accepts` and `history_rejects` stay 0. `integrate_front_to_back` is the energy-conserving slice integral, `L += T * (S / σ) * (1 - exp(-σ Δs))`, using the stored optical depth so a height-fog slice keeps the closed form. `froxel_frame` is those three calls. Compose stays the F1 trilinear read. No shader is involved.

`crates/debug/tests/fog_integrate.rs` grades that column against the F0 ray march (`Scene::reference`) on the same five scenes and the same five pixels per ray. The errors match the F1 table, because the integral is the one F1 already ran:

| `fog_ref_err_near` | Low | Medium | High | Ultra |
| --- | --- | --- | --- | --- |
| 1 homogeneous sun | 0.0027 | 0.0016 | 0.0010 | 0.0004 |
| 2 height fog low sun | 0.0078 | 0.0036 | 0.0015 | 0.0005 |
| 3 point lamp | 0.0323 | 0.0108 | 0.0051 | 0.0030 |
| 4 lamp switch dark room | 0.0070 | 0.0044 | 0.0013 | 0.0004 |
| 5 window shaft | 0.0483 | 0.0450 | 0.0388 | 0.0228 |

Low window shaft: `fog_ref_err_near` 0.0483, limit 0.05, margin 0.0017. The F1 record was the same figure. `history_accepts` is 0. Offering a lit previous volume to a dark frame leaves the composed radiance at 0.

Departures from Hillaire 2015 in this reference:

- Temporal reprojection is not applied (ADR 0026 decision 5; the same departure as ADR 0018). The stage is still called, and it returns the current injection.
- Extinction is the closed-form optical depth of the slice, not a density sample at the cell centre.
- In-scattering is single-scatter direct light. Probe indirect is stage F4 and is not added here.
- Shadows are rays against the boxes, not shadow maps.
- The picture is not marched. Compose is the F1 trilinear read of the integrated column.

### Probes

| Failure | Probe |
| --- | --- |
| Integrated column drifts from the ray march | `fog_ref_err_near` in `cargo test -p genos-fog-ref --test fog_integrate` |
| Low shaft slips the 0.05 gate | `low_shaft` margin line in `low_preset_shaft_margin` |
| A previous frame is blended | `history_accepts` and `history_rejects` (both 0); `temporal_reprojection_stays_off` |
| Preset grid or shadow-ray count changes the error | the per-preset columns of the same table |

The preset grid (`w`, `h`, `d`, shadow rays, reuse taps) is the tunable. The reference does not draw, so there is no overlay; the headless test is the probe. Fog stays on `debug/lighting` when the GPU pass lands. This change does not add `debug/fog`.

### Networking

- Replicated state: none. The injected grid and the integrated column are rebuilt every frame from the scene and the camera. Fog history is not stored.
- Authority: the machine that draws. The reference has no server.
- Prediction / rollback: none. A light change is in the next injection. A previous volume is not blended.
- Determinism: injection is a function of the scene, the camera, and the fixed jitter. Reprojection is the identity. The same inputs return the same column.
- Headless server: the reference does not draw. A headless server replicates fog settings only, as in the fog record.

## F2 GPU half

`FOG_F2_REFERENCE` on a froxel dispatch runs the same injection and front-to-back integral as `genos-fog-ref`: closed-form optical depth, two stratified shadow rays, the 4-neighbour reuse, and no tail past the reference far plane. The height-fog frame path does not set that flag, so its column stays the unshadowed integral. Potato still allocates no froxel, so a quit of that preset does not destroy an F2 pipeline. `crates/render/tests/froxel_gpu.rs` grades the Low grid on uniform density, height fog, a lamp, and a sun shaft. The gate is 1% relative where the reference is above 1e-3, otherwise 1e-5 absolute. Lavapipe timestamps are not the 4070 ceilings in ADR 0026.

### Probes

| Failure | Probe |
| --- | --- |
| GPU column drifts from fog-ref | `gpu_froxel_matches_fog_ref_within_one_percent` |
| The flag is ignored and the height-fog integral comes back | the same test's per-scene `f2` line |

### Networking

- Replicated state: none. The GPU grid is rebuilt from the scene, the camera, and `reference_far`.
- Authority: the machine that draws.
- Prediction / rollback: none. A light change is in the next injection.
- Determinism: the jitter table matches the CPU reference, so the same inputs return the same column.
- Headless server: the dispatch does not draw. A headless server replicates fog settings only.

## No pops, no flicker

- Fixed blue-noise jitter per froxel, constant for a still camera.
- No temporal reservoirs, so no lag or ghosting after a light change.
- Height-fog-only and froxel modes share the same closed form, so switching presets does not change mean brightness.
- Grid slices use an exponential distribution fixed per preset, so camera moves do not re-slice the volume.

## References

- Hillaire, "Physically Based and Unified Volumetric Rendering in Frostbite", SIGGRAPH 2015.
- Wronski, "Volumetric Fog", SIGGRAPH 2014.
- Bitterli et al., "Spatiotemporal reservoir resampling (ReSTIR)", SIGGRAPH 2020.
- Majercik et al., "Dynamic Diffuse Global Illumination with Ray-Traced Irradiance Fields", JCGT 2019.
- Kulla and Fajardo, "Importance Sampling Techniques for Path Tracing in Participating Media", EGSR 2012.
- [ADR 0018](../../../adr/0018-fog-and-volumetric-lighting.md), [ADR 0023](../../../adr/0023-graphics-settings-presets.md), [ADR 0026](../../../adr/0026-fog-on-lighting-v3.md).
