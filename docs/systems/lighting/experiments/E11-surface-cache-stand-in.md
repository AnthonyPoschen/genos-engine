# E11: Stand-in test: pixels read a converged world light cache (world=1) to decide whether a surface cache is worth building

- **Date:** 2026-10-11
- **Result:** finding (go)
- **Commit / patch:** none (experiment runs W6/W7)
- **Source:** Engine chat transcript (positions 33-3359), plus git where a commit is named.

## Idea
Stand-in test: pixels read a converged world light cache (world=1) to decide whether a surface cache is worth building

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
W6 (1024 rays, 25 cm): structure 0.0188, blotch 0.021, swim 0.0006/0.0002, close-up 0.0130. W7 (1024 rays, 12.5 cm): structure 0.0092 (passes 0.010), blotch 0.014, swim 0.0006/0.0002, close-up 0.0149 (fails 0.013), grid redraw 0.0004.

Numbers context: gi-check scores (error against path-traced references) unless stated; the GPU was the RTX 4070 on zanven-pc where the chat says "on the 4070"; resolution not recorded unless stated.

## Outcome and why
A converged world-anchored cache fixes swim and structure; costs: blocky close-up, newly seen surfaces start dark.

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony (t120u/t121u): 'it will negate needing the adaptive blending'; (t122u) next is Lumen's few-rays system for performance.

## Lessons / when to revisit
not recorded
