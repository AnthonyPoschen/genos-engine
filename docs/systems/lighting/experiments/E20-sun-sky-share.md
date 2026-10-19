# E20: Cache keeps sun/sky share separate from lamp light; sun/sky change rescales it at once (S63A); night->day black squares fix (S69A/B)

- **Date:** 2026-10-12
- **Result:** pushed
- **Commit / patch:** 0f288334, bfdb8900, 27afd41e
- **Source:** Engine chat transcript (positions 33-3359), plus git where a commit is named.

## Idea
Cache keeps sun/sky share separate from lamp light; sun/sky change rescales it at once (S63A); night->day black squares fix (S69A/B)

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
Outdoor day->night pan first-frame error 1.43 -> 0.065.

Numbers context: gi-check scores (error against path-traced references) unless stated; the GPU was the RTX 4070 on zanven-pc where the chat says "on the 4070"; resolution not recorded unless stated.

## Outcome and why
S69A: a texel lit at night stores almost no sun share; rescale of near-zero stayed black and the scheduler ranked it by its old small change.

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony (t271u): still partial bad rays but fixes very fast. (t281u-t286u): insane splotchiness and black squares going night->day, fps 27 vs 9 while building; 'something is wasting performance'. (t293u) after bfdb8900 squares only where the camera looks.

## Lessons / when to revisit
not recorded
