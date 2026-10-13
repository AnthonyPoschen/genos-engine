# S73B: All scout hits claim fine texels and stamp LastUsed

- **Date:** 2026-10-12
- **Result:** ported (71547c9 on cursor/gi-prefill-spare-budget-3544). Originally reverted (8b62829b) and kept on gi-v2-prefill (22bcd47f). Darkening cause and the fix: [S73-prefill-root-cause-and-plan](S73-prefill-root-cause-and-plan.md).
- **Commit / patch:** 22bcd47f

## Idea
All scout hits claim fine texels and stamp LastUsed

## Motivation
not recorded

## Paper basis
Lumen surface cache GPU feedback

## What was implemented
not recorded

## Numbers
not recorded

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Source (Engine report, 2026-10-12 ~16:05): S73B made every metric worse, including a darker image in all five captures; likely the off-screen feedback texels took ray budget; the darkening cause was not found. Suggested retune (fewer marked scout hits, keep marked-used texels out of cache reads until they have samples) was not done. Reverted (8b62829b); kept on gi-v2-prefill (22bcd47f). Lumen-only approach, once performance is good (Anthony t361u-t363u).

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Turn strip ~/.cache/gi2diag/perf/turn-S73B/strip.png was not looked at; logs ship-S73A.log, ship-S73B.log.

## Lessons / when to revisit
S-curve off-screen lag bound idea (Anthony 15:55) deferred until spare performance.
