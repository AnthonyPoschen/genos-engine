# S73A: Guard-band scout rays claim fine texels just outside the frustum

- **Date:** 2026-10-12
- **Result:** reverted (11639205); kept on gi-v2-prefill
- **Commit / patch:** 097d67de

## Idea
Guard-band scout rays claim fine texels just outside the frustum

## Motivation
Anthony 15:50: turning/walking backwards shows unfilled texels.

## Paper basis
Lumen ray-hit feedback; guard band (Bavoil & Sainz 2009; Mara et al. 2016)

## What was implemented
not recorded

## Numbers
not recorded

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Reverted to the checkpoint tree for S74 cost cuts; reason beyond that not recorded.

## Lessons / when to revisit
Bring back once there's spare budget. Was affected by the scout camera bug (S74A).
