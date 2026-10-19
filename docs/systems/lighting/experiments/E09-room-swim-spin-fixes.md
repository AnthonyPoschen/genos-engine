# E09: Room swim / spinning boxes: filter rounds 2/4/8 probes apart, 64 rays per light-cache patch; strata traced from inside a change are filled, patches near a change relit first

- **Date:** 2026-10-11
- **Result:** pushed
- **Commit / patch:** b5fa8eb7, 1b1101e2, 839d4495
- **Source:** Engine chat transcript (positions 33-3359), plus git where a commit is named.

## Idea
Room swim / spinning boxes: filter rounds 2/4/8 probes apart, 64 rays per light-cache patch; strata traced from inside a change are filled, patches near a change relit first

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
Room swim 0.0045 -> 0.0033 (limit 0.0039); grid redraw 0.0037 -> 0.0019 (0.0027); room blotch 0.0270 -> 0.0181 (0.023); turning with lamps+boxes 0.0070 -> 0.0040 (0.0055); spin error 0.0338 -> 0.0196 (0.027); spin flicker 0.0302 -> 0.0119 (0.02).

Numbers context: gi-check scores (error against path-traced references) unless stated; the GPU was the RTX 4070 on zanven-pc where the chat says "on the 4070"; resolution not recorded unless stated.

## Outcome and why
Settled picture was mottled and the pattern depended on where the probe grid sat on screen; turning redraws it.

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony (t102u): massively improved but final quality off; the reference is grainy. (t103u-t109u): a 'wiggle' run looked fantastic; it was GI v1, which swims ~8x less on large blobs; v1 is the stability floor, not the goal ('it shouldn't be a shining star'); changing quality while moving is bad; blur should scale with noise and screen size (adaptive blend, later put on hold t119u).

## Lessons / when to revisit
Swim score must measure large soft blobs, not fine detail.
