# E12: Bug: SC flags were capped at 127 on the Rust side (and lost precision in head w)

- **Date:** 2026-10-11
- **Result:** fixed (S19, 8526e44e)
- **Commit / patch:** 8526e44e
- **Source:** Engine chat transcript (positions 33-3359), plus git where a commit is named.

## Idea
Bug: SC flags were capped at 127 on the Rust side (and lost precision in head w)

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
After the fix (S20) contact darkness 9.5% -> 2% too dark; later best ~4% too dark in bounce-only areas (from 9.5%).

Numbers context: gi-check scores (error against path-traced references) unless stated; the GPU was the RTX 4070 on zanven-pc where the chat says "on the 4070"; resolution not recorded unless stated.

## Outcome and why
Every overnight run with a flag >= 128 ran as 127: S14-S18 'no gain' results were meaningless.

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
not recorded

## Lessons / when to revisit
Third 'the change never reached the GPU' bug (with S52 movers and S72V6 stage bits). Verify toggles actually reach shaders.
