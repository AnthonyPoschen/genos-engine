# E23: Profiling at the checkpoint: how much traced work changes the picture

- **Date:** 2026-10-12
- **Result:** finding
- **Commit / patch:** none (worktree ~/.cache/ge-perf at 6b4fe308)
- **Source:** Engine chat transcript (positions 33-3359), plus git where a commit is named.

## Idea
Profiling at the checkpoint: how much traced work changes the picture

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
1280x720 headless: static settled scene lighting 22.9 ms of a 23.4 ms frame; every ray still traced changes the picture by under 2%; rest of frame ~0.45 ms. Moving scene settled: ~72% of rays change the result by under 5%; over half are hint-driven and 97% of those wasted.

Numbers context: gi-check scores (error against path-traced references) unless stated; the GPU was the RTX 4070 on zanven-pc where the chat says "on the 4070"; resolution not recorded unless stated.

## Outcome and why
Proposed: (1) idle skip, (2) A-SVGF adaptive rays, (3) smooth only changed pages, (4) cache direct light.

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony (t366u-t372u): measure the dynamic scene for waste; a static scene should reach 1000+ fps; a cheap continuous research-based check still has to run. Decisions (t373u): never 'skip' lighting, always verify with an extraordinarily cheap check; adaptive rays yes; smoothing only changed pages doubtful (try separately later, t374u); direct-light cache only as shadow reuse for static lamps. (t376u-t378u) cache direct light Lumen's way; (Q t378) 'Follow Lumen exactly: cache feeds bounce-light rays only, screen stays per-pixel'.

## Lessons / when to revisit
Lumen does not adapt effort to measured change; A-SVGF does (for history only).
