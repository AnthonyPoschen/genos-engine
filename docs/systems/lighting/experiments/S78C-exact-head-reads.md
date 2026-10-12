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

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
shad-S78C a/b (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
7 of 8 strata refilled every frame (3.6M fill hits still).

## Lessons / when to revisit
Biggest single win; another 'always changed' bug.
