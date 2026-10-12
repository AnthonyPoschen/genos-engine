# S72E: Still frames reuse per-pixel direct light

- **Date:** 2026-10-12
- **Result:** reverted (S72F)
- **Commit / patch:** c35f8927 / d53603d1

## Idea
Still frames reuse per-pixel direct light

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
See README section 'Fill speed' entry S72E.

## Numbers
not recorded

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
No light-pass gain in the still run, picture shifted. (Later S78J did this successfully after S78C fixed probe reprojection.)

## Lessons / when to revisit
not recorded
