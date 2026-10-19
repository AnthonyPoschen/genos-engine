# S72Q: Layer-2 transmittance stops

- **Date:** 2026-10-12
- **Result:** pushed (superseded)
- **Commit / patch:** 61df885c

## Idea
Layer-2 transmittance stops

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
See README section 'Fill speed' entry S72Q.

## Numbers
S72P pop 0.33%/0.72% vs S72O 0.11%/0.19%; S72P used 19-25% more rays than S72O still.

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
not recorded

## Lessons / when to revisit
not recorded
