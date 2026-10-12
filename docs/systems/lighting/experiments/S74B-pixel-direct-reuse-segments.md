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

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Reverted for the Lumen direct-light plan; a screen read of texels would blur shadow edges.

## Lessons / when to revisit
not recorded
