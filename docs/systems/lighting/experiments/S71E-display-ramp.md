# S71E: Display ramp to hide convergence pop

- **Date:** 2026-10-12
- **Result:** reverted (S71F)
- **Commit / patch:** 3457f4b2 / revert 4aeeced6

## Idea
Display ramp to hide convergence pop

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
Anthony 12:37: smoothing would be lag; the goal is fast fill.

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony (t299u-t301u, 12:27-12:37): pops are fine if the screen fills near-immediately; smoothing becomes the issue.

## Lessons / when to revisit
Principle: pop-in is fine if the screen fills near-immediately; no display smoothing.
