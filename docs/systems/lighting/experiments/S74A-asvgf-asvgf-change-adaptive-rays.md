# S74A-asvgf: A-SVGF same-seed 1-ray checks on 1/9 resolved texels, lambda drives alpha and rays

- **Date:** 2026-10-12
- **Result:** pushed
- **Commit / patch:** 81c00734

## Idea
A-SVGF same-seed 1-ray checks on 1/9 resolved texels, lambda drives alpha and rays

## Motivation
not recorded

## Paper basis
A-SVGF 2018 eq. 6-7

## What was implemented
not recorded

## Numbers
not recorded

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
not recorded

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Deviations stated: lambda->rays mapping (whole set with probability min(1, lambda/0.25)); 1-ray checks spread as a box over 3x3 with no a-trous of gradients; checks are measurement only; dark-ray normalisation (>= 10% of texel mean); thresholds 0.05, 3%, 0.25, 900-frame age floor chosen by us.

## Lessons / when to revisit
not recorded
