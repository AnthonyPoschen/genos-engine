# 0022: lighting-v3 compute architecture

Status: **Accepted** by the Engine bot under Anthony's delegated design authority. Built on the long-lived `lighting-v3` branch; Anthony reviews the finished branch before it reaches master. Study: [`docs/systems/lighting/compute-rethink.md`](../systems/lighting/compute-rethink.md) (PR #95). Supersedes the convergence machinery of ADR 0021 (proposed, PR #94) except F1, which is kept as the display rule. Keeps ADR 0012 (stored light never depends on the camera) and ADR 0013 (invalidation is local).

## Context

Master spends about 18.6 ms on lighting at 1080p High on the 4070 (direct 8.1, resolve 4.0, à-trous 3.5, trace 2.6, compose 2.1). Cost scales with pixels × lamp candidates × passes, and half of it filters noise the estimator itself created. Texels that are still refining are shown, which draws the sun-cycle checkerboard (lavapipe oracle: `sun-x100` worst grid 0.567) and leaves movers patchy for 3–5 s.

Anthony's goals (28 Oct 2026):
- The primary metric is the cost of a **complete re-light every frame**. Skipping unchanged work is a bonus, never load-bearing.
- The worst case, a light switching on in a dark room, must fit the frame budget.
- Photoreal against the path-traced reference; no flicker and no pops.
- Probes and cache show only finished values and blend once.
- Basic scene about 500 fps at 1440p on the 4070; a Low preset for the GTX 1060; lighting VRAM about 200 MB (soft).

## Decision

1. **Direct light: ReSTIR DI**, 1–2 shadow rays per pixel picked from the light tree, with spatial and temporal reservoir reuse. Cost does not depend on lamp count. A change record (lamp on/off, mover) resets temporal reservoir confidence, so the first frame after a change uses spatial reuse only. Direct light is computed every frame, always.
2. **Near and mid bounce: full card re-light every frame.** Every visible card texel is relit at a fixed texel density each frame. The cost is set by the texel count, not by how much changed, so a full re-light is the normal frame and the light-switch frame costs the same as any other.
3. **Far bounce: a DDGI-style probe hierarchy** (irradiance probes with Chebyshev visibility, fully re-traced every frame at a fixed probe count × rays). Sannikov's 2023 interval-merged probe method (GI v1's retired approach) is not revived; finer probe tiers are a stage E experiment only.
4. **Screen space only for contact detail, specular and compose.** Half resolution only for the probe apply. No neural cache.
5. **Display rule (F1): show finished values only.** What a pixel shows is `mix(coarser finished level, own value, confidence)`; a value that is not finished is never shown alone, and a change blends in once.
6. **Kept possible, not built:** a Low-preset knob that spreads a full world-space bounce re-light over N frames and swaps it in with a short blend. Direct light stays per frame.
7. **Change-driven skipping** (stage H) is a power and headroom bonus. Its gate is image equality with the full re-light.
8. **Glossary:** one meaning per term. "Re-light" means recomputing a stored value from current inputs; "finished" means a value whose confidence for the current light is 1; "confidence" is per value and reset by a change record, never a raw sample count.

## Stage plan and gates

Every gate is a `make lighting-oracle` metric (`docs/systems/lighting/oracle-harness.md`) compared with master on the same machine, plus 4070 frame time where named. A stage that misses its gate is dropped or reworked, never merged to `lighting-v3` on a promise.

| Stage | What | Gate |
| --- | --- | --- |
| B | This ADR | Accepted |
| F1 | Show finished values only (confidence blend with the coarser finished level) | `sun-x100` `grid_worst` below master's (lavapipe 0.567), target ≤ 0.15; `ref_err_near` not worse than master |
| C | ReSTIR DI behind `restir_di` (`--restir-di`, `GENOS_RESTIR_DI`), off by default | `ref_err_near` ≤ master; `grid_worst` ≤ 0.15; direct ≤ 3 ms at 1440p on the 4070 |
| D | Full-coverage card re-light every frame | `spin` settles in ≤ 1 frame; `sun-x100` and `sun-x10` `grid_worst` ≤ 0.15; `switch` `frames_to_ref` ≤ 5; no wall-leak test regressions |
| E | DDGI probe hierarchy for far bounce | wall-leak tests pass; indirect ≤ 3 ms at 1440p on the 4070; `ref_err_near` ≤ stage D |
| F | Remove resolve / à-trous stages no longer needed, one at a time | `ref_err_near` unchanged within noise; lighting ≤ 8 ms p50 stress hall, basic scene ≥ 500 fps at 1440p on the 4070 |
| G | Low preset on a GTX 1060 budget | ≤ 16 ms modelled (measured if hardware is found); lighting VRAM ≤ 200 MB |
| H | Bonus: change-driven skipping | images match the full re-light (bit-exact or within ε) |

The worst case is gated directly: the `switch` frame's lighting ms must be within the frame budget and within 10% of a steady frame's.

## Known bugs the rewrite must fix

Each has an oracle check; the branch is not offered for master until all four pass.

1. **Brightness 0.68 of the reference.** Live light is about 0.68× the path-traced reference. Gate: mean live/reference luminance ratio within 0.95–1.05 on every scenario, or the gap explained and the reference fixed.
2. **Stale light from removed lamps.** Light from a lamp that was turned off or deleted lingers. Gate: within 5 frames of removal, `ref_err_near` is back within 1.1× settled.
3. **A room never goes dark when all lamps are off.** The oracle shows about 70% of the lit picture still present 60 frames after the switch-off. Gate: the dark frame of `switch` matches the dark reference (`ref_err_near` within 1.1× settled) within 5 frames.
4. **Turn edge pops.** Gate: `turn` `grid_worst` ≤ 0.15 and no frame-to-frame luminance step above the flicker threshold at the leading edge.

## Rejected

- More rays or a bigger budget on today's pipeline: full effort still grids (0.58) and costs more.
- Half-resolution lighting as the main path (C4): halos near the camera; ADR 0013 already rejected half-res denoise.
- Neural radiance cache (C5): not gradeable, non-deterministic, not viable on the 1060.
- Screen-space blur of the grid: a look hack that fails the reference check.

## Consequences

- `lighting-v3` diverges from master for weeks; fixes to master lighting are ported only if trivial.
- The ms numbers in the study are models until each stage measures them.
- Stage PRs target `lighting-v3`; harness and metrics changes still go to master.
