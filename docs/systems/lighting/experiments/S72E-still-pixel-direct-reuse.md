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

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
No light-pass gain in the still run, picture shifted. (Later S78J did this successfully after S78C fixed probe reprojection.)

From Chief's notes: listed among Engine's reverts not decided with Chief [uncertain who decided].

## Lessons / when to revisit
not recorded
