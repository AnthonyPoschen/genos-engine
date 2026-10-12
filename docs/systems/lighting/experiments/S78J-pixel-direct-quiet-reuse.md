# S78J: Per-pixel direct reuse in quiet frames

- **Date:** 2026-10-12
- **Result:** pushed
- **Commit / patch:** 18894551

## Idea
Per-pixel direct reuse in quiet frames

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
still 9.42/93 (light 1.56), moving 18.8/50, pop 0.088/0.115; +7.9 MiB

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
not recorded

## Lessons / when to revisit
Any mover relights all pixels.
