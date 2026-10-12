# S78C: Float +0.5 rounding killed probe reprojection

- **Date:** 2026-10-12
- **Result:** pushed (bug fix)
- **Commit / patch:** c660c0db

## Idea
Float +0.5 rounding killed probe reprojection

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
still 12.5/72, moving 18.3/52

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
shad-S78C a/b (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
7 of 8 strata refilled every frame (3.6M fill hits still).

## Lessons / when to revisit
Biggest single win; another 'always changed' bug.
