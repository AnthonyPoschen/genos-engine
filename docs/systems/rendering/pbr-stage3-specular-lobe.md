# PBR stage 3: the G-buffer specular lobe (design for review)

Status: **Proposed, for Anthony's review.** No shader code lands until this note and [ADR 0025](../../adr/0025-pbr-brdf-cpu-reference.md) (still Proposed) are reviewed. Stage list: [pbr-brdf-reference.md](pbr-brdf-reference.md#stages).

## Goal

The real-time picture is still Lambert, so metals render dark and dielectrics have no highlight. Stage 3 adds the glTF Appendix B specular lobe for **direct light** (sun and lamps) at G-buffer pixels, using the shared `pbr_brdf.glsl` that stage 2 already scores against `genos_math::brdf` on the GPU. Indirect specular stays with the reflections stack of [ADR 0019](../../adr/0019-reflections.md).

## Approach

1. **G-buffer:** store roughness and metallic per pixel (packed 2×8 bit into a spare field; the record is std430 in `gbuffer.frag`). Today only the base-colour and normal atlases are sampled; stage 3 adds the metallic-roughness texture (glTF B=metal, G=rough), falling back to the factors when absent. Diffuse albedo stays `base × (1 − metallic)` (ADR 0014); F0 is `mix(0.04, base, metallic)`.
2. **Direct pass:** the light loop already evaluates the shadow ray and `n·l` per light. Stage 3 multiplies by the full `pbr_brdf_f` (diffuse plus GGX `D·V·F`) instead of `albedo/π`. **No extra rays and no extra light picks.** The lobe replaces the Lambert term in the same pass; it does not run alongside it.
3. **Cache stays irradiance:** the surface light cache and probe hits keep the Lambert diffuse term (ADR 0014 decision 2). The view-dependent lobe is only computed at the pixel, so the camera does not change stored light.
4. **Branch:** lighting-v3 does not yet have `pbr_brdf.glsl` (it is on master from #107). Stage 3 targets master first, where the direct pass is the one shipping today, and is ported to lighting-v3 with ReSTIR DI (the reservoir's target function becomes the full BRDF, `p̂ = f·L·cos`), as a separate PR.
5. **No look hacks:** no ambient specular fill, no roughness clamp beyond `alpha ≥ 0.002` (already in the include for float range), no specular anti-aliasing tricks beyond what the reference also does. Very low roughness on point lamps will alias; that is measured by the flicker gate, and the fix if needed is Toksvig/Kaplanyan normal-variance roughness from the normal mip chain, graded against the reference, not a blur.

## Cost

| Item | RTX 4070, 1440p | GTX 1060 6 GB, Low 1080p |
|---|---|---|
| G-buffer: one more texture sample + 2 bytes | ~0.05 ms | ~0.1 ms |
| Direct: BRDF eval per (pixel, light) instead of Lambert | ~0.2–0.4 ms (ALU only, ~40 flops/light) | ~0.3–0.6 ms |
| VRAM | +0 to +4 MB (G-buffer field; MR texture already cooked) | same |

These are models; the direct-pass numbers in #124 show that code-read estimates can be badly off, so the 4070 measures before merge. Low may keep the lobe for the sun only if lamps miss its budget; that is a preset row, not a look change.

## Test plan and gates

1. **Parity (lavapipe and 4070):** `pbr_parity` keeps passing; a new row checks the direct-pass shade function on the same grid against `genos_math::brdf` (rel 1e-3 on D, V, F; 1e-2 on f).
2. **Picture (lavapipe):** DamagedHelmet presented frame differs from master in metal regions and keeps diffuse regions within 1/255 where metallic = 0 and roughness = 1 (the lobe is near-Lambert there).
3. **Energy (CPU):** white furnace on the reference already holds; the shader row for `f·cos` integrated over the hemisphere stays ≤ 1.
4. **Flicker (4070):** `lighting-oracle` still-camera max frame-to-frame delta unchanged from master within noise; spin/strafe pop gate unchanged.
5. **Cost (4070):** direct pass +≤ 0.5 ms at 1440p stress, basic scene stays ≥ 500 fps.

## Open choices for Anthony

- Accept ADR 0025 (currently Proposed) so stages 3–5 can land.
- Master first, then lighting-v3 port (proposed), or lighting-v3 only.
