# E22: Kept probe hits relit only on their stratum's frame plus one rotating stratum (S69C)

- **Date:** 2026-10-12
- **Result:** pushed
- **Commit / patch:** 2f35eaa2
- **Source:** Engine chat transcript (positions 33-3359), plus git where a commit is named.

## Idea
Kept probe hits relit only on their stratum's frame plus one rotating stratum (S69C)

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
Default stress scene, headless 1920x1080, indoor walk/stop/turn with night->day: frame ms S69B -> S69C stopA 34.3 -> 29.3 (28.4 -> 33.0 fps), walkAB 32.7 -> 28.1, stopB 31.3 -> 26.6, turn 29.5 -> 25.0 (32.8 -> 37.8 fps), night->day 32.9 -> 27.8, walkBC 35.1 -> 29.3, stopC 35.8 -> 29.7; light pass ~5 ms lower in each.

Numbers context: gi-check scores (error against path-traced references) unless stated; the GPU was the RTX 4070 on zanven-pc where the chat says "on the 4070"; resolution not recorded unless stated.

## Outcome and why
About +15% fps everywhere with no visible change.

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
not recorded

## Lessons / when to revisit
not recorded
