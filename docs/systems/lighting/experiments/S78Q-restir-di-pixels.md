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

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
shad-S78Q4/b (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Moving scene has ~5 lamps per pixel; probe-ray hits still loop all lamps.

## Lessons / when to revisit
Follow-up: apply picking to probe hits for many-lamp scenes.
