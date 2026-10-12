# E14: Survey item 1: GIBS change-aware cells (dormant cells fire few change-detection rays)

- **Date:** 2026-10-11
- **Result:** pushed (flag 1048576, see S26-S41)
- **Commit / patch:** 6ce97ac6..d8101b09
- **Source:** Engine chat transcript (positions 33-3359), plus git where a commit is named.

## Idea
Survey item 1: GIBS change-aware cells (dormant cells fire few change-detection rays)

## Motivation
not recorded

## Paper basis
GIBS [GIBS21]; survey /workspace/research/realtime-gi-survey-2021-2026.md (18 works)

## What was implemented
not recorded

## Numbers
Still shimmer 0, no drift over 600 frames; restarts during a turn 234 -> under 1.

Numbers context: gi-check scores (error against path-traced references) unless stated; the GPU was the RTX 4070 on zanven-pc where the chat says "on the 4070"; resolution not recorded unless stated.

## Outcome and why
not recorded

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony (t134u): constant faint pixel swimming while still on the darkness fix; (t135u) lights moving and returning should give an identical image (lampret test); (t138u-t147u): use Lumen and 5 years of papers from all venues when stuck; keep a lighting doc comparing each component with papers (this doc). (t149u-t154u): build each item, mark done when its quick tests pass, log each failure with where it first appeared (fault ledger).

## Lessons / when to revisit
not recorded
