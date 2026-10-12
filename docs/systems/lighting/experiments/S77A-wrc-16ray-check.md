# S77A: WRC change refresh as 16-ray check

- **Date:** 2026-10-12
- **Result:** pushed
- **Commit / patch:** c146bf1c

## Idea
WRC change refresh as 16-ray check

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
still 24.0/40, moving 27.9/35, pop 0.076/0.105; +1% brighter smoke.

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
not recorded

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Moving 28.4 -> 27.9 ms (34 -> 35 fps); light pass 11.2 -> 10.6 ms; sweep 26.3 -> 26.2 ms; turn/day smoke ~1% brighter (day30 .547 vs .541, turn30 .503 vs .497), counted as no worse (judgment call). Uncontended runs (nvidia-smi checked).

## Lessons / when to revisit
not recorded
