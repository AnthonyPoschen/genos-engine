# E08: Turn swim fix: trace missing strata from the probe in the same frame; pattern cell looked up half a cell in from the surface

- **Date:** 2026-10-10
- **Result:** pushed
- **Commit / patch:** 7ebb886, e14317f, edffa81
- **Source:** Engine chat transcript (positions 33-3359), plus git where a commit is named.

## Idea
Turn swim fix: trace missing strata from the probe in the same frame; pattern cell looked up half a cell in from the surface

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
Turn arrival 0.013 -> 0.005 (limit 0.008); pop at motion start 0.014 -> 0.005 (limit 0.009); blotch arriving from a fast turn ~0.016 -> 0.005. At 6 deg/frame ~27% of rays missing, 3 deg/frame ~13%. Quick tier 123 s -> 81 s (90-pose camera clip and corner walk cut).

Numbers context: gi-check scores (error against path-traced references) unless stated; the GPU was the RTX 4070 on zanven-pc where the chat says "on the 4070"; resolution not recorded unless stated.

## Outcome and why
New strip of surface entered with 1 of 8 strata; floor (0 m) and roof (3 m) sat exactly on 25 cm cell edges.

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony (t74u-t88u) test rules: soak and record a settled frame before any motion; end soaked; motion returns to the exact start so start and end can be compared; short tests for one situation with one realistic movement; cut tests that never caught anything. Stuck splotches after stopping were not reproduced with camera-only motion.

## Lessons / when to revisit
not recorded
