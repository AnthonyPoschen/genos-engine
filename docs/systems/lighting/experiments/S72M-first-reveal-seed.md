# S72M: Seed first-reveal texels from probe light

- **Date:** 2026-10-12
- **Result:** pushed
- **Commit / patch:** 3db494ca

## Idea
Seed first-reveal texels from probe light

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
See README section 'Fill speed' entry S72M.

## Numbers
not recorded

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Screen-aligned tearing on turn side (probe/texel seam).

## Lessons / when to revisit
not recorded
