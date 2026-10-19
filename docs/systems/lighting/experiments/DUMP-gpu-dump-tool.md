# DUMP: Debug dump of GPU data from two consecutive frames per component (direct, bounce, coarse/fine WRC, probe results, hit sources, change masks) plus freeze/isolate switches

- **Date:** 2026-10-12
- **Result:** in progress
- **Commit / patch:** none yet

## Idea
Debug dump of GPU data from two consecutive frames per component (direct, bounce, coarse/fine WRC, probe results, hit sources, change masks) plus freeze/isolate switches

## Motivation
Anthony: extract in-memory data as viewable files; isolate each component.

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
not recorded

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
not recorded

## Lessons / when to revisit
not recorded
