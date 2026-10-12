# S56: Texels at a new camera-distance size inherit converged light with full count; 10% hysteresis

- **Date:** 2026-10-12
- **Result:** superseded by S57 same day
- **Commit / patch:** 0624ee53

## Idea
Texels at a new camera-distance size inherit converged light with full count; 10% hysteresis

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
Approach grain 0.0059 -> 0.0052, recovery 71 -> 49 frames, end 0.0175 -> 0.0142; trail 0.0372 -> 0.0421.

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Anthony still saw a ring where quality dropped; band edges follow the camera.

## Lessons / when to revisit
Do not key cache cell size to the eye.
