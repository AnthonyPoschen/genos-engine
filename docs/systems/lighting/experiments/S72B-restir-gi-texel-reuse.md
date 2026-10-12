# S72B: ReSTIR GI reservoir reuse on texels

- **Date:** 2026-10-12
- **Result:** reverted (S72H)
- **Commit / patch:** b0e76a3b / 43690446

## Idea
ReSTIR GI reservoir reuse on texels

## Motivation
not recorded

## Paper basis
ReSTIR GI

## What was implemented
See README section 'Fill speed' entry S72B.

## Numbers
not recorded

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Neighbour reuse without visibility spread correlated low-frequency blotches.

## Lessons / when to revisit
not recorded
