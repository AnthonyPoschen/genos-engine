# S78N: Pixel reuse in frames with changes (segment test) + 1/16 verify

- **Date:** 2026-10-12
- **Result:** rejected
- **Commit / patch:** g78/S78N.patch

## Idea
Pixel reuse in frames with changes (segment test) + 1/16 verify

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
10.17/86, 19.1/50, pop 0.074/0.112

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Verify mismatched often (still light 0.9 -> 2.8); moving lamps relight anyway.

## Lessons / when to revisit
Retest: this was judged before the still-scene churn (surface cache still converging, waking probes every frame; see [counters](S78S-counters-still-churn.md)) was found. Re-run on the build that fixes it.
