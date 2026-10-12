# E06: Full-resolution short-range contact term (Lumen-style)

- **Date:** 2026-10-10
- **Result:** rejected
- **Commit / patch:** not pushed
- **Source:** Engine chat transcript (positions 33-3359), plus git where a commit is named.

## Idea
Full-resolution short-range contact term (Lumen-style)

## Motivation
not recorded

## Paper basis
Lumen short-range AO / screen traces [W21, W22]

## What was implemented
not recorded

## Numbers
Every version made contact error worse, even scored at 640x360 close up.

Numbers context: gi-check scores (error against path-traced references) unless stated; the GPU was the RTX 4070 on zanven-pc where the chat says "on the 4070"; resolution not recorded unless stated.

## Outcome and why
Suspected double darkening of contacts the probes already shade.

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
not recorded

## Lessons / when to revisit
Revisit only with a reason it would not double-count probe occlusion.
