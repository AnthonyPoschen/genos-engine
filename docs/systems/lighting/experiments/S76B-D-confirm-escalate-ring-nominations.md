# S76B-D: TDL and WRC confirm-then-escalate; persistent ring nominations, fair service, global check boost

- **Date:** 2026-10-12
- **Result:** pushed
- **Commit / patch:** dbb4777a

## Idea
TDL and WRC confirm-then-escalate; persistent ring nominations, fair service, global check boost

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
Baseline after: still 24.2 ms/40 fps, moving 28.6/34, 125k cache rays moving, pop 0.072/0.103.

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
not recorded

## Lessons / when to revisit
not recorded
