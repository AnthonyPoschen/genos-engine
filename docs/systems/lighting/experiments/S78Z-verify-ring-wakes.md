# S78Z: Always-on probe verification + impact-tested ring wakes

- **Date:** 2026-10-13
- **Result:** built, not pushed (did not fix the stuck patches; filter cost +0.6 ms). Patch g78/S78Z-verify-ring-wakes.patch
- **Commit / patch:** g78/S78Z-verify-ring-wakes.patch

## Design (original; Anthony's rules)
- Rotating, spatially stratified verification: one probe in every 4 x 4 block of the probe grid per frame (block position cycling over 16 frames) relights one held stratum (cycling over the 8) against the current caches. A woken probe relights all its strata.
- Surface-impact test in the gather: the probe's summed hit-light change over all its kept hits (its irradiance change as the surface sees it, albedo <= 1) against the probe's surface texel's own noise bound (3 SE of its mean; 3 % of its value when the estimator is not running). Within: the relight is kept quietly, no chain. Beyond: 4 probes on a golden-angle ring of radius R0 x impact / bound (at most half the previous hop's radius, so rings shrink) are woken for the next frame.
- Each probe wakes at most once per event (event-id stamp, atomic exchange); per-frame wake cap 2048 (overflow left to the rotating verification).
- Counters: chg slots 21-29 (vTests, vFails, wSeeded, wCapped, wTests, wFails, wDedupe, vRelit, wHops).
- Cites: A-SVGF sparse re-samples (Schied 2018), Lumen's background budget, progressive-refinement radiosity (Cohen 1988), adaptive group testing (Dorfman 1943), hierarchical radiosity (Hanrahan 1991).

## Repro (stress scene, z10/stuck.sh + stuckref.sh)
Walk and turn from (6, 1.7, 30) yaw 2.1 to the bench view (11.59, 1.7, 23.52) yaw -1.0 pitch -0.25 over 90 frames, hold still, capture at +10 / 60 / 180 / 480 / 1080 frames; reference: start at the bench view and wait 1410 / 2910 frames. Sun frozen, boxes still, dynamic 0.

## Numbers (rough, steady gaming load)
- Still scene: vTests 506 / frame, vFails 0 (no wakes); 21k kept hits change per frame (mostly < 1 %); 511 gathers re-run; filter 0.55 -> 1.13 ms, still total 6.41 -> 7.33 ms. Smoke a 0.349.
- Stuck repro: the moved view vs reference stays as before (t11: |d| 10.6 vs 10.0 without S78Z; 41 % of pixels > 12 levels apart).

## Postmortem
The stuck patches are not stale probes. Verification relit every held hit several times within the 1080 frames and found nothing beyond noise, and the picture did not move. The difference image (moved minus reference) is a broad, smooth brightness offset (+9 levels mean, up to ~30) over the region the camera passed through, including surfaces lit mostly by bounce; the reference is stable between 1410 and 2910 frames (|d| 0.44). So the surface/world cache settles into two different states depending on camera history: texels that settle while their neighbours' bounce is still rising keep a biased mean (the stop rule tests noise, not bias), and the ray budget favours texels near the camera's path. Raising the settled relight rate (240 -> 30 frames) did not change it either (one new set barely moves a mean of up to 64 sets). Next: texel-level verification (re-light settled texels on a rotating schedule and reset the long mean when the new sets disagree beyond their noise), not probe-level.
