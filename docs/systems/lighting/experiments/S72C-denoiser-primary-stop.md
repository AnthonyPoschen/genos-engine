# S72C: Denoiser as primary filter + stop on denoised noise

- **Date:** 2026-10-12
- **Result:** reverted (S72G)
- **Commit / patch:** 4c23e3e0 / ca01a371

## Idea
Denoiser as primary filter + stop on denoised noise

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
See README section 'Fill speed' entry S72C.

## Numbers
not recorded

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Baked low-frequency blotches; stopped texels while error was low-frequency.

## Lessons / when to revisit
not recorded
