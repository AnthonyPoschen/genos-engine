# RAYCEIL: World-cache ray count is a per-cell ceiling from an A-SVGF check

- **Date:** 2026-10-12 (idea); measured 2026-10-19
- **Result:** rejected (not pushed)
- **Commit / patch:** PR #8 tip `42d480a6` on `cursor/wrc-need-ray-ceilings-a7d6` (closed; branch deleted). Not on master.

## Idea
Each world-cache (WRC) cell's rays are a ceiling set by how much a cheap A-SVGF check disagrees with a stored ray. A steady scene spends almost nothing; the refresh budget is a ceiling, not a target.

## Motivation
About half of moving cache rays went to cells growing toward a fixed 128-sample cap. S78F already turned age-only full sets into deferred checks; RAYCEIL was meant to replace both the age check and growth-to-cap with one same-seed check plus proportional extra rays.

## Paper basis
A-SVGF (Schied, Peters, Dachsbacher 2018, eq. 6–7; Schied et al. 2019). SHaRC hashed world cells and Lumen's world-space radiance cache (fixed update budget, changed and nearest first) for where the rays live. The queued title said Ward / Krivánek irradiance-cache error bound; cells do not keep per-ray hit distances, so the bound used is the A-SVGF noise test.

## What was implemented (on the PR branch only)

Past 16 samples, a cell retraces one stored ray when its check is due. Within noise: no extra rays, next check deferred (8 frames, doubling to 128). Beyond noise: lambda goes to the cell and its 3×3 on the 25 cm grid, and the next round shoots `ceil(4 * lambda / 0.25)` sixteens (16 to 64 rays). Escalations first under the existing 4096-entry ceiling. Young cells that miss the reveal burst share 64 sixteen-ray entries a round. A still camera shoots 4 scout entries a round instead of 256 every eighth round. Prefill off (`ctrl_bits` bit 1024) still disables scouts. No new buffer, no CPU readback in the frame loop.

Deviations (also in the PR commit message): lambda sets ray count (A-SVGF uses it for the blend); the check ray is not accumulated; the 3×3 is the world grid; the noise test stands in for the IC error bound; Lumen/SHaRC spend a fixed world-cache budget every frame.

## Numbers

Exclusive zanven-pc RTX 4070 grant, Mon 19 Oct 2026 ~15:33–15:40 AEDT (outside the Fri–Mon clean window; Anthony granted exclusive use). Stress 1920×1080 `--proof` HDMI-A-1 workspace 6 silent, MCP 8814. BASE `5f9652f6` (master after #23). HEAD `42d480a6` (PR #8 after master merge). Full table: `/home/box/shared/p8-ab-results.md`.

| Metric | BASE still | HEAD still | BASE moving | HEAD moving |
|---|---:|---:|---:|---:|
| mean lightingsc rays (f200–550 / f200–580) | 73185 | 68259 | 320562 | **161388** |
| early still rays (f30–150) | 976595 | **223072** | — | — |
| mean scouts (same window) | 256.7 | **31.2** | 516.0 | **64.0** |
| GPU_MS frame ms (last 3) | 0.760 | 0.755 | **21.554** | **22.907** |
| GPU_MS lighting ms (last 3) | 0.422 | 0.421 | **21.201** | **22.550** |
| pop f20–80 / f80–320 | 0.076% / 0.142% | **0.020% / 0.092%** | — | — |
| pose-a mean luminance | 0.397 | 0.385 | — | — |
| VRAM mid-run | ~1619 MiB | ~1619 MiB | ~1619 MiB | ~1619 MiB |

Numbers context: still/moving ms from the new `GPU_MS` line (`--gpu-times 30`); rays/scouts from `GENOS_LIGHTING_SC_TRACE` `lightingsc` means over the frame window; pop from `popscore.py` on bounce captures; brightness is smoke mean luminance at pose a.

## Screenshots

not recorded (logs under `~/.cache/gi2diag/perf/ab-*-P8{BASE,HEAD}` on zanven-pc)

## Outcome and why

**Rejected.** Still early rays dropped ~4.4× and scouts trickled as designed; mid-still settled frame times matched; pop improved. Moving rays roughly halved, but **moving frame and lighting rose ~+1.35 ms** (~21.6 → 22.9 ms frame). Plan pass criteria required frame/lighting not worse. Likely the check/escalation path costs more GPU time than the ray cut saves under moving load (fewer rays, higher `light` / `atrous` wall time). Standing rule (Anthony 19 Oct): measured badly → research note, close PR, delete branch; do not implement a fix on the PR.

## Lessons / when to revisit

- Ray count is not a proxy for lighting ms when the remaining rays are more divergent or the check path adds dispatches/atomics.
- If revisiting: profile which `LIGHTING_MS` pass grew on HEAD moving before changing trickle/check deferral; measure only in a clean-GPU window (or another exclusive grant).
- Related open plan item: dynamic-perf plan Item C still cites RAYCEIL as related WRC pacing — treat that as historical; do not assume this branch will land.
