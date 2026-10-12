# E02: Probes keep 512 hits (8 rotating strata of 64) and re-light them every frame; no reset on lamp/sun/sky change

- **Date:** 2026-10-10
- **Result:** pushed
- **Commit / patch:** ae829f41
- **Source:** Engine chat transcript (positions 33-3359), plus git where a commit is named.

## Idea
Probes keep 512 hits (8 rotating strata of 64) and re-light them every frame; no reset on lamp/sun/sky change

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
Moving blob 0.035 -> 0.018 (target 0.02); mean error 0.054 -> 0.047; added a 2-4% brightness bias in corners and contacts.

Numbers context: gi-check scores (error against path-traced references) unless stated; the GPU was the RTX 4070 on zanven-pc where the chat says "on the 4070"; resolution not recorded unless stated.

## Outcome and why
Halves moving blotching.

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony (t15u-t19u): looks good when still; small artefacts behind moving objects; the big one is splotches swimming while the camera moves, 'the moment the camera is stable it instantly goes good'; uncovered areas behind moving objects are splotchy; treat what is behind a dynamic object as important (keep it warm).

## Lessons / when to revisit
Thin history is one problem with two triggers (camera moves, uncovered surfaces).
