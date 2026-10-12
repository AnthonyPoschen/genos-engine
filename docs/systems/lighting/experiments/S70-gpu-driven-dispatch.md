# S70: Indirect dispatch for cache rays (S70A), no GPU->CPU counter reads in frame loop (S70B), resident scene buffer with delta upload (S70C)

- **Date:** 2026-10-12
- **Result:** pushed
- **Commit / patch:** 8758d467, 76245067, b0675659

## Idea
Indirect dispatch for cache rays (S70A), no GPU->CPU counter reads in frame loop (S70B), resident scene buffer with delta upload (S70C)

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
not recorded

## Lessons / when to revisit
not recorded
