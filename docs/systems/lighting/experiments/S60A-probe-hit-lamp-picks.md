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

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Every kept hit of all 8 strata was relit every frame with one shadow ray per lamp.

## Lessons / when to revisit
Lamp picking already exists for probe hits; Q4 later applied picking to pixels.
