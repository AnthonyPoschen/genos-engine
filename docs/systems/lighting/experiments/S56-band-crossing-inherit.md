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

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Anthony still saw a ring where quality dropped; band edges follow the camera.

## Lessons / when to revisit
Do not key cache cell size to the eye.
