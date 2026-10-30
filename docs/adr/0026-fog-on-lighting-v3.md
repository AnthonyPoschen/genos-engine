# 0026: Fog on lighting-v3 reads ReSTIR direct and the probes, gated against the path tracer

Status: **Accepted** (29 Oct 2026). Engine decided under Anthony's delegated design authority; Anthony reviews later. Design: [`docs/systems/rendering/fog/plan.md`](../systems/rendering/fog/plan.md). Amends [ADR 0018](0018-fog-and-volumetric-lighting.md) for the lighting-v3 architecture ([ADR 0022](https://github.com/AnthonyPoschen/genos-engine/blob/lighting-v3/docs/adr/0022-lighting-v3-compute-architecture.md)) and the presets of [ADR 0023](0023-graphics-settings-presets.md).

## Context

ADR 0018 chose a budgeted froxel volume, lit by the old direct pass and the world cache, with ceilings for Potato, Low, Medium and High. Since then ADR 0022 replaced direct lighting with ReSTIR DI and far bounce with DDGI-style probes re-traced every frame, and ADR 0023 replaced the preset list with Low, Medium, High and Ultra. Fog has to read the new lighting without adding a second light loop, keep the light-switch frame inside the budget, and keep the basic scene at about 500 fps (2.0 ms) at 1440p on the 4070.

## Decision

1. **The froxel grid of ADR 0018 stays.** One view-aligned grid holds height fog, local fog volumes and in-scattering. Extinction is analytic and recomputed every frame; the composite stays `background * transmittance + in-scattering`.
2. **Direct in-scatter uses the ReSTIR light picks.** Each froxel draws its lamp candidates from the same light tree as ReSTIR DI and keeps one reservoir. It casts 1 shadow ray per froxel on Medium and 2 on Low, High and Ultra (Low raised to 2 in stage F1: with 1 ray its coarse slices miss the window-shaft gate), through ray queries when present and the software tracer otherwise. Froxel reservoirs reuse spatially only (neighbouring froxels in the same slice). They keep no temporal reservoirs, so a light switching on is in the fog on the same frame and fog cost does not depend on lamp count.
3. **Indirect in-scatter reads the DDGI probes.** Each froxel samples probe irradiance with Chebyshev visibility at its centre. Fog never writes the probes or the card cache.
4. **Height fog is analytic.** Exponential height density with the closed-form slanted optical depth from `genos_math::media`. With the froxel off (Low option), height fog is applied in compose by closed form only, lit by the sun and one ambient probe sample, so there is no pop when the grid toggles.
5. **No fog history on any preset.** ADR 0012 and ADR 0022 F1 show only finished values; the grid is re-lit in full every frame. Flicker is held down by a fixed per-froxel blue-noise jitter that is the same every frame for a still camera, and by tricubic upsampling of the grid in compose. A reprojected history stays an open decision for Anthony, as in ADR 0018.
6. **Per-preset ceilings** (1440p on the 4070 except Low; ceilings to calibrate, not measurements):

| Preset | Grid | Shadow rays/froxel | Fog ms | Fog VRAM |
| --- | --- | --- | --- | --- |
| Low (GTX 1060, 1080p) | 64×36×32, or height fog only | 2 | 0.9 on the 1060 | 2 MB |
| Medium | 96×54×48 | 1 | 0.30 | 5 MB |
| High | 160×90×64 | 2 | 0.45 | 15 MB |
| Ultra | 240×135×96 | 2 | 0.80 | 40 MB |

   The basic scene at High must stay ≥ 500 fps with fog on; fog has 0.45 ms of the 2.0 ms frame.
7. **The reference lands first.** The path tracer gains participating media (delta tracking for distance, equiangular sampling for point lamps) and a comparison test grades the closed forms and a CPU froxel model against it before any GPU fog code is written.

## Gates

Every gate is measured against the path-traced reference with fog, on the same machine as master.

- `fog_ref_err_near` (near-camera relative error of fogged luminance) ≤ 0.05 on every fog scenario.
- `fog_switch_frames_to_ref` ≤ 1: the frame a lamp switches on, its shaft is within the near error gate.
- `fog_flicker_max` (largest frame-to-frame luminance step on a still camera) below the oracle flicker threshold.
- `fog_yaw_pop_rel` and `froxel_temporal_smear_px` from ADR 0018 stay.
- Fog ms within the table above, and the switch frame within 10% of a steady frame.

## Rejected

- Temporal reservoirs for fog: a light switch would lag in the medium.
- Shadow maps for the medium: the engine traces shadows (ADR 0018).
- A separate fog light loop over all lamps: cost scales with lamp count.
- Screen-space radial shafts: a picture hack, not graded.

## Consequences

- The fog stages target `lighting-v3` after its stage E (probes) exists; the reference and comparison test go to master, since they are harness work.
- ADR 0018's Potato row and its "world cache" read are replaced by this ADR on lighting-v3.
