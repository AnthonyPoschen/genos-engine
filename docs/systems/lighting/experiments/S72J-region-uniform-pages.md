# S72J: 8x8-texel pages refine together

- **Date:** 2026-10-12
- **Result:** pushed (superseded by S72V)
- **Commit / patch:** 6d055836

## Idea
8x8-texel pages refine together

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
See README section 'Fill speed' entry S72J.

## Numbers
not recorded

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Anthony: uniform low or high beats a patchwork.

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
6d055836 still-scene run: every texel >= 4 sets by frame 60; 38 fps; light pass ~16 ms; page gate held 21-30k texels a round. Anthony (t321u/t322u): better on average but the detail pass still pops, even with all movement stopped.

## Lessons / when to revisit
not recorded
