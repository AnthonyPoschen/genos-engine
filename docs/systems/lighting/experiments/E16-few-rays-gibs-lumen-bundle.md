# E16: Lumen update order + GIBS extra rays + few rays per relight built together (S58A, S59C), then settle fixes (S59D, S59E)

- **Date:** 2026-10-12
- **Result:** pushed
- **Commit / patch:** a4a7b23d, e7803342, eedd6671, c4c787d1
- **Source:** Engine chat transcript (positions 33-3359), plus git where a commit is named.

## Idea
Lumen update order + GIBS extra rays + few rays per relight built together (S58A, S59C), then settle fixes (S59D, S59E)

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
e7803342: ~41 vs 39 fps (~5%) default scene, 1280x720 headless. eedd6671 smoke fps 18 -> 9 vs e7803342 (not a clean benchmark).

Numbers context: gi-check scores (error against path-traced references) unless stated; the GPU was the RTX 4070 on zanven-pc where the chat says "on the 4070"; resolution not recorded unless stated.

## Outcome and why
Surfaces settled with too few rays (splotchy); the traced surface-cache rays were not what limits frame rate.

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony (t220u-t230u): do all three, then test; don't loop on small misses. (t231u/t232u) 'it's settling too soon... splotchy'; (t233u) no performance gains seen; (t235u) bump quality before it settles (S59E). (t237u): quality eases in like an easing curve; (t238u) seed a big new area from a few heavily traced points and borrow -> world space radiance cache seeding (name chosen t241u).

From Chief's notes: 2026-10-09: probe priority to be simplified to one score, view coverage x likely error, with aging.

## Lessons / when to revisit
not recorded
