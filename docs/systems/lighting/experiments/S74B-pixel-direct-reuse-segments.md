# S74B: Pixel direct light reused when point unchanged and no change bound touches its shadow segments

- **Date:** 2026-10-12
- **Result:** reverted (6a0debae)
- **Commit / patch:** bf0f2d2b

## Idea
Pixel direct light reused when point unchanged and no change bound touches its shadow segments

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
not recorded

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Reverted for the Lumen direct-light plan; a screen read of texels would blur shadow edges.

## Lessons / when to revisit
not recorded
