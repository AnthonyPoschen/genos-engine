# S78A: List-driven a-trous (indirect dispatch over active texels)

- **Date:** 2026-10-12
- **Result:** rejected (stash)
- **Commit / patch:** git stash in ge-merge

## Idea
List-driven a-trous (indirect dispatch over active texels)

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
still 21.9/44 (atrous 4.1 vs 3.5), moving 28.1/35 (8.7 vs 6.1)

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Cost is the active texels (atomic order, worse locality).

## Lessons / when to revisit
not recorded
