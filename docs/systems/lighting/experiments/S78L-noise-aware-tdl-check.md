# S78L: Noise-aware TDL check (sigma EMA)

- **Date:** 2026-10-12
- **Result:** rejected
- **Commit / patch:** g78/S78L.patch

## Idea
Noise-aware TDL check (sigma EMA)

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
8.30/103, 18.7/51, pop 0.076/0.117; +16 MiB

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
No compute gain for 16 MiB.

## Lessons / when to revisit
Retest: this was judged before the still-scene churn (surface cache still converging, waking probes every frame; see [counters](S78S-counters-still-churn.md)) was found. Re-run on the build that fixes it.
