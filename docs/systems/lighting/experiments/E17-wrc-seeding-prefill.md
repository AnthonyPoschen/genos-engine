# E17: World space radiance cache first draft: running average of 32 rounds, scout rays around the camera pre-fill cells, new fine cells seed from it

- **Date:** 2026-10-12
- **Result:** pushed
- **Commit / patch:** aa1b245e
- **Source:** Engine chat transcript (positions 33-3359), plus git where a commit is named.

## Idea
World space radiance cache first draft: running average of 32 rounds, scout rays around the camera pre-fill cells, new fine cells seed from it

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
Whole picture ~3% darker than c4c787d1 (unexplained at the time).

Numbers context: gi-check scores (error against path-traced references) unless stated; the GPU was the RTX 4070 on zanven-pc where the chat says "on the 4070"; resolution not recorded unless stated.

## Outcome and why
not recorded

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony (t242u): seed slightly before it's needed, not just revealed areas. (t245u): corner reveal works with static lights, but seeded cells go stale when lights change. (t246u): refresh must be very cheap. (t249u, e00238dc): 'everything stays super low detail in a scene full of moving objects'; (t250u) converges once sun and boxes stop. (t253u) WRC refresh back to 64 rays; (t254u) never reset on light change (S62D). (t255u) day->night floor fades as rays build (-> S63A sun/sky share).

## Lessons / when to revisit
not recorded
