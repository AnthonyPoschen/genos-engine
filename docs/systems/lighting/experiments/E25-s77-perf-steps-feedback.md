# E25: S76B-S77 one-at-a-time perf steps and Anthony's verdicts

- **Date:** 2026-10-12
- **Result:** record
- **Commit / patch:** dbb4777a..9e242987
- **Source:** Engine chat transcript (positions 33-3359), plus git where a commit is named.

## Idea
S76B-S77 one-at-a-time perf steps and Anthony's verdicts

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
not recorded

## Numbers
78637784 -> dbb4777a moving 29.2 -> 28.6 ms (33 -> 34 fps); cache rays 96k -> 125k (re-scheduled WRC cells ~27k). dbb4777a -> c146bf1c moving 28.4 -> 27.9 ms (34 -> 35 fps), light pass 11.2 -> 10.6 ms. Memory: surface-cache buffer ~368 MB after S77D (was ~240 MB); lighting total ~630 MB at 1080p.

Numbers context: gi-check scores (error against path-traced references) unless stated; the GPU was the RTX 4070 on zanven-pc where the chat says "on the 4070"; resolution not recorded unless stated.

## Outcome and why
not recorded

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony (t380u-t385u): barely notices gains; something keeps the budget busy; A-SVGF should grade change, a cheap ray that comes back close keeps the area quiet. (t386u) after S76 sun sweep left skipped spots between regions (-> ring nominations, S76B-D). (t387 answer): build cheap WRC check, per-pixel shadow caching, dirty-page smoothing and gather/compact-only-changed, one at a time; (t394u-t397u) put them straight on gi-v2, quick passes. (t399u/t400u) moving shadows jittery torn edges -> S77B reverted. (t401u) 'very disappointed these changes did not drastically increase frame rate'. (t402u-t405u) cap rays by need, stop refining within tolerance, want 1000s fps and spend free compute on drastic changes. (t415u/t416u) keep lighting VRAM roughly fixed but adding memory is fine if it cuts compute. (t417u) suspects rays shot at stale data because budget allows. (t419u/t420u) his own fps on c660c0db mostly 20-30 with moments at 52; keep tests consistent. (t422u) low-priority needed work stays queued at a small fixed rate. (t423u) moving scene 0 improvement, still 80 fps, no visual issues.

## Lessons / when to revisit
not recorded
