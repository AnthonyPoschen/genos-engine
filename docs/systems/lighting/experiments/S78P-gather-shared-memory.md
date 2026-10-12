# S78P: Gather bins in shared memory, one workgroup per probe

- **Date:** 2026-10-12
- **Result:** pushed
- **Commit / patch:** 8c763bb2

## Idea
Gather bins in shared memory, one workgroup per probe

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
still 8.21/105 (gather 0.91 vs 1.25), moving 18.7/51 (0.95 vs 1.62); no VRAM

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
shad-S78P/b (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
not recorded

## Lessons / when to revisit
not recorded
