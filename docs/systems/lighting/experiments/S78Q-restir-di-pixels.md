# S78Q: Weighted lamp picking per pixel (ReSTIR DI): Q fused spatial, Q2 temporal-only, Q3 +RIS, Q4 RIS 8 cand + temporal + 1 spatial

- **Date:** 2026-10-12
- **Result:** shelved (Anthony)
- **Commit / patch:** g78/S78Q4.patch

## Idea
Weighted lamp picking per pixel (ReSTIR DI): Q fused spatial, Q2 temporal-only, Q3 +RIS, Q4 RIS 8 cand + temporal + 1 spatial

## Motivation
not recorded

## Paper basis
ReSTIR DI (Bitterli 2020), RIS

## What was implemented
not recorded

## Numbers
Q4: still 8.62/100, moving 18.9/50 (light 6.38 vs 6.59), pop 0.086/0.122 vs 0.069/0.104, noise 3.67 vs 3.08; 100-lamp room light 47.9 -> 35.2 ms (17 -> 21 fps); +16.6 MiB. Q: washed out shadow wedges; Q2 noise 9.0 vs 3.1.

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
shad-S78Q4/b (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Moving scene has ~5 lamps per pixel; probe-ray hits still loop all lamps.

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony (t430u/t431u): asked whether it throws lamps away (no: unbiased weighted picking); add the 100-lights small room as a benchmark. Decision (21:22, answering question t436s1373 'Push weighted lamp picking?', options push / push and do probe hits too / shelve): 'Shelve Q4 for now'.

## Lessons / when to revisit
Follow-up: apply picking to probe hits for many-lamp scenes.
