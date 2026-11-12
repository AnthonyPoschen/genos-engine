# 0024: Specular and reflections in lighting-v3

Status: **Proposed** (Engine bot, 28 Oct 2026). Targets the `lighting-v3` branch; Anthony reviews the finished branch before master. Design: [`docs/systems/lighting/specular-v3-design.md`](../systems/lighting/specular-v3-design.md). Builds on [ADR 0022](0022-lighting-v3-compute-architecture.md) (decision 4: screen space for contact detail, specular and compose). Keeps [ADR 0019](0019-reflections.md) for mirrors, portals, water and the per-preset reflection caps, and re-points its glossy fallback at the lighting-v3 stores.

## Context

ADR 0019 chose a screen march, then a world-cache read, then an optional ray, reading the old surface cache and screen probes. ADR 0022 replaces those stores: cards are fully re-lit every frame, far bounce lives in DDGI-style probes, and only finished values are shown (F1). The specular path has to read the new stores, keep the 0019 caps, and be graded with the same oracle as the rest of lighting-v3. No specular scenario exists in `make lighting-oracle` today, so glossy error is invisible to every 0022 gate.

## Decision

1. **Reference test lands first (stage S0).** Add a `gloss` scenario family to `make lighting-oracle` and a `spec_err_near` metric (specular-only error near the camera) before any shader change. Master and `lighting-v3` are measured with it so every later stage has a baseline. Details in the design.
2. **Roughness selects the path, per pixel, with blended edges.** `r < 0.02` on a planar surface: ADR 0019 planar view. `r < 0.6`: glossy band (steps 3–5). `r ≥ 0.6`: no specular trace; the specular lobe is read from the probe irradiance with a split-sum term. Each boundary has a 0.05 roughness blend band so a material edit or LOD change cannot pop.
3. **Screen space first, for contact detail.** One hierarchical-Z march along `reflect(I, N)` (Medium/High: up to 32 steps; Low: 16 steps on half-length). A hit reads the current frame's lit colour (pre-compose), never last frame's. Confidence falls with distance to the screen edge, ray thickness and roughness.
4. **World fallback: cards, then probes.** Where screen confidence < 1, the remainder is filled from a world lookup along the same direction: the card atlas hit (the software SDF tracer on every GPU, short range ≤ 8 m) and, past that or on a miss, the nearest DDGI probe's radiance in that direction. Cards and probes are finished values, so the fallback inherits F1 and needs no history.
5. **Optional `VK_KHR_ray_query` path.** On RT GPUs (High, and Medium when enabled) the card lookup is replaced by one hardware ray per glossy pixel that shades its hit from the card atlas (no hit lighting, per ADR 0019). Same inputs and output as decision 4, so the switch is a quality knob, not a separate pipeline. Off on Low; macOS uses the software tracer.
6. **No radiance history.** As in ADR 0019, the specular term stores a hit, re-shaded in the same frame; one spatial filter, radius from roughness. A light switch is visible in specular in the same frame it reaches cards and direct light.
7. **Budgets.** Specular total at 1440p on the RTX 4070: ≤ 0.35 ms on the basic scene (keeps 500 fps with ADR 0022's lighting budget), ≤ 1.0 ms High in the stress hall. GTX 1060 Low at 1080p: ≤ 0.40 ms (ADR 0019's Low cap), zero rays, 8 MB.

## Gates

Each is a `make lighting-oracle` metric against master on the same machine, plus 4070 ms where named. A stage that misses is dropped or reworked.

| Stage | What | Gate |
| --- | --- | --- |
| S0 | `gloss` scenarios + `spec_err_near`, `spec_pop` metrics, self-tested | Lands on master (harness); baselines recorded for master and `lighting-v3` |
| S1 | Roughness classify + rough-lobe probe read | `spec_err_near` (rough rows) ≤ master; `spec_pop` ≤ flicker threshold across the 0.6 edge; ≤ 0.05 ms basic scene |
| S2 | Screen march with confidence | `spec_err_near` (glossy-near rows) ≤ 0.7 × master; `grid_worst` ≤ 0.15; `turn` `spec_pop` ≤ threshold |
| S3 | Card/probe fallback | `spec_err_near` (off-screen rows) ≤ 0.7 × master; `switch` specular `frames_to_ref` ≤ 1 after card frames_to_ref |
| S4 | `ray_query` path | `spec_err_near` ≤ S3 on the 4070; no regression with the path disabled |
| S5 | Budgets | 4070 1440p: specular ≤ 0.35 ms basic, ≤ 1.0 ms stress High, basic ≥ 500 fps; GTX 1060 Low ≤ 0.40 ms (modelled if no hardware), 0 rays, VRAM ≤ 8 MB |

`ref_err_near` overall must never rise at any stage.

## Rejected

- Temporal accumulation of reflection radiance: lag and ghosts, refused by ADR 0012/0019.
- Stochastic multi-lobe rays as the main gather: noise needs a denoiser with history.
- Last-frame colour for screen hits: one frame of stale light after a switch, which fails the `switch` gate.
- Pre-filtered cubemap probes as a separate store: a second light cache; DDGI probes already carry far radiance.
- Hard roughness cut-offs: pop on material LOD and edits.

## Consequences

- S0 is harness work and goes to master; S1–S5 target `lighting-v3`.
- ADR 0019's glossy fallback text now means "cards then DDGI probes" on `lighting-v3`; mirrors, portals, water and caps are unchanged.
- The ms figures are budgets, not measurements, until S5.
