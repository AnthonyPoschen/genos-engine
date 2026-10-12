# S78O: Kept hits remember their texel

- **Date:** 2026-10-12
- **Result:** rejected
- **Commit / patch:** g78/S78O.patch

## Idea
Kept hits remember their texel

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
8.39/103, 18.7/51 (light 6.59 vs 6.65), pop 0.069/0.104; +16 MiB

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Lookup was ~0.06 ms; cost is the lighting itself.

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony picked it as an option after S78M/N; later (t429u) asked for a deep dive and accepted the rejection.

## Lessons / when to revisit
not recorded
