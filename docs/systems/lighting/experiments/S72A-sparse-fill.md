# S72A: Only key texels get rays; others bilinear from keys

- **Date:** 2026-10-12
- **Result:** pushed (superseded by S72V)
- **Commit / patch:** d38d58ec

## Idea
Only key texels get rays; others bilinear from keys

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
See README section 'Fill speed' entry S72A.

## Numbers
Due backlog 35-50% lower than S71G; fps unchanged.

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
not recorded

## Lessons / when to revisit
not recorded
