# Fog F2, GPU half: density, integrate and compose graded against the CPU reference (design for review)

Status: **Proposed, for Anthony's review.** [ADR 0026](../../../adr/0026-fog-on-lighting-v3.md) was accepted under delegated authority and still awaits his review, so this GPU stage waits too. Plan: [plan.md](plan.md#stages). The CPU half of F2 merged as #111.

## Goal

Make the GPU froxel passes produce the same column as `crates/fog-ref` (`inject_froxels` → identity `reproject` → `integrate_front_to_back`, then the trilinear compose read) within 1%, on lighting-v3, inside the ADR 0026 ceilings. Lighting stays the F1/F2 single-scatter direct term; ReSTIR picks are F3 and probes are F4.

## What exists

lighting-v3 already carries the ADR 0018 froxel passes from #76: `froxel_inject.comp`, `froxel_integrate.comp`, `froxel_common.glsl`, `froxel_compose.glsl` (called from `lighting_compose.comp`), `fog_pass.rs`, and `tests/froxel_gpu.rs`, which checks the **unshadowed** integral only. So F2-GPU is a conformance change, not a new pass.

## Approach

1. **Inject** writes the F2 layout: closed-form slab optical depth per froxel (`genos_math::media` mirrored in `media.glsl`) and the in-scatter source, with 2 shadow rays per froxel on Low/High/Ultra and 1 on Medium at the fixed blue-noise jitter F1 uses. Same jitter table as the CPU model, so a still camera gives identical froxels every frame (no flicker, no history).
2. **Integrate** uses `L += T · (S/σ) · (1 − e^(−σΔs))` with the stored optical depth (no per-slice re-derivation), so height-fog slices keep the closed form. σ→0 takes the `S·Δs` limit.
3. **Reproject** stays absent on the GPU (ADR 0026 decision 5). No previous-volume buffer is allocated.
4. **Compose** keeps `background · T + L` and the current trilinear read; tricubic upsampling is a separate, measured change.
5. **Grid sizes** take the ADR 0026 table (Low 64×36×32 … Ultra 240×135×96) through the existing settings; zero keeps the preset.

## Cost

| Preset | Froxels | Model ms | VRAM |
|---|---|---|---|
| Low, GTX 1060 6 GB, 1080p | 73,728 | 0.9 (0.6 with 1 ray) | 2 MB |
| Medium, 4070 1440p | 248,832 | 0.30 | 5 MB |
| High, 4070 1440p | 921,600 | 0.45 | 15 MB |
| Ultra, 4070 1440p | 3,110,400 | 0.80 | 40 MB |

Models from plan.md, not measurements. Shadow rays are about half the cost. All within 4 GB and the ~200 MB lighting budget.

## Test plan and gates

1. **GPU vs CPU (lavapipe, CI):** extend `froxel_gpu.rs` to run the five F0 scenes (including the window shaft) at the Low grid and compare each column to `fog-ref`'s `froxel_frame`: max relative error ≤ 1% where L > 1e-3, else absolute ≤ 1e-5. Shadow rays use the same jitter on both, so the comparison is deterministic.
2. **Low shaft margin:** `fog_ref_err_near` on the GPU result ≤ 0.05 (CPU had 0.0483). If it slips, next is 3×3 reuse, then height-fog-only, per plan.md.
3. **No history:** a dark frame after a lit frame composes 0 fog radiance.
4. **Still-camera flicker (lavapipe):** two frames, same camera: byte-identical fog columns.
5. **Density zero (lavapipe):** no fog dispatch is recorded.
6. **Cost (4070):** fog ms per preset ≤ the table; basic scene at High ≥ 500 fps with fog on. **1060 Low** ms is a model until a 1060 is available.

## Open choices for Anthony

- Review ADR 0026 so F2-GPU (and F3–F5) can land on lighting-v3.
- Land F2-GPU before or after ReSTIR C5 passes its gates (proposed: after, since lighting-v3 timings are still moving).
