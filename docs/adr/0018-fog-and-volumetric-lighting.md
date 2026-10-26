# 0018: Fog is a budgeted froxel volume

Status: **Proposed**. Design: [`docs/systems/fog-and-volumetric-lighting-design.md`](../systems/fog-and-volumetric-lighting-design.md).

## Context

The picture has no participating medium. Smoke is up to eight analytic puff spheres in `fog_along` (`crates/render/src/particles.rs`), with no shadows, no height field, and no read of the world cache. The path tracer does not trace fog (`crates/debug/src/reference.rs`). [ADR 0012](0012-gi.md) left a froxel volume that reads the same caches as a non-goal of GI v2. [ADR 0013](0013-lighting-invalidation-is-local.md) caps every expensive pass and forbids a full-frame fallback. The rendering extras plan allows a height tint on the miss and leaves a froxel volume out of that plan.

Anthony's targets stay in force: RTX 4070 at 1440p, Low tuned for a GTX 1060 6 GB and an RX 580, 4 GB minimum, `VK_KHR_ray_query` optional, macOS on the software tracer. The basic scene stays above 500 fps and the stress scene near 100 fps. The grade is the path-traced reference. Near-camera error matters most. No flicker and no pop.

## Decision

1. The realtime volume is a **froxel grid** (Hillaire, SIGGRAPH 2015; the same family as Wronski, SIGGRAPH 2014). One grid holds height fog, local fog volumes, and shadowed in-scattering. Per-pixel ray marching and epipolar shaft sampling are rejected.
2. Extinction is recomputed for the whole grid every frame it is enabled. It is analytic. In-scattering is the expensive term. It has a per-preset sample cap. Work beyond the cap waits. There is no full-volume refresh.
3. Fog history is off on Potato, Low, Medium, and High. [ADR 0012](0012-gi.md) stores no radiance history. Enabling fog history is an open decision for Anthony. The evidence is the smear gate and the yaw-pop gate in the design. This ADR does not choose that outcome. An unsampled cell shows the unshadowed closed-form integral.
4. The sun and point lamps shadow the medium through the existing tracer. Ray queries when `VK_KHR_ray_query` is present, otherwise the software tracer. macOS uses the software tracer. Low and Medium may read the shadow cell cache.
5. Indirect in-scattering is a read of the world cache. Fog does not write the cache.
6. The path tracer gains participating media (delta tracking, equiangular sampling) so the picture can be graded. Until that lands, `fog-check` grades the shared closed forms in `genos_math::media` (Beer-Lambert, height density, slanted height-fog optical depth, Henyey-Greenstein and its sphere integral, the homogeneous integral) and a synthetic picture, and marks every renderer metric `NOT-IMPLEMENTED`. A `NOT-IMPLEMENTED` row exits 0. A failed metric exits 1. A bad argument exits 2.

## Rules kept

Finished values only. The camera does not change stored surface light ([ADR 0012](0012-gi.md)). Budgets are ceilings ([ADR 0013](0013-lighting-invalidation-is-local.md)). Density zero runs no volume pass. Lighting VRAM stays inside the ~200 MB soft budget. The composite stays `background * transmittance + in-scattering`. No radial-blur shaft, and no other picture hack.

## Rejected

- **Per-pixel ray march** of the medium (Tóth and Umenhoffer, Eurographics 2009). Cost follows the picture size. It misses the High millisecond ceiling at 1440p.
- **Epipolar sampling** (Engelhardt and Dachsbacher, I3D 2010). One directional light, single scatter, no shared volume for local fog, point lamps, or the world cache.
- **Screen-space radial shafts.** A picture hack. Not graded.
- **Radiance history** (Hillaire's continuous blend, and the accumulation [ADR 0012](0012-gi.md) already refused for surfaces). Fog history defaults off. Turning it on is not decided here.
- **Shadow maps** for the medium. The engine traces shadows.
- **Writing fog into the surface light cache** in this design. Surfaces would relight the volume that relights the surfaces.

## Conflicts named

- [ADR 0012](0012-gi.md) keeps no radiance history for GI. Fog history is therefore off on every preset. Enabling it is an open decision for Anthony. The evidence is `froxel_temporal_smear_px` and `fog_yaw_pop_rel`. This ADR does not choose the outcome. The froxel does not write the surface cache.
- [ADR 0012](0012-gi.md) lists fog as a GI non-goal and leaves room for this volume. This ADR does not change the GI passes. It adds the reader.
- The extras plan's miss tint stays off while the froxel is on, so height fog is not applied twice.

## Consequences

Phases and gates are in the design. A phase that misses its gate is dropped. `fog-check` is the metric harness and does not draw. Preset ceilings (1440p, RTX 4070): Potato 0.05 ms and 0.05 MB, Low 0.40 ms and 1.5 MB, Medium 0.80 ms and 4 MB, High 1.20 ms and 12 MB. Those are ceilings to calibrate, not measurements.
