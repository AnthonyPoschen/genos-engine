# S72K: Smooth cross-page bar, count-based handover

- **Date:** 2026-10-12
- **Result:** pushed (superseded)
- **Commit / patch:** b802fa3b

## Idea
Smooth cross-page bar, count-based handover

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
See README section 'Fill speed' entry S72K.

## Numbers
not recorded

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Video showed 8x8 pages popping as squares.

## Lessons / when to revisit
not recorded
