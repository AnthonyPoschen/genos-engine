# S78X2: Dense a-trous when the dirty fraction is high

- **Date:** 2026-10-12
- **Result:** pushed (not re-timed; the owner measures on the RTX 4070)
- **Commit / patch:** this commit

## Idea
Fall back to the dense full-frame a-trous when the dirty fraction exceeds a threshold.

## Motivation
S78X sped up a still scene and regressed a moving one (owner, RTX 4070, 1920x1080): a-trous 2.3 -> 1.8 ms still, 6.6 -> 9.4 ms moving, 42 -> 37 fps.

## Paper basis
Same filter as S72V / S78X. The choice is which texels the dispatch visits.

## What was implemented
Levels 1-5 keep S78X's work lists while last frame's fullest level was at or under `GENOS_LIGHTING_ATROUS_DIRTY` (default 0.25 of the filtered texels). Above that they skip the list and run the dense slot sweep (clean stamps still return inside the level). `0` is always dense. `1` is the S78X lists until the 65536 cap.

The list is built with one contended `atomicAdd` per dirty slot, in the filter, and the dispatch then reads slots in append order. A still scene is ~5-18k dirty of ~108k filtered, so the million threads return after one uniform read. A moving scene dirties most of that set: the atomic compaction stays on the critical path, and the scattered slots lose the coalescing of the dense sweep. The old cap fell back only after the list had already been built, and only past 65536. S78A was rejected for that locality cost.

`GENOS_LIGHTING_DEBUG` or `GENOS_LIGHTING_SC_TRACE` prints `lightingatrous` with the dirty fraction, the mode (`list` or `dense`), the threshold and the filtered count. `Renderer::lighting_atrous_stats` reads the same words. The lavapipe test forces threshold 1 and 0, checks the mode switch and the counters, and checks the two pictures match within the same-mode frame step.

## Numbers
Not re-timed here.

## Screenshots
not recorded

## Outcome and why
The still-scene list stays the default. The moving scene takes the dense sweep once a level's dirty fraction has exceeded the threshold for a frame.

## Lessons / when to revisit
If a settled still scene's fullest level sits near 0.25, raise the default. The owner's A/B is `GENOS_LIGHTING_ATROUS_DIRTY=1` (S78X lists), `=0` (always dense), and unset or `=0.25` (this fallback).
