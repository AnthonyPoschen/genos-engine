# PBR BRDF reference (glTF metallic-roughness and emissive)

Decision: [ADR 0025](../../adr/0025-pbr-brdf-cpu-reference.md). Material record: [ADR 0014](../../adr/0014-pbr-materials.md), [PBR materials design](../pbr-materials-design.md).

## Goal

One CPU evaluation of the glTF 2.0 Appendix B BRDF that every GPU shade is scored against, so a specular or material change is judged by a number, not by eye.

## Model

Inputs are a `Material` (`base_color`, `metallic`, `roughness`, `emissive`, `emissive_strength`, all linear) and the four cosines `n·l`, `n·v`, `n·h`, `v·h`.

| Term | Formula |
|---|---|
| `alpha` | `roughness²` |
| `D` | `α² / (π ((n·h)²(α² − 1) + 1)²)` |
| `V` | `0.5 / (n·l √((n·v)²(1 − α²) + α²) + n·v √((n·l)²(1 − α²) + α²))` |
| `F(f0)` | `f0 + (1 − f0)(1 − v·h)⁵` |
| dielectric | `(1 − F(0.04)) · base/π + F(0.04) · D · V` |
| metal | `F(base) · D · V` |
| `f` | `mix(dielectric, metal, metallic)` |
| radiance | `emissive × strength + f · Lᵢ · n·l` |

`directional_albedo` integrates `f · n·l` over the hemisphere by midpoint quadrature. It is the white furnace: under uniform radiance 1 the reflected radiance equals it.

## Reference values (the oracle)

| Check | Value | Tolerance |
|---|---|---|
| `∫ D cosθh dω` | 1 | 2e-3 |
| `D(1, α)` | `1/(πα²)` | 1e-9 |
| Smith weak white furnace | 1 | 1e-2 |
| `V(1, 1, α)` | 0.25 | 1e-12 |
| white metal, roughness 1, `n·v = 1` | `1 − ln 2` | 1e-5 |
| white dielectric, roughness 0.3, `n·v = 0.1` | 1.2837 | 2e-3 |
| white metal albedo, any roughness and view | ≤ 1 | 5e-3 |
| reciprocity `f(l,v) = f(v,l)` | equal | 1e-12 |

## Stages

1. **Reference and its tests** (this change). `cargo test -p genos-math --test brdf`.
2. **Shader parity harness.** A headless test feeds a grid of materials and cosines through the GLSL shade (compiled once, run on the GPU) and compares to the reference. Tolerance: relative 1e-3 for `D`, `V`, `F`; 1e-2 on `f` where a shader uses an approximation. Runs on the 4070 in the first PR.
3. **G-buffer specular lobe** (ADR 0019 path). Gate: parity harness plus a presented-frame change on DamagedHelmet.
4. **Path tracer material shade.** Gate: parity harness, then the path-traced reference picture of `panel.gltf` against the materials gate in ADR 0014.
5. **Emissive.** The shade adds emission; the surface light cache does not store it (ADR 0014). Gate: emissive parity row in the harness, plus a cache read that excludes emission.

Each stage is its own PR with its own gate.
