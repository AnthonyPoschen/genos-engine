# S26-S41: Surface cache: world texels pixels read directly, running mean to cap, flag experiments (239882, 524288 camera-only ordering, 1048576 GIBS/MSME estimator, 262144 hold converged)

- **Date:** 2026-10-11
- **Result:** pushed (default from S54K)
- **Commit / patch:** 21cb8a41..d8101b09

## Idea
Surface cache: world texels pixels read directly, running mean to cap, flag experiments (239882, 524288 camera-only ordering, 1048576 GIBS/MSME estimator, 262144 hold converged)

## Motivation
not recorded

## Paper basis
Lumen surface cache [W22]; GIBS [GIBS21]; MSME [MSME]; SHaRC read rule

## What was implemented
not recorded

## Numbers
S41 (flags 1812746): room-a 0.0405, world swim 0.0019 PASS, grid redraw 0.0008 PASS, still drift 0; spin 0.1096 FAIL, mix 0.0116 FAIL. S27 (239882): world swim 0.0042 FAIL. Flag 262144 fixed drift (0.0006) but blotch 0.0207 / close-up 0.0165. Offer restarts 234 -> 0.6 per readback with 524288. fps not recorded.

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Texels are world-anchored, removing probe swim. 262144 (hold) rejected for blotch.

## Lessons / when to revisit
Holding converged texels stops drift but freezes noise; later work (S72R/MSME, IC stop rule) revisits that trade-off.
