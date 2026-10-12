# S63-S65: Sun/sky share per texel; rate-based scheduling (batch is a ceiling); two tiers (trickle under change 0.173, whole sets above)

- **Date:** 2026-10-12
- **Result:** pushed
- **Commit / patch:** 0f288334..b5a72df4

## Idea
Sun/sky share per texel; rate-based scheduling (batch is a ceiling); two tiers (trickle under change 0.173, whole sets above)

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
Threshold 0.173 vs no trickle tier: 56% fewer rays per round, +15% fps, flicker 0.0052 -> 0.0030.

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
not recorded

## Lessons / when to revisit
GPU can idle when little changes, unlike Lumen.
