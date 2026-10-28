# Specular and reflections in lighting-v3

Status: **Proposed** with [ADR 0024](../../adr/0024-lighting-v3-specular.md). Architecture: [ADR 0022](../../adr/0022-lighting-v3-compute-architecture.md). Mirrors, portals, water and preset caps: [ADR 0019](../../adr/0019-reflections.md), [reflections design](../reflections-design.md). Harness: [oracle harness](oracle-harness.md). Terms: [glossary](../../lighting/GLOSSARY.md).

## Goals

1. Glossy and rough specular match the path-traced reference near the camera (`spec_err_near`, `ref_err_near`).
2. No pops: not at the screen edge, not across roughness bands, not on a light switch.
3. Basic scene ≥ 500 fps at 1440p on the RTX 4070 with specular on; GTX 1060 Low holds its 0.40 ms cap at 1080p with zero rays.
4. One path per roughness band, every input a finished value; no radiance history.

## Non-goals

Mirrors, portals and water (ADR 0019, unchanged). Anisotropy, clearcoat, glTF specular extensions. Hit lighting. A second light cache.

## Pipeline (per frame, after direct light and card re-light, before compose)

1. **Classify** (8×8 tiles): roughness from the G-buffer. Tiles with no pixel below 0.65 skip steps 2–4. Output tile lists for march, fallback and rough-only.
2. **Rough lobe** (`r ≥ 0.55`): read the DDGI probe radiance in the reflected direction, pre-integrated with the split-sum DFG table; weight blends to the glossy result over `0.55–0.65`.
3. **Screen march** (`r < 0.65`): hierarchical-Z march along `reflect(I, N)` against this frame's depth. Hit colour is this frame's lit colour before compose (direct + card + probe), so a light change appears in specular the same frame. Confidence `c_ss = edge_fade × thickness_fade × (1 − smoothstep(0.45, 0.65, r))`.
4. **World fallback** for `1 − c_ss`:
   - RT GPU with `ray_query` enabled: one ray, hit shaded from the card atlas texel at the hit (finished value).
   - Otherwise: software SDF trace up to 8 m (Low: 0 rays, skip directly to probe), hit read from cards.
   - Miss or past range: DDGI probe radiance in the reflected direction, Chebyshev-weighted.
5. **Filter**: one spatial pass, radius from roughness and hit distance; no temporal history.
6. **Compose**: `spec = F(n·v, F0) × (c_ss · L_ss + (1 − c_ss) · L_world)` with F from the DFG table; diffuse is scaled by `1 − F` for dielectrics.

Planar mirrors (`r < 0.02`, planar flag) keep ADR 0019's extra views, blending into the glossy path over `0.02–0.07`.

## Presets

| Preset | March steps | World fallback | Rays | Cap (ADR 0019) |
| --- | --- | --- | --- | --- |
| Low (GTX 1060) | 16, half length | probes only | 0 | 0.40 ms, 8 MB |
| Medium | 32 | SDF → cards → probes | software | 0.70 ms, 16 MB |
| High (RTX 4070) | 32 | `ray_query` → cards → probes | 1/glossy px | 1.00 ms, 32 MB |

Basic scene budget on the 4070 at 1440p: specular ≤ 0.35 ms so the frame stays ≤ 2 ms.

## Reference comparison test (lands first, stage S0)

Goes to master as harness work, then is merged into `lighting-v3`.

**Scenes** (added to `make lighting-oracle`, same fixed 1/60 s step and HIST = 30 rewind):

| Scenario | Content | Frames |
| --- | --- | --- |
| `gloss-sweep` | Basic room; floor and five spheres at roughness 0.05, 0.2, 0.4, 0.6, 0.9 (metal and dielectric rows) | 60 still |
| `gloss-edge` | Camera pans so a bright reflected lamp crosses the screen edge | 120 |
| `gloss-offscreen` | Glossy floor reflecting a lit wall behind the camera | 60 still |
| `gloss-switch` | Lamp seen only in reflection turns on, then off | 30 + 120 + 120 |
| `gloss-band` | Roughness animated 0.0 → 1.0 over 180 frames on one panel | 180 |

**Metrics** (in `scripts/lighting/oracle_metrics.py`, self-tested in `scripts/lint.sh`):

- `spec_err_near`: `ref_err_near` restricted to pixels where the reference specular share `spec_ref / total_ref ≥ 0.2`. The path tracer writes a specular-only AOV alongside the beauty image to build the mask; reported per roughness row and per scenario.
- `spec_pop`: worst frame-to-frame luminance step in the masked pixels, minus the same step in the reference sequence; must stay below the existing flicker threshold.
- `frames_to_ref` on `gloss-switch`, as for `switch`.
- `spec_ms`: GPU time of steps 1–5 from timestamp queries (lavapipe numbers only compared run against run).

**Self-test:** synthetic image pairs check that the mask selects only specular pixels, that a one-frame spike is caught by `spec_pop`, and that a constant offset scales `spec_err_near` linearly.

**Baseline:** S0 records master and `lighting-v3` numbers in `docs/systems/lighting/history.md`. Later stage gates (ADR 0024) are written against those numbers.

## Failure modes and probes

| Failure | Probe |
| --- | --- |
| Edge pop when screen hit leaves view | `gloss-edge` `spec_pop` |
| Band pop at roughness thresholds | `gloss-band` `spec_pop` |
| Stale reflection after light switch | `gloss-switch` `frames_to_ref` |
| Light leak through walls in fallback | Existing wall-leak tests with glossy floor variant |
| Over-budget on Low | `spec_ms`, `low_reflection_rays` = 0 |

Debug rows (setting keys only): `spec.path` view (screen / card / ray / probe per pixel), `spec.confidence`, `spec.roughness_band`.
