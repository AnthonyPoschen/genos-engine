# E18: Variance-guided surface-space denoise plus perceptual stop rule (S62A)

- **Date:** 2026-10-12
- **Result:** pushed, later superseded (S72C reverted the always-on version)
- **Commit / patch:** 052fba94
- **Source:** Engine chat transcript (positions 33-3359), plus git where a commit is named.

## Idea
Variance-guided surface-space denoise plus perceptual stop rule (S62A)

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
not recorded

Numbers context: gi-check scores (error against path-traced references) unless stated; the GPU was the RTX 4070 on zanven-pc where the chat says "on the 4070"; resolution not recorded unless stated.

## Outcome and why
not recorded

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony (t244u): still scene goes 20 -> 180 fps when idle; find a shortcut instead of brute-forcing the sqrt(n) tail. Approved queueing after the WRC fix (t247u).

## Lessons / when to revisit
not recorded
