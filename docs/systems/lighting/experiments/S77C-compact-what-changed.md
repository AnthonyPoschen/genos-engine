# S77C: Compact visits unread texels with no check due 1 round in 8

- **Date:** 2026-10-12
- **Result:** pushed
- **Commit / patch:** 14f1b644

## Idea
Compact visits unread texels with no check due 1 round in 8

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
smoke fps 39; gather not reduced (inputs change every frame).

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
not recorded

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Only compact was changed (texels unread for 60 frames with no check due visited 1 round in 8); gather unchanged because every probe gets a new stratum every frame (skipping would need ~130 MB per-stratum sums at 1080p). Anthony (t373u-t374u) had doubted 'only changed' smoothing but asked to try it separately.

## Lessons / when to revisit
Retest: this was judged before the still-scene churn (surface cache still converging, waking probes every frame; see [counters](S78S-counters-still-churn.md)) was found. Re-run on the build that fixes it.
