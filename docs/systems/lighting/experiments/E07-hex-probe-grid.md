# E07: Hex-offset (triangular lattice) probe grid

- **Date:** 2026-10-10
- **Result:** parked (Anthony)
- **Commit / patch:** none
- **Source:** Engine chat transcript (positions 33-3359), plus git where a commit is named.

## Idea
Hex-offset (triangular lattice) probe grid

## Motivation
not recorded

## Paper basis
OpenAI preprint, universal optimality of the triangular lattice

## What was implemented
not recorded

## Numbers
Estimate only: about 13% fewer probes for the same quality (not measured).

Numbers context: gi-check scores (error against path-traced references) unless stated; the GPU was the RTX 4070 on zanven-pc where the chat says "on the 4070"; resolution not recorded unless stated.

## Outcome and why
not recorded

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony (t44u): A/B candidate after v2 is good; park and remind. (t123u): fold into the performance pass with the few-rays work.

From Chief's notes: the OpenAI maths shortlist (/workspace/openai-math-engine-shortlist.md) [date uncertain, ~10-11 Oct] was sent to Engine; Engine adopted the sphere-energy score for probe ray sets and queued the hex-grid experiment. Result still open.

## Lessons / when to revisit
not recorded
