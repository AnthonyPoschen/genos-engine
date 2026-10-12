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

Numbers context: 1080p headless. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Few probes qualified: kept light changes most frames.

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony (t440u-t442u): need enhanced debug readouts; suspected a cycle of probes waking each other; this would have affected every earlier 'go quiet' test.

## Lessons / when to revisit
Led to the churn investigation. Retest: this was judged before the still-scene churn (surface cache still converging, waking probes every frame; see [counters](S78S-counters-still-churn.md)) was found. Re-run on the build that fixes it.
