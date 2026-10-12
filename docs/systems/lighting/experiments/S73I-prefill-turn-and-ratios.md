# S73I: Turn fill-in did not speed up; the stuck-dump ratio mean is outliers

- **Date:** 2026-10-12
- **Result:** counters and the set-completion fix are in. Not measured on the 4070.
- **Commit / patch:** ffaba86, 15d3e1e

## What the 4070 run showed (PR #4 at 888a069)

Still 134 fps against 124 on the tip, moving 43 against 37. comp5 +10.6 against +10.7, so the pre-fill no longer darkens the picture. The turn sequence's near-black share matches the tip: empty texels do not fill in sooner. The stuck dump's fine-texel ratio mean is 61.2, median 1.03, p10 0.73, with 61,946 texels matched against about 41k before.

## Why the turn did not fill faster

Two gates, both in the code at 888a069.

1. **Spare budget is the only budget, and a reveal spends it.** `lighting_sc_prefill_take` places never-shown texels after the young and mature entries and stops at the base ceiling (about 55k entries). Texels scrolling onto the screen are shown, so they are the mature entries, unsettled, four entries each. While they fill the ceiling, `prefill_taken` is 0. Pre-fill then claims texels and does not trace them. The higher fps fits a run that did not add those rays.

2. **One scout hit did not finish a set.** A relight is four 16-ray quarters. The first quarter stamped LastUpdated, the due test is LastUsed > LastUpdated, and the texel left the list with its sample count still 0. The screen shows a texel's own samples only once that count reaches 1 (S72V8), so a partial quarter is still an empty texel. The guard band leads a 3 degree/frame turn by about four frames, which is not four more scout hits on the same cell.

S73J (15d3e1e) stamps LastUpdated when the set completes, so the remaining quarters keep taking spare entries without another hit. It does not spend budget the screen is already using. `GENOS_LIGHTING_SC_TRACE` now prints `prefill_listed`, `prefill_taken` and `prefill_sets` (diag 60-62). On the next turn, `prefill_taken` near 0 means the ceiling was full; `prefill_taken` high and `prefill_sets` near 0 was the quarter bug.

## The ratio mean and the extra matches

The extra matches are real cells, not a dump layout mismatch. S73E grew the keys buffer, not the cache record. S73B claims a fine texel for every scout hit within 40 m, and a still camera still scouts one round in 8, so both the moved run and the never-moved run hold the ball of cells around the end pose. The tip only held cells a pixel had shown (about 41k). The join is the cache key, which is the cell, so the same surface is the same slot. About 21k extra matches is that ball. `tools/lighting/texel_ratio.py DIR_A DIR_B FRAME` joins two dumps the same way and splits the ratios.

The mean of 61.2 with a median of 1.03 is a few enormous ratios, not the picture. comp5 agreeing (+10.6 against +10.7) says the same. Two sources, both real:

- A cell one run claimed and never traced stores luminance 0. Dividing by it (or by a first sample near 0) makes the ratio arbitrarily large. The matched set now includes those never-shown cells.
- A pre-fill set skips the MSME firefly cap (that cap runs only on the estimator path, which starts once a pixel has shown the texel). One bright set against a normal one is a large ratio.

p10 0.73 is the dark tail of the ratios that are actually finite. It is not what pulls the mean to 61.

## Lavapipe

`examples/stress/tests/gi_prefill.rs` (ignored; `tools/lighting/headless.sh`) settles the small hall, dumps the cache, turns half a radian, and dumps again. A comma-separated `GENOS_LIGHTING_DUMP` copies those two frames and not their neighbours. The test requires never-shown fine texels with a finished set before the turn, and some of those same keys shown after it. That is the fill the 4070 turn sequence did not show. A miss with `prefill_taken` near 0 is the full ceiling; a miss with quarters stuck under 4 is the set still not finishing.
