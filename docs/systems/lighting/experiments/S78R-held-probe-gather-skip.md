# S78R: Gather skip for held probes

- **Date:** 2026-10-12
- **Result:** rejected
- **Commit / patch:** g78/S78R.patch

## Idea
Gather skip for held probes

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
8.26-8.43/102-105 (gather 0.84-0.92 vs 0.91), moving 19.2/52, pop 0.077/0.127; +1 MiB

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Few probes qualified: kept light changes most frames.

## Lessons / when to revisit
Led to the churn investigation. Retest: this was judged before the still-scene churn (surface cache still converging, waking probes every frame; see [counters](S78S-counters-still-churn.md)) was found. Re-run on the build that fixes it.
