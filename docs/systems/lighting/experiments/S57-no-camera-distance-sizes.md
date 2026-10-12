# S57: One texel size (12.5 cm pixels, 25 cm hit patches) at every distance

- **Date:** 2026-10-12
- **Result:** pushed (decision, Anthony)
- **Commit / patch:** 1dbbca64

## Idea
One texel size (12.5 cm pixels, 25 cm hit patches) at every distance

## Motivation
not recorded

## Paper basis
Lumen cards keep resolution through camera moves; no surveyed paper keys cell size to the eye

## What was implemented
not recorded

## Numbers
not recorded

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Band rings followed the camera and showed coarse grain while running.

## Lessons / when to revisit
Rule: camera decides residency and update order only, never cell size.
