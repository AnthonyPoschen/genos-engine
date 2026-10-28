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

Low on the GTX 1060 at 1080p: 73,728 froxels, 2 rays each, modelled 0.9 ms (0.6 ms with 1 ray; the direct pass is about half the fog cost, so a second ray adds about 0.3 ms); or height fog only in compose at about 0.05 ms. These numbers are models until stage F2 measures them.

## Stages

| Stage | What | Target branch | Gate |
| --- | --- | --- | --- |
| F0 | Path tracer participating media + `fog-reference` comparison test | master | Closed forms match the path tracer within 1% on 5 scenes; test runs on CPU |
| F1 | CPU froxel model graded against F0 | master | `fog_ref_err_near` ≤ 0.05 on the model |
| F2 | GPU density + integrate + compose, height fog | lighting-v3 | matches F1 within 1%; ms ≤ table |
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

Each scene compares the closed form or CPU froxel model with the path tracer at fixed sample counts and reports `fog_ref_err_near`. Renderer rows report `NOT-IMPLEMENTED` until F2.

## F1 results (CPU froxel model)

`crates/fog-ref/src/froxel.rs` rebuilds the grid for each preset every frame: closed-form optical depth per froxel, 2 stratified shadow rays per froxel on Low, High and Ultra (1 on Medium) at fixed blue-noise jitter, light picks resampled by unshadowed contribution, reuse of the 4 slice neighbours, front-to-back closed-form integration, and a trilinear read in compose. `crates/debug/tests/fog_froxel.rs` grades it against the F0 reference on 5 pixels per scene ray.

| `fog_ref_err_near` | Low | Medium | High | Ultra |
| --- | --- | --- | --- | --- |
| 1 homogeneous sun | 0.0027 | 0.0016 | 0.0010 | 0.0004 |
| 2 height fog low sun | 0.0078 | 0.0036 | 0.0015 | 0.0005 |
| 3 point lamp | 0.0323 | 0.0108 | 0.0051 | 0.0030 |
| 4 lamp switch dark room | 0.0070 | 0.0044 | 0.0013 | 0.0004 |
| 5 window shaft | 0.0483 | 0.0450 | 0.0388 | 0.0228 |

`fog_switch_frames_to_ref` is 1 on every preset (frame 0 exactly black, frame 1 within the gate), and `fog_flicker_max` is 0 on a still camera.

### Decision: Low casts 2 shadow rays per froxel

With 1 ray Low scored 0.147 on the window shaft, while 64 rays on the same grid scored 0.007. The miss was visibility noise, not grid layout. The options were tried in order: 2 rays first, then 2 rays with 3×3 reuse, then height-fog-only. 2 rays reach 0.048, inside the 0.05 gate, so Low keeps the froxel grid at about +0.3 ms modelled (0.9 ms on the 1060). The margin is thin, so F2 must recheck the shaft on the GPU; if it slips, the next step is 3×3 reuse, then height-fog-only. Decided by the engine owner on Anthony's delegated authority, for his later review.

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
