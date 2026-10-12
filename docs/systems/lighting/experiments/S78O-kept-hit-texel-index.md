# S78O: Kept hits remember their texel

- **Date:** 2026-10-12
- **Result:** rejected
- **Commit / patch:** g78/S78O.patch

## Idea
Kept hits remember their texel

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
8.39/103, 18.7/51 (light 6.59 vs 6.65), pop 0.069/0.104; +16 MiB

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Lookup was ~0.06 ms; cost is the lighting itself.

## Lessons / when to revisit
not recorded
