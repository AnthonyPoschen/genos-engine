# E21: Code audit: is CPU<->GPU transfer the bottleneck?

- **Date:** 2026-10-12
- **Result:** finding (no)
- **Commit / patch:** none
- **Source:** Engine chat transcript (positions 33-3359), plus git where a commit is named.

## Idea
Code audit: is CPU<->GPU transfer the bottleneck?

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
GPU-bound: ~35 ms GPU per frame (28 fps); light pass is the cost.

Numbers context: gi-check scores (error against path-traced references) unless stated; the GPU was the RTX 4070 on zanven-pc where the chat says "on the 4070"; resolution not recorded unless stated.

## Outcome and why
not recorded

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony (t294u-t297u): suspected transfers; design rule: everything frequently needed lives in GPU memory at deterministic locations, passes read/write each other there, CPU sends only small changes. (t297u answer) remove all v1 lighting and rename v2 (-> S71A-C); (t298u) no env variable needed.

From Chief's notes: 6a3cd7c (S71A, 12 Oct 12:12) removed GI v1 [uncertain whether Anthony judged a master vs gi-v2 head-to-head first, and whether it is on master].

## Lessons / when to revisit
Led to S69C and S70A-C.
