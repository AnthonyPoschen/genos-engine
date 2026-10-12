# WAKE: Anthony's wake design: no wakes from shared field; real change wakes neighbours by coarse-to-fine ring tests with group-test splitting; always-on stratified spread-out checks

- **Date:** 2026-10-12
- **Result:** planned (approved by Anthony)
- **Commit / patch:** none yet

## Idea
Anthony's wake design: no wakes from shared field; real change wakes neighbours by coarse-to-fine ring tests with group-test splitting; always-on stratified spread-out checks

## Motivation
not recorded

## Paper basis
Ward/Krivanek IC error bound; A-SVGF sparse checks; Lumen background budget; progressive radiosity (Cohen 1988); hierarchical radiosity (Hanrahan 1991); group testing (Dorfman 1943); galloping search. Combination is original (documented deviation).

## What was implemented
Rings at r,2r,4r,8r.. on golden-angle pattern; fill in only behind failed tests; failure only beyond noise bound; max radius scales with change.

## Numbers
not recorded

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
not recorded

## Notes from chat
Anthony (2026-10-12 22:50, t453u) asked whether the constant probe searching plus fan-out technique would be worth publishing if it works and performs well. Answer not in the available transcript.

## Lessons / when to revisit
After it lands, retest S77C, S78G, S78H, S78L, S78N, S78R.
