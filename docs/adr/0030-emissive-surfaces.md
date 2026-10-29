# 0030: Emissive surfaces are lights

Status: **Proposed**. Reference: `genos_math::emissive`. Builds on [ADR 0012](0012-gi.md), [ADR 0014](0014-pbr-materials.md), [ADR 0022](https://github.com/AnthonyPoschen/genos-engine/blob/lighting-v3/docs/adr/0022-lighting-v3-compute-architecture.md) (accepted on `lighting-v3`, not a file on master), and [ADR 0023](0023-graphics-settings-presets.md).

## Context

glTF stores emission as `emissiveFactor`, an optional `emissiveTexture`, and `KHR_materials_emissive_strength` (strength 1 when the extension is absent). The loader already reads those into `PbrMaterial`. ADR 0014 stores irradiance in the surface cache and adds emission at the shade. ADR 0012 and the GI design light every emissive surface through that cache, and they extract small or bright regions at import into 64 power-stratified points in the light tree.

ADR 0022 replaces direct lighting with ReSTIR DI: one or two shadow rays per pixel, picked from the light tree, with spatial and temporal reservoir reuse. A change record resets temporal confidence. Direct light is computed every frame on every preset (ADR 0023). Low is a GTX 1060 at 1080p, inside a lighting budget of about 16 ms and about 200 MB.

A surface that only adds its own colour in the picture does not light the room. Bloom, an ambient lift, and a clamp of strength to 1 are the same kind of substitution. The integral below is the light.

`crates/math/src/brdf.rs` is not on master (tree `46b94c59`). Pull request #101, branch `feat/pbr-reference`, defines `Material::emission` as `emissive × strength`, added after the BRDF and not multiplied into it. This decision uses that product and does not call that file.

## Decision

1. **Emitted radiance** at a shade sample is `emissiveTexture` (sRGB decoded to linear) × `emissiveFactor` × `emissiveStrength`. With no texture the factor covers the triangle. A zero product emits nothing. An alpha-mask rejection emits nothing. The picture still adds the shaded pixel's own emission after lighting. Channels are not clamped.
2. **Direct light** puts every triangle with nonzero emission into the ADR 0022 ReSTIR DI light tree, with the analytic lamps. A candidate picks a triangle with probability proportional to flux, then a point uniformly in area (Shirley: `b0 = 1 - sqrt(u)`, `b1 = v * sqrt(u)`). The source density with respect to solid angle is `p(triangle) * (1 / area) * r² / cos θ_light`. The reservoir target, the spatial and temporal reuse, and the one or two shadow rays stay ADR 0022. Emissive triangles do not add a second shadow-ray loop. A triangle does not sample itself. A strength change, a moving emissive body, or an animated emissive texture resets temporal confidence the same way a lamp change does. Direct light for that frame uses the new value.
3. **Flux** of a constant triangle is `π * area * L`, with `L` the Rec. 709 luminance `(0.2126, 0.7152, 0.0722) · Le` (the hold path's weights). A textured triangle uses the area-weighted mean of `Le` for that flux, and the shade-sample `Le` at the drawn point for the estimate. The area pdf does not follow the texture, so the estimate stays unbiased. A later power-warped area sample has to match the same rectangle integral.
4. **Bounce.** Card re-light and probe shading take direct emission from the same tree, so a wall beside a panel is lit without a bounce ray having to hit the panel. The cache stores irradiance. The shade adds emission. A bounce hit on a triangle that is in the tree does not add `Le` again. A bounce hit on a triangle the cap dropped does add `Le`, and that triangle is not in the tree. The camera's view of the emitter still shows its full emission.
5. **The cap** keeps the highest-flux triangles up to the preset count. The rest are the dropped set in decision 4. `emissive_flux_dropped / emissive_flux_total` is the gate. The cap is a ceiling on how many triangles are built. It does not scale emission down to fit.
6. **One-sided.** The front face is the winding. A `doubleSided` material emits both ways: the caller samples the flipped normal as a second light. The estimator does not light the back of a one-sided surface.
7. **Preset ceilings** below are models, not measurements. A row that misses its gate moves down a preset (ADR 0023). Shadow rays are the ReSTIR DI rays, shared with lamps. The millisecond ceiling is the triangle build and the candidate draw inside that pass. A triangle record is 128 bytes.

| Preset | Device | Cap | RIS candidates (shared) | Shadow rays | Build ceiling | Buffer |
| --- | --- | --- | --- | --- | --- | --- |
| Low | GTX 1060, 1080p | 256 | 4 | 1 | 0.40 ms | 32 KB |
| Medium | 4070, 1440p | 1 024 | 8 | 1 | 0.15 ms | 128 KB |
| High | 4070, 1440p | 4 096 | 16 | 2 | 0.25 ms | 512 KB |
| Ultra | 4070, 1440p | 16 384 | 32 | 2 | 0.40 ms | 2 MB |

8. **The reference lands first**, in `genos_math::emissive`, with no renderer, no shader, and no `f32` wrapper. `cargo test -p genos-math --test emissive` is the gate. On the offset ceiling (x from −0.2 to 1.4, z from −0.5 to 0.7, height 1.25, `Le = emitted_radiance([0.5, 0.25, 1.0], 3)`), stratified area samples of the two triangles give relative error 1.059e-2, 3.374e-3, 1.098e-3, 3.638e-4 at 4², 8², 16², and 32² cells per triangle (32, 128, 512, 2048 samples). The run requires the error to fall at each step and the last step to be under 5e-4. A 96² midpoint quadrature of `h² / r⁴` matches the closed form within 1.76e-5. The closed form on a plane that fills the front hemisphere matches `π` within 1e-9. The two triangles have equal area and equal radiance, so the mixture pdf is 1 over the rectangle area. Flux-weighted picking among many triangles is decision 2. It is not a second test.
9. **Settings**, when a renderer reads the cap: `emissive_triangle_cap` is an engine setting, on the MCP catalogue and the debug menu, and the preset writes it. The command-line argument wins over the environment mirror.

## Failure modes and probes

| Failure | What it looks like | Probe |
| --- | --- | --- |
| Solid-angle pdf bias | The rectangle test is dark or bright | `stratified_area_samples_converge_to_rectangle_irradiance`. Later `emissive_irr_err` on the emissive panel room in `debug/lighting`. |
| Back face, grazing, or zero distance | NaN, a firefly, or a black pixel | `back_face_grazing_and_coincident_samples_add_nothing`. Later `emissive_nan_samples`. |
| Counted twice | A sealed room settles above `E / (1 − albedo)` | Later `emissive_double_count` on `debug/lighting`. |
| Cap drops flux | A small emitter is missing on Low | Later `emissive_flux_dropped`, `emissive_flux_total`, `emissive_triangle_count`. |
| List rebuild hitch | A spike when many emissive bodies move | Later `emissive_build_ms`. |

The three later probes are named for the renderer change. This change does not add them, and it does not add a catalogue entry. The tunables are the cap, the shared RIS candidate count, and the flux rank. The candidate count is ADR 0022's direct budget.

## Rejected

These substitutes paint a glow and leave the integral wrong.

- Adding emission only in the picture, so the surface glows and the room stays dark.
- Clamping `emissiveStrength` to 1.
- Raising ambient fill because emissive meshes are in the scene.
- Treating bloom as illumination.
- Blurring the emissive buffer in screen space and calling that a light.
- Using the GI design's 64 fixed points as the direct integral.
- Pulling the estimate toward a preferred exposure.

## Departures

- **ADR 0012 and the GI design, path 2.** The 64 power-stratified points, and the rule that only a small or bright region enters the tree, are replaced by the triangle and a flux-ranked cap. A large dim surface can outrank a smaller brighter one when its flux is higher. `emissive_flux_dropped` is the record of that.
- **ADR 0014 decision 3.** The split between "in the tree" and "added on a hit" is the cap, not an import-time region. Triangles in the tree contribute emission only as direct samples.
- **ADR 0022.** Reservoir reuse, the shadow-ray counts, and the confidence reset stay. Emissive triangles are more lights in that tree. The ADR file is on `lighting-v3`, not on master.
- **Bitterli, Wyman, Shirley, and Lefohn, "Spatiotemporal reservoir resampling for real-time ray tracing with dynamic direct lighting", ACM TOG 39(4), 2020.** <https://doi.org/10.1145/3386569.3392481>. The CPU reference is the initial sample's area measure and solid-angle density. It does not implement spatiotemporal reuse, the target-function weight, or visibility. Those stay with ADR 0022's renderer stage.
- **Pharr, Jakob, and Humphreys, Physically Based Rendering, 3rd ed., "Sampling Shapes".** <https://pbr-book.org/3ed-2018/Light_Transport_I_Surface_Reflection/Sampling_Light_Sources#SamplingShapes>. Followed: `pdf_ω = pdf_A * r² / cos θ` on the front face. The back face returns no density and zero irradiance. The light is one-sided.
- **Howell's catalog of radiation configuration factors.** The closed form is `π` times the view factor from a differential area to a parallel rectangle. `f64::midpoint` is the average of the two terms, the same average as `½(A+B)` for these finite inputs.
- **Ureña, Fajardo, and King, "An Area-Preserving Parametrization for Spherical Rectangles", EGSR 2013.** Not this reference. A later solid-angle sampler may replace the area draw if it matches `parallel_rectangle_irradiance`.
- **Arvo, "Stratified Sampling of Spherical Triangles", SIGGRAPH 1995.** Not used.
- **Pull request #101.** `emitted_radiance` matches `Material::emission` there. This reference returns irradiance, before the Lambert factor `albedo / π`. The BRDF file is not on master, so the two functions are not linked.

## Consequences

Renderer work waits for ReSTIR DI on `lighting-v3`. The reference is on master so the integral is fixed before a shader copies it. The millisecond rows stay models until a GTX 1060 measures Low and a 4070 measures the other rows. [Math](../systems/math.md) records the five networking lines: nothing is replicated, every caller evaluates the same function, there is no prediction, the value is fixed by the arguments, and a headless server runs the same functions.

## Open questions

1. When pull request #101 merges, which function owns the product, `emitted_radiance` or `Material::emission`?
2. A city of windows can drop flux on the Low cap of 256. The fix is a stochastic tail or a higher cap. Which one?
3. A power-warped area sample for a masked texture has to match this rectangle integral. When is that worth the variance?
4. Does the name "promoted emitter" retire once this is accepted?
5. ADR 0022 is not on master. This decision does not merge `lighting-v3`. The tree cannot be built here until that direct-light stage exists.
