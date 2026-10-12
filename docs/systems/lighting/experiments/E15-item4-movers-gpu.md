# E15: Item 4: object-local texels for moving/spinning boxes

- **Date:** 2026-10-11
- **Result:** pushed as default with ba9adca0; targets still failing
- **Commit / patch:** f13d56ba, ba9adca0
- **Source:** Engine chat transcript (positions 33-3359), plus git where a commit is named.

## Idea
Item 4: object-local texels for moving/spinning boxes

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
First run spin 0.088 (limit 0.027), moving boxes 0.0084 (0.0055); local texels 0.088 -> 0.095 then 0.084; box-face error 0.016 -> 0.010; shorter history next to the box: spin 0.086 -> 0.072, flicker 0.046 -> 0.040.

Numbers context: gi-check scores (error against path-traced references) unless stated; the GPU was the RTX 4070 on zanven-pc where the chat says "on the 4070"; resolution not recorded unless stated.

## Outcome and why
Mover data was never uploaded to the GPU (S52), so none of the mover handling had ever run.

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony (t171u): S49 at ~20 fps, 'looks pretty good visually, a massive improvement'; (t187u) 'everything was looking good'; (t188u) make it the default config so plain make stress works; (t193u) 'just git push' (ba9adca0, pushed untested).

## Lessons / when to revisit
not recorded
