# S74A-cam: Fix: last-frame camera read from stale scene-block offsets

- **Date:** 2026-10-12
- **Result:** pushed
- **Commit / patch:** 81c00734

## Idea
Fix: last-frame camera read from stale scene-block offsets

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
not recorded

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Scouts saw motion every frame (4096 scout rays a round when still); probe reprojection read garbage.

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Reported with 50db0f70: 32-38 fps vs 27-30 for the checkpoint, same brightness; pop 0.105% vs 0.084% settled (later shown to be within the checkpoint's own run-to-run range).

## Lessons / when to revisit
Bugs that make everything look 'changed' defeat quiet paths; see also S78C and S78S.
