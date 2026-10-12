# S79: World-cache ray count follows the measured change

- **Date:** 2026-10-12
- **Result:** pushed (GPU numbers not yet; compile-checked only)
- **Commit / patch:** this branch

## Idea

The world radiance cache's per-frame ray count is a ceiling set by how much each cell's results have been changing. A cheap same-seed check that lands close to the stored ray spends no further rays and defers the next check. A change past the cell's own noise spends rays in proportion to that change. Low-priority growth trickles. A steady cache spends almost nothing, and it is verified rather than skipped.

## Motivation

About half of the moving cache rays were cells growing toward 128 samples on whole 64-ray sets, plus a 16-ray check on any nominated or stale cell, plus scout rays one round in eight while the camera was still. The budget was filled because cells were listed, not because a measurement asked for the rays.

## Paper basis

- A-SVGF (Schied, Peters and Dachsbacher, HPG 2018, eq. 6-7; Schied et al. 2019, gradient divided by the sample's own variance).
- SHaRC stale-entry refresh and Lumen's world-space radiance cache (fixed update budget, changed and nearest first). The structure kept from those is the hashed world cell and the running mean. The fixed per-frame spend is the deviation.

## What was implemented

- One stored ray per cell (the same slot the texel check uses). The check re-traces that ray. `lambda = max(0, d - 2 sigma)` with the texel noise cap.
- Confirm (`lambda < 0.05`): no extra rays, next check waits twice as long, from 8 frames up to 128.
- Change: the cell's ceiling is `ceil(4 * lambda / 0.25)` entries of 16 rays. The same lambda is written to the 3x3 block, which is the region.
- `LIGHTING_WRC_REFRESH` (4096 entries) is a ceiling. The dispatch uses the listed count.
- Cells under the 128-sample cap that were read in the last 60 frames trickle one 16-ray entry every 16 frames, at most 64 entries a round. When more are due, service rotates by hash.
- A cell with no stored ray yet takes one 16-ray entry to create the baseline, then joins the check.
- A mover, a moved lamp, or a global-light move of 5% or more only nominates the check.
- Scout rays run only while the camera moves or turns. The reveal burst (a pixel needs a cell under 16 samples) is unchanged.
- No new buffer and no CPU read of the lists in the frame loop.

## Deviations

- Lambda sets the ray ceiling. A-SVGF uses it for the temporal blend only.
- The check ray is measurement only. A-SVGF keeps it as a sample.
- Lambda is copied across the 3x3 block. A-SVGF filters the gradient.
- The 0.05 and 0.25 thresholds are the texel ones. The 16-frame trickle and the 64-entry trickle cap were chosen here.
- A still camera spends no scout rays. Lumen and SHaRC keep a fixed world-cache budget every frame.
- Not this change: stopping at an irradiance-cache error bound instead of 128 samples (RAYCEIL, still queued).

## Numbers

not recorded (no GPU in this run)

## What to measure

On a settled still stress scene, after the one-time baseline (one 16-ray entry per cell that had no stored ray): world-cache rays per frame should be the deferred one-ray checks, on the order of resident cells / 128, plus a trickle only for cells still under 128 samples and read in the last 60 frames (at most 64 x 16 rays). Scout rays should be 0 while the camera is still.

On a moving-light or moving-object run: checks rise where the light reaches, and the extra rays should track lambda (16, 32, 48, or 64) rather than a flat 64. The 4096-entry ceiling should not be full unless a large share of cells actually changed.

`gstats.rays` counts world-cache sample rays and world-cache check rays (not texel checks). With `GENOS_LIGHTING_SC_TRACE`, diag 16 is baseline entries, 17 is trickle entries, 44 is checks queued, 45 is escalation entries. Compare still and moving frame time, pop, and brightness with the previous build. VRAM should be unchanged.
