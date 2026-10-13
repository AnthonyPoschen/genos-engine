# WAKE: Anthony's wake design: no wakes from shared field; real change wakes neighbours by coarse-to-fine ring tests with group-test splitting; always-on stratified spread-out checks

- **Date:** 2026-10-12
- **Result:** implemented behind `GENOS_LIGHTING_WAKE=rings` (default remains the legacy check ray)
- **Commit / patch:** this commit

## Idea
Anthony's wake design: no wakes from shared field; real change wakes neighbours by coarse-to-fine ring tests with group-test splitting; always-on stratified spread-out checks

## Motivation
not recorded

## Paper basis
Ward/Krivanek IC error bound; A-SVGF sparse checks; Lumen background budget; progressive radiosity (Cohen 1988); hierarchical radiosity (Hanrahan 1991); group testing (Dorfman 1943); galloping search. Combination is original (documented deviation).

## What was implemented
`GENOS_LIGHTING_WAKE=rings` (default `legacy`). A lamp move or a geometry change queues up to eight events for eight frames. The place pass tests one probe per sector of a ring at r, 2r, 4r, … and splits 4 → 8 → 16 sectors when the group changed by more than the tolerance. A failed ring fully traces a quarter of the probes inside it; probes inside the first radius are traced in full. A negative radius (sun, lamp added or removed, colour change above 0.75 relative, or a change list that overflowed) traces every placed probe once. Verification retraces `wake_budget` probes a frame (default 16) and the CPU tightens the tolerance on a miss, down to a quarter of the setting. `GENOS_LIGHTING_WAKE_TOL` (0.08), `GENOS_LIGHTING_WAKE_BUDGET`, `GENOS_LIGHTING_WAKE_RADIUS` (8 m). Counters: rings tested, groups split, probes woken, verification hits and misses, probes traced, rays traced (`lightingwake` under `GENOS_LIGHTING_DEBUG`). A settled rings frame still runs the probe passes. Legacy is unchanged, including the settled idle.

## Numbers
Legacy baseline, before this scheduler: [WAKE0](WAKE0-legacy-trace-baseline.md). Lavapipe, 320×180 shipped hall: a held still frame traced 1862 probes and 1862 rays (one check ray each); a moving camera traced 2089 probes and 133696 rays (64 per probe).

Rings, same window and hall, budget 16: frames 0–7 still trace the stratum (1862 probes, 119168 rays) because the strata are not held yet. From frame 8, a still frame traces 9 probes and 9 rays, wakes 0, and tests 0 rings. That is the verification budget (16 slots, about half placed). 9 against 1862 is about a 200× cut in traced probes on a held still frame.

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
not recorded

## Notes from chat
Anthony (2026-10-12 22:50, t453u) asked whether the constant probe searching plus fan-out technique would be worth publishing if it works and performs well. Answer not in the available transcript.

## Lessons / when to revisit
After it lands, retest S77C, S78G, S78H, S78L, S78N, S78R.
