# E03: Project each kept ray along its own direction; moving/uncovered probes borrow strata from last frame's 5x5 neighbours on the same surface

- **Date:** 2026-10-10
- **Result:** pushed
- **Commit / patch:** 52712f9b, 0fb4294e
- **Source:** Engine chat transcript (positions 33-3359), plus git where a commit is named.

## Idea
Project each kept ray along its own direction; moving/uncovered probes borrow strata from last frame's 5x5 neighbours on the same surface

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
Camera test (4070): mid-move error v2 0.092 -> 0.032 (v1 0.028); flicker 0.121 -> 0.047 (v1 0.034); after stop 0.014 -> 0.005; settle 5 -> 1 frames. Trail unchanged 0.058 (v1 0.109). Removed the 2% still brightness bias.

Numbers context: gi-check scores (error against path-traced references) unless stated; the GPU was the RTX 4070 on zanven-pc where the chat says "on the 4070"; resolution not recorded unless stated.

## Outcome and why
The real cause: each probe's ray pattern is chosen by its 25 cm world cell; tiny camera motion flipped floor probes between cells and averaging mixed patterns toward the normal (too bright, blotchy). The light-cache theory was wrong (skipping the cache made it worse); relighting the cache near moved objects did not help the trail (parked).

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony (t22u-t26u): the test must capture the real problem and fail on today's build first; 'only measure at the end'.

## Lessons / when to revisit
not recorded
