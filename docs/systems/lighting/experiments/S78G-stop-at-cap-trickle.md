# S78G: Stop at cap + 64/round trickle for unconverged (G2: stop at cap only)

- **Date:** 2026-10-12
- **Result:** rejected
- **Commit / patch:** g78/S78G.patch

## Idea
Stop at cap + 64/round trickle for unconverged (G2: stop at cap only)

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
G: 11.07/82, 18.3/52, brightness a .318 vs .348 (-9%). G2: 11.11/81, 18.6/51, pop 0.102/0.167

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Trickle starved converging texels; G2 pop +40-50% for 0.09 ms.

## Lessons / when to revisit
Retest: this was judged before the still-scene churn (surface cache still converging, waking probes every frame; see [counters](S78S-counters-still-churn.md)) was found. Re-run on the build that fixes it. The planned IC-style stop rule supersedes this.
