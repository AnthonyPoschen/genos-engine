# E04: Lumen-style direction-space probe neighbour filter, plus quick/mid/full gi-check tiers

- **Date:** 2026-10-10
- **Result:** pushed
- **Commit / patch:** 1c3d6d1f, a6432375, 46550f1
- **Source:** Engine chat transcript (positions 33-3359), plus git where a commit is named.

## Idea
Lumen-style direction-space probe neighbour filter, plus quick/mid/full gi-check tiers

## Motivation
not recorded

## Paper basis
Lumen screen probe filter [W21]

## What was implemented
not recorded

## Numbers
Moving error 0.029 vs v1 0.028 (limit 0.03, level, not better); trail 0.058 -> 0.035; moving flicker 0.0007 (limit 0.0005, still FAIL). As rebased it made views 2-6% too bright (settled 0.0498 FAIL); cell-centre fix: flat light 3.5% -> 0.4% too bright.

Numbers context: gi-check scores (error against path-traced references) unless stated; the GPU was the RTX 4070 on zanven-pc where the chat says "on the 4070"; resolution not recorded unless stated.

## Outcome and why
Pushed because every row was same or better.

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
not recorded

## Lessons / when to revisit
not recorded
