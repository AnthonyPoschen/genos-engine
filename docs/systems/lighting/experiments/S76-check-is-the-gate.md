# S76: Variance-aware lambda, deferred checks, lambda-proportional rays, hints only nominate

- **Date:** 2026-10-12
- **Result:** pushed
- **Commit / patch:** 78637784

## Idea
Variance-aware lambda, deferred checks, lambda-proportional rays, hints only nominate

## Motivation
not recorded

## Paper basis
A-SVGF graded checks

## What was implemented
not recorded

## Numbers
not recorded

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
not recorded

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Moving scene d591abfa -> S76: cache rays per frame 621k -> 561k (f20-120), 242k -> 154k (f120-300), 232k -> 96k (f300-610); texels relit 3,630 -> 1,486; still 24.5/24.0 ms (39-40 fps); moving 29.8 -> 29.4 ms (33 fps), light 12.7 -> 12.5, trace 5.1 -> 4.8; pop 0.087/0.114 -> 0.079/0.107 %; brightness .350/.498/.504/.537/.529. Confirmations defer the next check 8 -> 128 frames; escalation rays scale 16..64 with lambda. Why fps barely moved: the cached direct light hit its 32,768-texel limit every frame under moving lamps. Anthony (t386u): during the sun sweep some spots between regions were skipped and artefacts lingered until a check landed on them.

## Lessons / when to revisit
not recorded
