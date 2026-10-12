# S78H: WRC growth trickle 1024/round

- **Date:** 2026-10-12
- **Result:** rejected
- **Commit / patch:** not saved

## Idea
WRC growth trickle 1024/round

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
12.9/71, 19.6/48; brightness a .332

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Darker and slower.

## Lessons / when to revisit
Retest: this was judged before the still-scene churn (surface cache still converging, waking probes every frame; see [counters](S78S-counters-still-churn.md)) was found. Re-run on the build that fixes it.
