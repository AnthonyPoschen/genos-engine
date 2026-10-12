# S78S: Debug counters: what keeps still probes awake

- **Date:** 2026-10-12
- **Result:** finding (diag WIP, g78/S78T-wip.patch)
- **Commit / patch:** g78/S78T-wip.patch

## Idea
Debug counters: what keeps still probes awake

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
Still scene/frame: ~145k kept hits rewritten, ~4,100 changed (>5%) across ~4,100 of 8,100 probes; ~740 texels written. Disabling the surface-cache read: changed hits ~6,500 -> ~300, quiet probes 7,780/8,093. Ignoring <2% change: quiet 20% -> 49% (timing spoiled by GPU contention).

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Main cause: probe hits read surface-cache texels that keep converging (decays, not a loop, but not settled by 600 frames). Unexplained: ~4,000 hits/frame reading coarse cells jump >5% (coarse-patch flip mystery).

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony (t445u/t446u): dump GPU data (lighting maps as textures etc.) and make each component separable and isolatable. (t448u-t451u): papers vs our refire logic; always-on spread checks; no wakes from the shared field; ring-test group-split wakes (approved).

## Lessons / when to revisit
Fix: per-texel IC-style stop rule; no wakes from the shared field. Dump tool planned to find the coarse flips.
