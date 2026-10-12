# S72V: Layer 2 as SVGF a-trous + Gortler pull-push; irradiance-caching placement (V-V11 fixes)

- **Date:** 2026-10-12
- **Result:** pushed (checkpoint 6b4fe308)
- **Commit / patch:** d23a9218..6b4fe308

## Idea
Layer 2 as SVGF a-trous + Gortler pull-push; irradiance-caching placement (V-V11 fixes)

## Motivation
not recorded

## Paper basis
SVGF 2017, Dammertz 2010, Gortler 1996, Ward 1988, Krivanek 2006

## What was implemented
See README section 'Fill speed' entry S72V.

## Numbers
Under S72V9 the still smoke used 744k rays a round settled against 87k (the frame-wide quiet test never held with the sun running); fixed per record in S72V11. fps not recorded.

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
V6 found stage bits ORed into bit 3, so the filter never ran; V5 cell addressing; see README.

## Lessons / when to revisit
not recorded
