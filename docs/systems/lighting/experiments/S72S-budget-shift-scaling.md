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

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Anthony 14:03: must scale down to GTX 1060; nothing may depend on a minimum ray count.

## Lessons / when to revisit
not recorded
