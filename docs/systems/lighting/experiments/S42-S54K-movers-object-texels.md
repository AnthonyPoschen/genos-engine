# S42-S54K: Movers in the surface cache: mover expiry 240 frames, pixels near movers, object-local texels, mover upload fix

- **Date:** 2026-10-11/12
- **Result:** pushed
- **Commit / patch:** 1e7573a3, f13d56ba, ba9adca0

## Idea
Movers in the surface cache: mover expiry 240 frames, pixels near movers, object-local texels, mover upload fix

## Motivation
not recorded

## Paper basis
GIBS object-local surfels; Lumen card recapture for movers

## What was implemented
not recorded

## Numbers
S49: mix 0.0084/0.0087, spin 0.0871/0.0470. S54K headless: spin 0.0716/0.0397, mix 0.0092/0.0095.

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
S52 found movers never reached the GPU (change block upload stopped after the changes). Swapping pixels to probes under light change (S46/S47) was most of the mix error, so texels stay shown.

## Lessons / when to revisit
Check that change data actually reaches the GPU before tuning reactions to it.
