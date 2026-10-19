# S78X: A-trous levels run work lists of dirty texels

- **Date:** 2026-10-13
- **Result:** pushed (rough A/B under steady gaming load; idle A/B queued)
- **Commit / patch:** this commit; first try (page skip + noise-aware level 0) g78/S78X-atrous-pages-noise.patch

## Idea
S77D already recomputes only dirty texels per level, but each of levels 1-5 dispatched 1M threads that each read their slot's key, flags and dirty stamps. Counters (debug run): still scene, ~108k active texels, ~310 level-0 recomputes, dirty texels per level 5.6k / 10k / 12.6k / 16k / 18.5k. The first try skipped 64-slot pages with no dirty slot: no gain, since texels are hashed across slots and 18k scattered marks touch nearly every page.

## What was implemented
`lighting_svgf_mark` appends a slot to its level's work list once a frame (atomic exchange on a per-slot, per-level stamp in cache_keys; +20 MiB fixed), and levels 1-5 run only the listed slots (threads past the count return after one uniform read). The compact pass empties the lists each round. A list past 65,536 entries falls back to the full sweep. The noise-aware level-0 threshold (2 SE) is in the code but off (`LIGHTING_DIRTY_NOISE`): it shifted smoke brightness 0.348 -> 0.345 (held values lag toward darker while texels converge upward).
The cache_keys buffer also reserves the S78Z verify/wake words (next commit) and chg slots 21-29 are renamed to the S78Z counters.

## Numbers (rough, steady gaming load; RTX 4070, stress, 1080p, alternating)
| | S78Y | S78X |
|---|---|---|
| atrous (7 dispatches) | 2.60 / 2.62 ms | 2.23 / 2.33 ms |
| still lighting total | 7.05 / 6.86 ms | 6.39 / 6.39 ms |
Smoke a 0.348; pop 0.096 % / 0.132 % (S78Y 0.098 / 0.139).

## Screenshots
shad-S78X3/b.png viewed (mean abs diff vs S78Y 0.76/255).
