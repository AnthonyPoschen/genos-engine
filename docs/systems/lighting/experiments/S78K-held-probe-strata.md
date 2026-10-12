# S78K: Held probe strata + 1 check ray/probe

- **Date:** 2026-10-12
- **Result:** pushed
- **Commit / patch:** a6559e5e

## Idea
Held probe strata + 1 check ray/probe

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
still 8.33/103, moving 18.9/50, pop 0.077/0.114; no VRAM

Numbers context: 1080p headless. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
not recorded

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Light-pass breakdown by switching parts off (moving, a6559e5e, light 6.60 ms, frame 18.64): no kept-hit relight 3.34 / 15.62; no per-pixel direct 3.86 / 16.04; no TDL lookup 5.99 / 18.10. Conclusion: at dusk moving lamps genuinely change the light; the work is real.

## Lessons / when to revisit
not recorded
