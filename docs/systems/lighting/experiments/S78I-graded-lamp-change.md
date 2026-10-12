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

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
At dusk moving lamps dominate wherever they reach (kept relit 1.08M/frame moving vs 0.50M still).

## Lessons / when to revisit
not recorded
