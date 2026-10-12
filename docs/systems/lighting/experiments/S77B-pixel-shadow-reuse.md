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

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
No speedup measured; re-pushed on request after a smoke test without moving shadows; moving shadow edges tore (staircase). See postmortem.

## Lessons / when to revisit
Every step needs a moving-shadow smoke test with screenshots looked at (shad2-S77R/mv-full.png after revert).
