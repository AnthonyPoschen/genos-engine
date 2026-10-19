# E19: Rate-based updates: each surface's rate follows how much its light changes; budget is a ceiling, not a target

- **Date:** 2026-10-12
- **Result:** pushed (S64-S65)
- **Commit / patch:** 70d9017f..b5a72df4
- **Source:** Engine chat transcript (positions 33-3359), plus git where a commit is named.

## Idea
Rate-based updates: each surface's rate follows how much its light changes; budget is a ceiling, not a target

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
Anthony (t257u-t266u): moving lights cause swimming from non-uniform updates; neighbour group updates must stay inside the budget; rays proportional to how much the light changed; fewer total updates in the moving scene; 'without picking winners and losers', 'spotting waste in chasing low priority changes'; the denoiser keeps it smooth.

## Lessons / when to revisit
not recorded
