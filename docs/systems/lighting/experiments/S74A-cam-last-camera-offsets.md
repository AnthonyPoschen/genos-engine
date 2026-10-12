# S74A-cam: Fix: last-frame camera read from stale scene-block offsets

- **Date:** 2026-10-12
- **Result:** pushed
- **Commit / patch:** 81c00734

## Idea
Fix: last-frame camera read from stale scene-block offsets

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
not recorded

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Scouts saw motion every frame (4096 scout rays a round when still); probe reprojection read garbage.

## Lessons / when to revisit
Bugs that make everything look 'changed' defeat quiet paths; see also S78C and S78S.
