# S73S: Accumulation reads raw means; display keeps dark fill

- **Date:** 2026-10-13 (closed 2026-10-19)
- **Result:** closed without merge (misses reveal target; error grows)
- **Commit / patch:** PR [#25](https://github.com/AnthonyPoschen/genos-engine/pull/25), branch `cursor/unfilled-prefill-bias-8bfe`

## Idea
Split surface-cache feedback so accumulation reads raw means (with a coarser-level blend) while the displayed picture keeps the dark fill for unfilled texels.

## Motivation
Camera-turn pre-fill looked about **1.5× too bright** on the 4070. The display dark fill was feeding back into accumulation, so new texels inherited an inflated mean.

## What was implemented
- Feedback / accumulation reads **raw means**, blended toward a **coarser level** with **k=2**.
- **Display** still uses dark fill for unfinished texels (visual continuity only).
- Unfilled-on-screen detector work stayed on PR [#29](https://github.com/AnthonyPoschen/genos-engine/pull/29) (split out; already merged).

## Numbers
Lavapipe (no path-traced reference on the 4070 for this trial):

| Metric | S73S | S73R (prior) | Target |
|---|---|---|---|
| Reveal error | 0.168 | 0.313 | 0.10 |
| Reveal +8 frames | 0.236 | 0.329 | 0.10 |
| Still image vs former gi-v2 tip | 3.6% darker overall; **8.4%** darker in the dark quartile | — | — |

Better than S73R on reveal, but still above the 0.10 target, and the +8-frame error **grows** (0.168 → 0.236).

Numbers context: lavapipe cloud checks only; no path-traced A/B on the 4070 for S73S.

## Screenshots
PR #25 (agent evidence); not copied into this tree.

## Outcome and why
Closed. Misses the reveal error target; the residual grows after the reveal rather than settling. A plausible real cause is the **16-set freeze** diagnosed around PR [#3](https://github.com/AnthonyPoschen/genos-engine/pull/3) (stuck / never-moved texels), not only display-fill feedback. Shipping S73S would bake a darker bias without fixing convergence.

## Lessons / when to revisit
Keep **display-only** tweaks out of accumulation. Retry as a **small fresh experiment** against the **path-traced reference on the 4070**, after the current perf PR stack lands, and only once the 16-set freeze / stuck-patch path is understood. The unfilled detector (#29) remains the observability tool for that retry.
