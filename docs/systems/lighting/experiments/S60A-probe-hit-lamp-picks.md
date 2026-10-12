# S60A: Probe hits take at most 2 shadow rays, lamps picked systematically in proportion to unshadowed light

- **Date:** 2026-10-12
- **Result:** pushed
- **Commit / patch:** a2b41c52

## Idea
Probe hits take at most 2 shadow rays, lamps picked systematically in proportion to unshadowed light

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
Before: light pass 27-31 ms of a 41-48 ms frame. After: not recorded in commit.

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Every kept hit of all 8 strata was relit every frame with one shadow ray per lamp.

## Lessons / when to revisit
Lamp picking already exists for probe hits; Q4 later applied picking to pixels.
