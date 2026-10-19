# S70: Indirect dispatch for cache rays (S70A), no GPU->CPU counter reads in frame loop (S70B), resident scene buffer with delta upload (S70C)

- **Date:** 2026-10-12
- **Result:** pushed
- **Commit / patch:** 8758d467, 76245067, b0675659

## Idea
Indirect dispatch for cache rays (S70A), no GPU->CPU counter reads in frame loop (S70B), resident scene buffer with delta upload (S70C)

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
not recorded

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Followed the E21 audit (transfer not the bottleneck). Anthony's design rule: GPU-resident data at deterministic locations.

## Lessons / when to revisit
not recorded
