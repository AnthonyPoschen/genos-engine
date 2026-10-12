# S77B: Per-pixel shadow reuse with check and escalate

- **Date:** 2026-10-12
- **Result:** reverted (9e242987)
- **Commit / patch:** cc0b4726

## Idea
Per-pixel shadow reuse with check and escalate

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
still 24.2/40, moving 27.9/35, pop 0.070/0.107; 2/3 shadow rays reused but light pass unchanged 8.6 ms.

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
No speedup measured; re-pushed on request after a smoke test without moving shadows; moving shadow edges tore (staircase). See postmortem.

## Lessons / when to revisit
Every step needs a moving-shadow smoke test with screenshots looked at (shad2-S77R/mv-full.png after revert).
