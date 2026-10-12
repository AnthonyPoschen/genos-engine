# S72A: Only key texels get rays; others bilinear from keys

- **Date:** 2026-10-12
- **Result:** pushed (superseded by S72V)
- **Commit / patch:** d38d58ec

## Idea
Only key texels get rays; others bilinear from keys

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
See README section 'Fill speed' entry S72A.

## Numbers
Due backlog 35-50% lower than S71G; fps unchanged.

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
not recorded

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony (t307u): frame rate noticeably up, texels fill faster, but pop-ins are bigger; not spread wide enough; still scene ~40 fps. A video showed static coarse blocks that never refined (-> S72D).

## Lessons / when to revisit
not recorded
