# S73A: Guard-band scout rays claim fine texels just outside the frustum

- **Date:** 2026-10-12
- **Result:** reverted (11639205); kept on gi-v2-prefill
- **Commit / patch:** 097d67de

## Idea
Guard-band scout rays claim fine texels just outside the frustum

## Motivation
Anthony 15:50: turning/walking backwards shows unfilled texels.

## Paper basis
Lumen ray-hit feedback; guard band (Bavoil & Sainz 2009; Mara et al. 2016)

## What was implemented
not recorded

## Numbers
not recorded

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Source (Engine report on S73B, 2026-10-12 ~16:05): against the 6b4fe308 checkpoint the S73A+S73B build was darker, slower and popped more; Anthony should not test it as-is. Reverted to the checkpoint (11639205, 8b62829b) and kept on gi-v2-prefill. Per Anthony (t363u), off-screen low-priority areas can only be filled once there is spare capacity, so pre-fill waits for performance.

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony (15:51-15:57, t357u-t363u): do off-screen pre-fill for turns and walking backwards next; pre-fill directions not faced and occluded spaces at lower priority (S-curve lag idea); asked for paper basis, then: try the Lumen approach, expecting it to look bad until there is spare performance.

## Lessons / when to revisit
Bring back once there's spare budget. Was affected by the scout camera bug (S74A).
