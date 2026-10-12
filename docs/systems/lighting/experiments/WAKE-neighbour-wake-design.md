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

Still and moving scene: default stress scene (dusk, moving lamps), RTX 4070, 1080p unless noted; ms / fps per frame; pop = share of pixel-frames stepping >1% at frames 20-80 / 80-320; brightness = smoke mean luminance at pose a. Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
not recorded

## Lessons / when to revisit
After it lands, retest S77C, S78G, S78H, S78L, S78N, S78R.
