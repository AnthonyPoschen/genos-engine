# RAYCEIL: World-cache ray count is a per-cell ceiling from an A-SVGF check

- **Date:** 2026-10-12
- **Result:** pushed
- **Commit / patch:** this change

## Idea
Each world-cache cell's rays are a ceiling set by how much a cheap check disagrees with the stored ray. A steady scene spends almost nothing, and the refresh budget is not filled up.

## Motivation
About half of moving cache rays go to cells growing to 128.

## Paper basis
A-SVGF (Schied, Peters, Dachsbacher 2018, eq. 6-7; Schied et al. 2019, gradient divided by its own variance). SHaRC's hashed world cells and Lumen's world-space radiance cache (fixed update budget, changed and nearest first) for where the rays live. The queued title said Ward / Krivánek irradiance-cache error bound; cells do not keep per-ray hit distances, so the bound used is the A-SVGF noise test.

## What was implemented
Past 16 samples, a cell retraces one stored ray when its check is due. Within noise: no extra rays, next check deferred (8 frames, doubling to 128). Beyond noise: lambda goes to the cell and its 3×3 on the 25 cm grid, and the next round shoots `ceil(4 * lambda / 0.25)` sixteens (16 to 64 rays). That list is served first, under the existing 4096-entry ceiling, and the ceiling is not a target. Cells under 16 samples that something read, and that missed the reveal burst, share 64 sixteen-ray entries a round. A still camera shoots 4 scout entries a round instead of 256 every eighth round. Same gradient slots as the texel checks. No new buffer, no CPU readback in the frame loop.

Deviations, also in the commit message: lambda sets the ray count (A-SVGF uses it for the blend); the check ray is not accumulated (A-SVGF reuses the sample, SHaRC accumulates); the 3×3 is the world grid, not screen pixels; the noise test stands in for the irradiance-cache error bound; Lumen and SHaRC spend a fixed world-cache budget every frame.

## Numbers
not recorded (measure on the owner's GPU; this machine does not run the frame loop for numbers)

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Implemented. Still and moving cost, pop, and brightness were not measured here (no GPU frame loop on this machine).

## Lessons / when to revisit
If a still scene still spends a large world-cache ray count, the reveal burst or the young-cell trickle is the place to look, not the check. If bounce stays dark in occluded rooms, the check is not seeing the late bounce (same-seed ray agrees while the mean is biased) and the cell has already passed 16 samples.
