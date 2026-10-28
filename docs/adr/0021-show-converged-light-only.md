# 0021: Show converged light only; the sun is a global coarse update

Status: **Proposed**. Design: [`docs/systems/lighting/convergence-and-frame-cost-design.md`](../systems/lighting/convergence-and-frame-cost-design.md). Extends ADR [0013](0013-lighting-invalidation-is-local.md).

## Context

On a 4070, light that is still refining shows per texel:
- movers stay patchy for 3–5 s;
- sunrise and sunset draw a cell-grid checkerboard (worst grid score 0.54 at 100x and 0.39 at 10x on the full-day sweep);
- camera turns pop at the edges.

Lighting costs 18.6 ms (about a 50 fps cap), and only 6 ms of it is budgeted.

## Decision

1. A pixel shows `mix(coarser converged prior, texel, confidence)`, where confidence is reset by a change record and grows with sets taken under the current light. A texel that is not finished is never shown alone.
2. A sun move relights the coarse levels as a whole in the same frame. Fine texels hold their sun bounce as a ratio to the coarse cell. The banded sun amortiser is removed.
3. Refresh priority is screen coverage × change / camera distance. Near-camera, low-confidence texels skip the refinement thinning gates.
4. The guard band follows the rotation, wide on the leading side.
5. The gates are `scripts/lighting/sun_cycle_grid.sh` (worst grid ≤ 0.15 at 100x, 10x and 1x over a full day; a mover settles within 30 frames) plus a 4070 lighting cost of ≤ 8 ms p50 at 1080p High.

## Rejected

- More rays or a bigger budget: full effort still grids, at 0.58.
- Screen-space blur of the grid: a look hack, and it fails the reference check.
- Hiding stale texels by holding the old frame: it shows values that are not finished, and it lags.
