# S78J: Per-pixel direct reuse in quiet frames

- **Date:** 2026-10-12
- **Result:** pushed
- **Commit / patch:** 18894551

## Idea
Per-pixel direct reuse in quiet frames

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
still 9.42/93 (light 1.56), moving 18.8/50, pop 0.088/0.115; +7.9 MiB

Numbers context: headless weston 1080p, frames 300-600. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
not recorded

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Moving scene (headless weston 1080p, f300-600) 6474a941 -> 18894551 total 18.72 -> 18.84 ms; light 6.31 -> 6.50; a-trous 4.87 -> 4.81. Kept probe hits relit 1.08M/frame moving vs 0.50M still; TDL checks 50.6k/frame moving with half escalating.

## Lessons / when to revisit
Any mover relights all pixels.
