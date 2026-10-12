# E10: GPU path-traced reference on hardware ray queries (independent of the engine)

- **Date:** 2026-10-11
- **Result:** pushed
- **Commit / patch:** 5bb08892, 86e90392, 6ce7fd48
- **Source:** Engine chat transcript (positions 33-3359), plus git where a commit is named.

## Idea
GPU path-traced reference on hardware ray queries (independent of the engine)

## Motivation
Anthony (t114u): use the 4070's ray tracing to make a true ground truth faster.

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
About 14x faster than the CPU reference on 16 cores; less than half the grain; GPU refs read 0.002-0.004 higher error for the same engine (old CPU corner leak), so the mean line moved 0.040 -> 0.043.

Numbers context: gi-check scores (error against path-traced references) unless stated; the GPU was the RTX 4070 on zanven-pc where the chat says "on the 4070"; resolution not recorded unless stated.

## Outcome and why
not recorded

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony (t116u): smoothed reference shows a clear bounce band off the walls while v2 shows blotches that do not follow light; became the light-structure grade.

## Lessons / when to revisit
not recorded
