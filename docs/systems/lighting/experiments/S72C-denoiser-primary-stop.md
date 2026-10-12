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

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Baked low-frequency blotches; stopped texels while error was low-frequency.

## Lessons / when to revisit
not recorded
