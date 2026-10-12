# S72M: Seed first-reveal texels from probe light

- **Date:** 2026-10-12
- **Result:** pushed
- **Commit / patch:** 3db494ca

## Idea
Seed first-reveal texels from probe light

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
See README section 'Fill speed' entry S72M.

## Numbers
not recorded

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Screen-aligned tearing on turn side (probe/texel seam).

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Bundle 3db494ca: fps 27-34, still ~37, light pass 16.8 ms, total 26 ms; pop 0.40% vs 0.76% early, 0.22% vs 0.29% later. Anthony (t326u): tearing fixed, squares building up less noticeable.

## Lessons / when to revisit
not recorded
