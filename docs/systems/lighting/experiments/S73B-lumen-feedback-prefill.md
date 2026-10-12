# S73B: All scout hits claim fine texels and stamp LastUsed

- **Date:** 2026-10-12
- **Result:** reverted (8b62829b); kept on gi-v2-prefill (22bcd47f)
- **Commit / patch:** 22bcd47f

## Idea
All scout hits claim fine texels and stamp LastUsed

## Motivation
not recorded

## Paper basis
Lumen surface cache GPU feedback

## What was implemented
not recorded

## Numbers
not recorded

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
not recorded

## Lessons / when to revisit
S-curve off-screen lag bound idea (Anthony 15:55) deferred until spare performance.
