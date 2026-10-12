# S72P: Two layers: traced data (layer 1) never shown; shown field (layer 2) rebuilt per round

- **Date:** 2026-10-12
- **Result:** pushed (core design; layer 2 later SVGF in S72V)
- **Commit / patch:** f582015e

## Idea
Two layers: traced data (layer 1) never shown; shown field (layer 2) rebuilt per round

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
See README section 'Fill speed' entry S72P.

## Numbers
not recorded

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Anthony 13:48 design.

## Lessons / when to revisit
Principle: layer 2 never feeds back into layer 1.
