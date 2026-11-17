# 0025: The glTF metallic-roughness BRDF has one CPU reference, and shaders are scored against it

Status: **Proposed**. Design: [`docs/systems/rendering/pbr-brdf-reference.md`](../systems/rendering/pbr-brdf-reference.md). Builds on [ADR 0014](0014-pbr-materials.md) (one material record) and [ADR 0019](0019-reflections.md) (the specular path).

## Context

ADR 0014 fixed one `PbrMaterial` and named glTF 2.0 Appendix B as the path tracer's target, but the real-time shade is still Lambert and there is no code that evaluates Appendix B. `genos_math::optics::ggx_ndf` exists for reflections, with no visibility term, no Fresnel mix, and no test of energy. The go-hard review of #62, #70, #72 and #76 found that the most expensive render work had no ground truth to score against.

## Decision

1. `genos_math::brdf` is the reference: GGX `D` (`alpha = roughness²`), height-correlated Smith GGX `V`, Schlick Fresnel with `F0 = 0.04` for dielectrics and base colour for metals, Lambert diffuse `base × (1 − metallic) / π`, combined by Appendix B's `fresnel_mix` and `conductor_fresnel`, mixed by `metallic`. Emission is `emissive × KHR_materials_emissive_strength`, added after reflection, never reflected.
2. It is `f64`, scalar, table-free, and lives in `genos-math`, which builds and tests headless with no Vulkan, Wayland, or shader compiler.
3. Its own gate is `cargo test -p genos-math --test brdf`: closed forms (GGX peak `1/(πα²)`, Smith at normal incidence, `1 − ln 2` for a white rough metal, normal-incidence hand values), the GGX projected-area integral, the Smith weak white furnace, reciprocity, and white-furnace bounds on whole materials.
4. Every GPU shade of this BRDF (G-buffer specular, path tracer hit shade, cache shade) gets a parity test that evaluates both on the same cosines and materials and fails above a stated tolerance. The reference is not retuned to make a shader pass.

## Departures

- **Energy at grazing view.** Appendix B's dielectric mix scales diffuse by `1 − F(v·h)` per half vector, which does not conserve energy: a white dielectric at roughness 0.3 seen at `n·v = 0.1` reflects 1.284 of what it receives. The reference keeps the formula and pins that number, so shaders match the specification. A multiple-scatter or albedo-scaled correction (Kulla and Conty 2017, `KHR_materials_*` energy work) is a later decision with its own reference value.
- **Single scattering.** A white metal at roughness 1 seen head-on returns `1 − ln 2 ≈ 0.307`. The lost energy is not added back.
- **Filament's `V` approximation** and Unreal's Schlick-GGX `k = α/2` are not the reference. Shaders may use them only inside the parity tolerance.

## Rejected

- **Putting the reference in `genos-render` or `genos-gpuref`.** Both need the GPU toolchain to build, which makes the oracle unavailable exactly where shader work is reviewed.
- **A precomputed lookup table as the reference.** A table is an implementation choice to be scored, not the score.
- **Monte Carlo integration in the tests.** Deterministic quadrature gives the same number on every machine and converges to the closed forms above.

## Consequences

The real-time specular lobe (ADR 0019) and the path tracer's material shade each land with a parity test against `genos_math::brdf`. The design doc lists the tolerances and the order.
