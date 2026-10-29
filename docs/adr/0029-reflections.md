# 0029: Hierarchical-Z reflections

Status: **Proposed** (29 Oct 2026). Builds on [ADR 0019](0019-reflections.md) (accepted) and [ADR 0023](0023-graphics-settings-presets.md). Names [ADR 0024](https://github.com/AnthonyPoschen/genos-engine/pull/100) (proposed, on `lighting-v3`, not on master) and the glTF BRDF reference on [PR #101](https://github.com/AnthonyPoschen/genos-engine/pull/101). This ADR does not replace 0019. It specifies the screen march 0019 left unimplemented.

## Context

ADR 0019 accepts glossy reflections, planar mirrors, portals, and water. The glossy path is one screen march, then a surface-light-cache read, then an optional ray on the shared tracer. The march is not in the tree. `ssr_depth_rel` stays `NOT-IMPLEMENTED` in `reflections-check`. The design's data note calls the buffer a min-depth pyramid. Its departures table calls McGuire and Mara's march "Hi-Z". Those are different algorithms.

McGuire and Mara, "Efficient GPU Screen-Space Ray Tracing", Journal of Computer Graphics Techniques, vol. 3, no. 4, pp. 73–85, 2014, walk a perspective-correct DDA. The walk visits each pixel the ray crosses. A step cap stops it early. Yasin Uludag, "Hi-Z Screen-Space Cone-Traced Reflections", in Wolfgang Engel (ed.), GPU Pro 5, CRC Press, 2014, pp. 149–192, §4.4.3, walks a depth pyramid and skips empty cells. The printed tracer (Listing 4.5) reads the minimum. Listing 4.4 stores the maximum in the green channel, and the chapter says the demo's tracer uses both. The listing of that tracer is not in the chapter.

The picture has to match a path tracer. A miss has to show a finished cache or probe value. A fade, a dilation, or last frame's colour would hide a miss and fail that comparison. Low is a GTX 1060 6 GB and an RX 580, Vulkan 1.3, and ray queries stay optional (ADR 0012, ADR 0023).

## Decision

1. **The screen march is Uludag's hierarchical walk** (§4.4.3, Listing 4.2). One direction, `reflect(I, N)`, with `I` pointing at the surface, the same formula as `optics::mirror_dir`. The ray is affine in screen pixels and in `1/z` (the chapter's view-space alternative, §4.4.3 and §4.6.3). The CPU reference stores linear view-space depth in metres, camera at the origin, looking down −Z, Y up, pixel y down. That matches the engine view and the Y-flipped projection.
2. **The pyramid stores the minimum and the maximum.** A cell is skipped when the ray's depth interval over that cell misses `[min, max]` expanded by the thickness. A cell that overlaps and is coarser than one pixel descends one level and does not move. A cell that misses steps to its boundary and may ascend one level. The finest cell accepts the hit. Listing 4.5's minimum-only test is not the one we run. A ray already behind a cell's minimum descends into that cell, so a nearer surface the ray has passed spends the step cap. `behind_post` is that case: a post closer than the reflection sits on the screen segment, and the wall behind it is the hit. The min-max walk skips the post. The minimum-only walk does not.
3. **The finest level is 0.** Listing 4.5 stops at the demo's `HIZ_STOP_LEVEL` of 2. A two-level cell is coarser than `ssr_depth_rel` ≤ 0.02. We refine to the pixel.
4. **Thickness is 25 cm along the ray, in view depth**, `max(0.25 × |D_z|, 1 mm)`, the ADR 0019 thickness. The 1 mm floor is there because a ray parallel to the screen has `|D_z| = 0` and would never overlap a slab. The march starts 1 cm along the ray so it leaves the shading point. A hit on the receiver's own surface id is rejected. The GPU march will not have analytic ids; crossing the start cell (Listing 4.5) is its form of the same reject.
5. **A step cap bounds the walk.** The caps are ADR 0019's: Low 16, Medium 32, High 48. Ultra uses High (ADR 0019). `ReflectionPreset::Potato` stays 0. Potato is not a shipped preset (ADR 0023, ADR 0027). A walk that hits the cap, leaves the screen, or runs out of surfaces returns a miss.
6. **A miss reads finished light, in this order.** The surface light cache along the reflection direction, then the world radiance cache, then the ambient fill (ADR 0013's last resort). Roughness at or above 0.6 does not march. It uses the diffuse gather (ADR 0019). The stored record is the hit, not radiance. The shade reads the cache in the same frame.
7. **A screen hit is a position.** Its colour is the surface light cache at that point, this frame. ADR 0024 reads this frame's lit colour for contact and then cards and DDGI probes. Those stores are the lighting-v3 proposal. On master the stores are the surface light cache and the world radiance cache. The march is the same either way.
8. **`VK_KHR_ray_query` is optional.** Medium, High, and Ultra may spend the ADR 0019 ray cap on glossy pixels the march missed. The ray shades from the cache. The software SDF is the same output when the device has no ray queries, which is the macOS path. Low schedules zero rays. No preset requires ray-query hardware. The floor is Vulkan 1.3.
9. **Planar mirrors and portals stay extra raster views** (ADR 0019). The view cap is Low 1, Medium 2, High 3, Ultra 3. A further view is drawn while the running Fresnel product stays at least 0.02 and the cap has room. The near plane is the oblique clip. A view that does not fit shows the cache reflection.
10. **Water stays the ADR 0019 shade.** The reflection direction is the march above, on the Gerstner normal, once that pass exists. Refraction is one Snell direction of the opaque colour. Absorption is Beer–Lambert. The march is not the refraction. Caustics stay on the ADR 0019 preset table.
11. **The path change blends.** The centres stay 0.02 (planar) and 0.6 (diffuse gather). A band of 0.05 in roughness blends the two paths. ADR 0019 cuts at those centres. A hard cut pops when a material LOD crosses it. The band is the same width ADR 0024 proposes. The hit-finder does not blend. The later shade does.
12. **The grade is the path tracer and the analytic ray.** `ssr_depth_rel` ≤ 0.02 against the analytic intersection on the CPU scenes below. The picture, once the lobe is shaded, keeps ADR 0019's `near_weighted_rel` ≤ 0.03 and `flicker_reversal` ≤ 2/255. PR #101 is the BRDF the path tracer should call. This ADR does not evaluate it. There is no per-scene nudge and no fade to make a miss look finished.

## Cost

Ceilings, carried from ADR 0019, on the ADR 0023 presets. Low is the GTX 1060 6 GB and the RX 580 at 1080p. These are ceilings. They are not a measurement of those cards. Lavapipe time is not the millisecond cap.

| | Low | Medium | High | Ultra |
|---|---|---|---|---|
| Device | GTX 1060 6 GB, RX 580, 1080p | RTX 2060 class, 1080p | RTX 4070, 1440p | High's column |
| API | Vulkan 1.3 | Vulkan 1.3 | Vulkan 1.3 | Vulkan 1.3 |
| Ray query | off | if present, else the software tracer | if present, else the software tracer | High's column |
| Reflection ms | 0.40 | 0.70 | 1.00 | 1.00 |
| Reflection VRAM | 8 MB | 16 MB | 32 MB | 32 MB |
| Hierarchical-Z steps | 16 | 32 | 48 | 48 |
| Reflection rays | 0 | 32 000 | 128 000 | 128 000 |
| Mirror and portal views | 1 | 2 | 3 | 3 |

The pyramid is half resolution, minimum and maximum, `R16G16`. That doubles ADR 0019's min-only Hi-Z note: about 2.6 MB at 1080p and 5.0 MB at 1440p. Low's other reflection images stay 4.15 MB of hits and 0.8 MB of quarter-resolution mirror target. The sum is 7.55 MB, inside 8 MB. Work past a cap waits. It is not a full-frame fallback.

A basic scene with nothing in the glossy band, and no mirror and no water, still stops at the classify (0.05 ms). The pyramid is built only when that count is non-zero.

## CPU reference

`genos_math::ssr` is the hit-finder. It is `f64`, it allocates only the depth pyramid for the buffer it is given, and it does not call Vulkan. `cargo test -p genos-math ssr` is the gate. The scenes, camera at the origin:

| Scene | Analytic result |
|---|---|
| `floor_to_wall` | the reflection of a floor point hits a fronto-parallel wall |
| `floor_to_sphere` | the same kind of ray hits a sphere |
| `near_contact` | a short ray hits a nearby wall inside Low's 16 steps |
| `behind_post` | the ray passes behind a nearer post and hits the wall. The minimum-only walk takes more steps |
| `near_occludes_far` | the nearer of two walls |
| `leaves_screen` | the ray leaves the buffer; both sides miss |
| `beside_wall` | the ray passes beside a wall; both sides miss |

`ssr_depth_rel` is `|z_march − z_analytic| / max(z_analytic, 1e-3)` when both hit the same surface within one pixel, `0` when both miss, and `1` otherwise. The gate is 0.02. Two marches of one buffer return the same `Trace`. Moving the wall 2 cm moves the hit depth 2 cm. `reflections-check` still reports `ssr_depth_rel` as `NOT-IMPLEMENTED`: that row reads the GPU march, which this ADR does not add.

## How Uludag's chapter maps

| Chapter part | Role | Our term | Verdict |
|---|---|---|---|
| Hi-Z minimum pyramid, §4.4.1 | VIS | hierarchical-Z march | extend: we store the maximum too |
| Ray-tracing pass, §4.4.3 | SAMPLE | hierarchical-Z march | extend: finest level is the pixel; min-max test |
| Pre-integration, §4.4.2 | VIS | — | no-equivalent (one direction, no cone) |
| Pre-convolution, §4.4.4 | DENOISE | glossy denoise | no-equivalent (we do not blur a colour pyramid) |
| Cone trace, §4.4.5 | SAMPLE | — | no-equivalent (one direction, fixed by the normal and the view) |
| Screen-edge fade, §4.6.1 | OUT | — | rejected (a fade is a look hack) |
| Previous-frame bounces, §4.6.4 | TEMPORAL | — | rejected (radiance history) |

## Rejected

- **Listing 4.5's minimum-only test as the coarse decision.** The ray descends whenever it is behind the closest depth in the cell. In `behind_post` that closer depth is the post, and the walk spends its steps there instead of on the wall.
- **Stopping at Hi-Z level 2.** The demo constant. The depth error is then the cell, not the pixel.
- **Uludag's cone trace, pre-convolution, and pre-integration** (§4.4.2, §4.4.4, §4.4.5). ADR 0019 already chose one direction. Roughness at or above 0.6 uses the diffuse gather.
- **The screen-edge fade, the travel fade, and the toward-camera fade** (§4.6.1). A miss is a miss. The cache is the colour. A fade changes the comparison with the path tracer and is a look hack.
- **Hole dilation** (§4.6.2). The hole is the cache read.
- **Previous-frame colour for extra bounces** (§4.6.4). That is radiance history. Mirrors of mirrors are the view cap.
- **McGuire and Mara's DDA as the march.** It is the right thickness and the right step cap, and it visits every pixel. The pyramid exists so a short cap can still reach a near hit. ADR 0019's departures table calls that paper "Hi-Z". This ADR is the correction.
- **A cubemap, a baked reflection, or a scrolling normal as the miss colour.** The miss colour is the surface light cache, the world radiance cache, or the ambient fill.
- **A reflection ray on Low.** ADR 0019. The floor cards are compute-bound.
- **Stochastic lobe samples and a radiance history.** ADR 0012 and ADR 0019.

## Departures

| Source | What we take | Departure |
|---|---|---|
| Uludag, GPU Pro 5, §4.4.1 and Listing 4.4 | A depth pyramid. The demo stores min and max. | The printed construction listing is min-only. We store both, in linear view depth. |
| Uludag, §4.4.3, Listing 4.2 and Listing 4.5 | Descend on an overlapping cell, step to the boundary and ascend on a miss, one direction from the reflection formula. | The coarse test is the min-max overlap, which the chapter says the demo uses and does not print. We stop at level 0, not at level 2. The step cap is 16/32/48. Thickness is 25 cm along the ray with a 1 mm floor. The start is 1 cm along the ray, and the reference also skips the receiver id. |
| Uludag, §4.4.5 and §4.6 | — | Cone trace, fade, dilation, and previous-frame bounces are not used. |
| McGuire and Mara, JCGT 2014, §3 and §4 | Camera-space thickness and `maxSteps`. | Not the DDA. The thickness is 25 cm along the ray. `maxSteps` is the preset cap. |
| ADR 0019 data note, "min-depth pyramid" | The half-resolution pyramid and the 8/16/32 MB caps. | The pyramid is min and max. Low's Hi-Z estimate goes from 1.3 MB to 2.6 MB. The 8 MB cap still holds. |
| ADR 0019 classify | Cuts at roughness 0.02 and 0.6. | A 0.05 blend band across each cut. The centres stay. |
| ADR 0024 | Hierarchical-Z, then a world fallback, ray queries optional, no radiance history, Low at 0.40 ms and 0 rays. | The fallback on master is the surface light cache, then the world radiance cache. Cards and DDGI probes wait with lighting-v3. A screen hit is re-shaded from the cache, not copied from the lit image. |

## Probes

The CPU probe is `cargo test -p genos-math ssr`. The GPU row in `reflections-check` stays `NOT-IMPLEMENTED` until a later change marches on the device. `debug/reflections` is still the binary ADR 0019 names, and it is not in this change.

| Failure | Probe |
|---|---|
| The march hits the wrong depth | `ssr_depth_rel` on `floor_to_wall`, `floor_to_sphere`, `near_occludes_far`, `behind_post` |
| The march sticks to a wall it passes | `beside_wall` (both miss) |
| An off-screen ray becomes a hit | `leaves_screen` (both miss) |
| The march hits the receiver | `floor_to_wall` surface is the wall |
| The march accepts the post it passes behind | `behind_post` surface is the wall |
| Low's cap misses a contact hit | `near_contact` at 16 steps |
| The same buffer flickers | two marches return the same `Trace` |
| A 2 cm wall move pops the hit | the hit depth moves 2 cm |
| A min-only walk is treated as the decision | `min_plane_steps` on `behind_post` is greater than the min-max `steps` |

## Networking

- Replicated state: roughness, metallic, IOR, surface kind, portal link ids, Gerstner amplitudes, and the water clock. The depth pyramid, the hit record, and the mirror targets are derived.
- Authority: the server owns material factors, portal links, and the water clock. A client does not spawn a portal.
- Prediction / rollback: the picture is recomputed from the camera. It is not rolled back. Walking through a portal waits on the networking plan.
- Determinism: the march is a pure function of the depth buffer, the ray, the thickness, and the step cap. Fresnel, Snell, and the mirror and portal maps stay the ADR 0019 pure functions.
- Headless server: the server does not march, and it does not draw mirrors, water, or reflections. The CPU reference is the function a headless test calls. It is not a picture.

## Open questions

1. ADR 0024's lit-colour screen hit and its card-then-DDGI fallback are not on master. If that ADR is accepted, the stores change. The march in this ADR does not.
2. Listing 4.5's demo stopped at level 2. Refining to the pixel costs more steps. The 1060 cost of that last refinement is unmeasured. The ceilings above are not a measurement.
3. The 0.05 roughness band is unmeasured on a picture. The hit-finder does not blend. A later shade that pops at 0.02 or 0.6 fails `flicker_reversal` and the band is the first fix.
4. A long ray in a mixed cell can spend Low's 16 steps before a far hit. That miss is a cache read, by decision. Whether the cache value and the screen hit match at that boundary is unmeasured until the shade exists.
5. The GPU pyramid may store the G-buffer's depth instead of linear view depth. The comparison space has to be the one the march interpolates. The CPU reference uses linear view depth and `1/z`. A GPU port that interpolates a different encoding fails `ssr_depth_rel`.
6. PR #101's BRDF is the lobe the path tracer should use. It is not on master. This hit-finder does not call it.

## Consequences

The accepted mirror, portal, and water paths stay as they are. The new code is `crates/math/src/ssr.rs` and `crates/math/tests/ssr.rs`. No shader, no frame pass, and no edit to `lighting_schedule.rs`. A later GPU march is a separate change, and it copies the departures above into the lighting deviations list in full when it touches those passes. The one-line pointer is already in that list.
