# S75: Lumen surface-cache direct lighting for GI hits (TDL); A-SVGF SE 0.05

- **Date:** 2026-10-12
- **Result:** pushed
- **Commit / patch:** d591abfa

## Idea
Lumen surface-cache direct lighting for GI hits (TDL); A-SVGF SE 0.05

## Motivation
not recorded

## Paper basis
Lumen [W22]: cached direct light read by GI hits only

## What was implemented
not recorded

## Numbers
not recorded

Numbers context: 1920x1080 on zanven-pc (RTX 4070), back-to-back runs; still scene and perf/moving.rhai. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
not recorded

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
1920x1080, back-to-back on zanven-pc. 6b4fe308 / S74A / S75: still total 31.3-37.0 / 25.6 / 24.3-24.0 ms; still light pass 16.0 / 10.1 / 8.7 ms; still fps 27-31 / 37 / 40; moving total 35.0 / 29.1 / 29.8 ms; moving light 17.9 / 12.0 / 12.7 ms; moving fps 28 / 33 / 33; pop 0.081-0.090/0.111-0.124 / 0.072-0.094/0.102-0.110 / 0.087/0.114 %; brightness night/turn30/turn180/day30/day180 .351/.497/.505/.534/.531 -> .351/.495/.504/.536/.528. Moving scene is perf/moving.rhai (dusk, lamps and boxes moving, 50% dynamic, 600 frames). Resolved SE threshold 3% -> 5% (no paper value; 10% tried and rejected: early pop 0.45-0.53%, turn/day ~2.5% darker). The 0.084% 'pop regression' was one lucky checkpoint run. Anthony (t380u): 'barely notice the performance gains... it now looks great'.

## Lessons / when to revisit
not recorded
