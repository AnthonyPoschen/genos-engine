# S57: One texel size (12.5 cm pixels, 25 cm hit patches) at every distance

- **Date:** 2026-10-12
- **Result:** pushed (decision, Anthony)
- **Commit / patch:** 1dbbca64

## Idea
One texel size (12.5 cm pixels, 25 cm hit patches) at every distance

## Motivation
not recorded

## Paper basis
Lumen cards keep resolution through camera moves; no surveyed paper keys cell size to the eye

## What was implemented
not recorded

## Numbers
not recorded

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Band rings followed the camera and showed coarse grain while running.

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony (t194u-t212u): saw a coarse circle looking down and walls/floors dropping to coarse grain when walking closer; with everything frozen saw the ring; first 'don't delete them, make sure they don't remove what existed' (-> S56), then 'it's not in Lumen or the papers, it's causing more issues than it helps' -> removed.

## Lessons / when to revisit
Rule: camera decides residency and update order only, never cell size.
