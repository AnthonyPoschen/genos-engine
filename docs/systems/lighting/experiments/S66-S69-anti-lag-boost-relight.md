# S66-S69: Budget boost after turns (S66A), neighbourhood clamp/anti-lag (S67A), read-since-relight slack (S68A), seed weight 2 (S68B, reverted), stale after sun rise (S69A/B), kept hits relit on stratum frame + halfway (S69C)

- **Date:** 2026-10-12
- **Result:** pushed (S68B reverted)
- **Commit / patch:** 782fe38b..2f35eaa2

## Idea
Budget boost after turns (S66A), neighbourhood clamp/anti-lag (S67A), read-since-relight slack (S68A), seed weight 2 (S68B, reverted), stale after sun rise (S69A/B), kept hits relit on stratum frame + halfway (S69C)

## Motivation
not recorded

## Paper basis
ReBLUR / A-SVGF history acceleration; Karis TAA clamp; Lumen LastUsed

## What was implemented
not recorded

## Numbers
S68A vs S67A (640x360): turn error area 0.0573 -> 0.0425, light move 0.0262 -> 0.0205, frames to 2% 120 -> 60, fps 50.4 -> 49.5. S68B: turn AUC .0468 vs .0425 (reverted). S69C: light pass was ~10 ms on all 8 strata.

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
S66A did not change time to correct; S68B worse.

## Lessons / when to revisit
not recorded
