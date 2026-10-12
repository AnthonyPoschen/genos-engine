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

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Trickle starved converging texels; G2 pop +40-50% for 0.09 ms.

## Lessons / when to revisit
Retest: this was judged before the still-scene churn (surface cache still converging, waking probes every frame; see [counters](S78S-counters-still-churn.md)) was found. Re-run on the build that fixes it. The planned IC-style stop rule supersedes this.
