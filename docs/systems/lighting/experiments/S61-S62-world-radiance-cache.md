# S61-S62: World radiance cache (WRC, 25 cm cells) for hits; variance-guided denoise; WRC refresh budget; global light rescale (S62E, reverted S62F)

- **Date:** 2026-10-12
- **Result:** pushed (S62E reverted)
- **Commit / patch:** aa1b245e..4eb10b86

## Idea
World radiance cache (WRC, 25 cm cells) for hits; variance-guided denoise; WRC refresh budget; global light rescale (S62E, reverted S62F)

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
S62F: global rescale dimmed lamp-bounced light at night.

## Lessons / when to revisit
not recorded
