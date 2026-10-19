# WORKCUT: Moving-scene lighting work reduction (PR #24)

- **Date:** 2026-10-13 (design); closed 2026-10-19
- **Result:** rejected / superseded (not pushed as a unit)
- **Commit / patch:** PR #24 tip `878aa141` on `cursor/lighting-work-reduction-design-d9d4` (closed; branch deleted). Sun amortisation landed separately via PR #36. Work-cut phases 3–7 never entered the tree.

## Idea

Cut moving-frame lighting (~14.5 ms / ~93% lighting on the owner's 1080p window) by splitting **full effort** vs **settled** regimes, sparse probe updates for moving lights, and a **small sun step** that amortises indirect over bands instead of dirtying every compose block and the a-trous field.

## Motivation

PR #26 showed the day-clock sun moves every frame, so every 8×8 compose block and the a-trous dirty fraction stay hot; list/block skipping cannot help until a small sun step exists. Broader moving cost is dominated by light / a-trous / compose (see dynamic-perf plan and the work-reduction design on the PR branch).

## Paper basis

Departures named on the PR: SVGF (Schied et al., HPG 2017) and Lumen (Wright et al., SIGGRAPH 2022) recompute every pixel every frame; a small sun step does not. Wake design cites the approved WAKE note. Confirmed-visibility shade (phase 2) was a local invention and failed.

## What was implemented

On the PR branch only (never merged as a stack):

| Phase | Intent | Status on PR tip |
|---|---|---|
| 0 | Full-effort picture floor (lavapipe) | landed on branch |
| 1 | Regime tags without changing shade; fringe tube on regime view only | landed on branch |
| 2 | Confirmed-visibility shade | **failed picture; reverted** |
| Sun (dep. 14) | Amortise small directional step over bands; direct every frame | iterated on branch; **landed on master via PR #36** (`lighting_sun_compose`, SUN experiment) |
| 3–7 | Sparse probes, first-bounce delta, etc. | **not in the tree** |

Also on the branch: `bounce_corner` script, `lighting_work` lavapipe harness, `work-reduction-design.md`. Stale `gi-v2` / radiance-cascade docs were still present on the tip and must not return to master.

## Numbers

No exclusive still/moving A/B of the full PR tip vs post-#36 master in this window (merge hit 12 conflict files against the evolved sun/compose path; standing rule: do not spend long untangling).

Prior notes on the PR / SUN experiment:

- Lavapipe sun-step checks: small step kept compose list-sized; 40° jump full compose; direct |ΔL| 0 vs full-effort replay of the step (see `SUN-small-sun-step.md`).
- Early 4070 look at an intermediate tip: moving compose rose (regime walk on full compose) before `2a0f07d` restored base shade — no millisecond win claimed for the work cut itself.
- Default moving stress never takes the sun path (boxes/lamps move).

## Outcome and why

**Closed without merging the PR tip.** The only productised piece (small sun) is already on master through #36. The unique remainder is an incomplete design (phase 2 reverted, 3–7 absent) plus a harness/docs stack that does not cleanly merge onto current master. Standing rule (Anthony 19 Oct): do not implement a conflict fix on a measured-or-superseded PR — research note, close, delete branch.

## Lessons / when to revisit

- Land sun amortisation as its own PR (done: #36) before stacking regime/sparse-probe phases.
- Phase 2 shade must beat a zero direct floor on the first moving frame before any further work-cut phases.
- Revisit only with a fresh branch from current master, one phase per PR, exclusive or clean-GPU A/B on still + moving + sun-only hold (`--still-boxes --dynamic 0`).
