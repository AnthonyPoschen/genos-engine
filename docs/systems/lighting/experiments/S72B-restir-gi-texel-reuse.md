# S72B: ReSTIR GI reservoir reuse on texels

- **Date:** 2026-10-12
- **Result:** reverted (S72H)
- **Commit / patch:** b0e76a3b / 43690446

## Idea
ReSTIR GI reservoir reuse on texels

## Motivation
not recorded

## Paper basis
ReSTIR GI

## What was implemented
See README section 'Fill speed' entry S72B.

## Numbers
not recorded

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Neighbour reuse without visibility spread correlated low-frequency blotches.

## Lessons / when to revisit
not recorded
