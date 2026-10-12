# S72S: Budgets scale by GENOS_LIGHTING_SC_BUDGET_SHIFT; MSME warm-up

- **Date:** 2026-10-12
- **Result:** pushed
- **Commit / patch:** ec3d5e74

## Idea
Budgets scale by GENOS_LIGHTING_SC_BUDGET_SHIFT; MSME warm-up

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
See README section 'Fill speed' entry S72S.

## Numbers
S72R pop 2.8% at f20-80 vs 0.15%.

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Anthony 14:03: must scale down to GTX 1060; nothing may depend on a minimum ray count.

## Lessons / when to revisit
not recorded
