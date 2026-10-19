# E05: When a moving box crosses a kept ray, re-trace it in the same frame instead of dropping it

- **Date:** 2026-10-10
- **Result:** pushed
- **Commit / patch:** defa10af
- **Source:** Engine chat transcript (positions 33-3359), plus git where a commit is named.

## Idea
When a moving box crosses a kept ray, re-trace it in the same frame instead of dropping it

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
Flicker while moving (full) 0.0008 -> 0.0004 (limit 0.0005); back-and-forth flicker 0.0005 -> 0.0001 (limit 0.0003); trail 0.037 -> 0.022; rays held per probe while moving 472 -> 510 of 512.

Numbers context: gi-check scores (error against path-traced references) unless stated; the GPU was the RTX 4070 on zanven-pc where the chat says "on the 4070"; resolution not recorded unless stated.

## Outcome and why
not recorded

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony (t46u-t52u): starts beautiful, goes splotchy and darker the moment anything moves, worst in the hallway by the dark room; this was later found to be a test artefact for the hallway (bounce-only view), see E09.

## Lessons / when to revisit
not recorded
