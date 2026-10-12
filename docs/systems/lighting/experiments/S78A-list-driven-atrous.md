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

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Cost is the active texels (atomic order, worse locality).

## Lessons / when to revisit
not recorded
