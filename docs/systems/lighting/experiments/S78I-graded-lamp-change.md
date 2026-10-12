# S78I: Lamp change graded by its bound vs shown light

- **Date:** 2026-10-12
- **Result:** rejected
- **Commit / patch:** g78/S78I.patch

## Idea
Lamp change graded by its bound vs shown light

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
11.45/78, 18.9/50; +2% brighter

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
At dusk moving lamps dominate wherever they reach (kept relit 1.08M/frame moving vs 0.50M still).

## Lessons / when to revisit
not recorded
