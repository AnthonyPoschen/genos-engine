# S77D: Dirty-page a-trous with per-level storage

- **Date:** 2026-10-12
- **Result:** pushed
- **Commit / patch:** bc63443d

## Idea
Dirty-page a-trous with per-level storage

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
still 21.0/45 (atrous 3.5), moving 26.8/36 (atrous 6.1); +128 MB VRAM.

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
not recorded

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
All six filter levels stored per slot: 240 -> 368 B per slot, +128 MB VRAM; re-filter only when input moved > 1%; a-trous 6.4 -> 3.5 ms still. Single uncontended run. Earlier judged impossible as briefed (seams) without per-level storage (~80 MB estimate).

## Lessons / when to revisit
not recorded
