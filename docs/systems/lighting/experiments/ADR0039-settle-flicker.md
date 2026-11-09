# ADR 0039: static settle flicker and the camera-step snap

- **Date:** 2026-11-09
- **Result:** the tables below are the legacy baseline, measured before the cell path was allowed into the drain. This branch turns the four High and Medium bits on and, while `lighting_lamp_settling()` is true, shows the live cell (including a texel that already has samples), skips the screen-probe evaluate for that pixel, and cuts fine-texel rays when that read agrees. settle0 max and popped are compared with the legacy column. Step and snap must be no worse. Thresholds are unchanged.
- **Commit:** this change

## What was asked

Anthony, 5 Nov 2026: in a static scene with every dynamic off, the picture flickers while the light settles, then holds. One camera step snaps between the settled picture and the moving picture. Measure it with the ADR 0039 settings on and off. Defaults stay off. No threshold change.

## How it was measured

Lavapipe, Weston headless, `genos-stress --proof --no-panel --freeze-sun --still-boxes --dynamic 0 --size 160x90`, fixed dt 1/60, time of day 12, sun frozen, lamps still. `GENOS_LIGHTING_SC_TRACE=1` prints the cache, compose, and wavelet counters. It does not change the picture. `restir_di` stayed 0.

The on path sets `surface_cache.sh_cells`, `sh_texel_detail`, `sh_texel_ray_cut`, and `sh_probe_retire` to 1 through the env mirrors and the script. The off path leaves all four at 0. Preset High otherwise (denoise on, 5 iterations, screen launch, `sc_flags` text 6007050).

`mean_delta` is the mean per-pixel linear-luminance change inside an 8-frame window. `popped_share` counts pixels whose change crosses the existing flicker pop of 0.004. That number is a counter here, not a new gate. The snap is the first captured frame at the new pose against the picture held there later (`diff`, threshold 8 display codes).

Scripts: `examples/stress/scripts/settle_flicker.rhai` (one-lamp room, full view, plus one CPU path trace) and `examples/stress/scripts/settle_flicker_hall.rhai` (stress hall, bounce view, no path trace).

## Room, full view

Scene `examples/stress/scenes/room.rhai`. Eye `[0.664, 1.699, 9.629]`, yaw `-0.679`, pitch `-0.12`. The step is `+3` degrees of yaw.

| Window | Legacy | ADR 0039 on |
|---|---|---|
| settle0 mean / max / popped | 0.00337 / 0.06079 / 0.561 | 0.00335 / 0.0623 / 0.552 |
| settle2 (peak max) | 0.00342 / 0.13868 / 0.526 | 0.00336 / 0.12268 / 0.509 |
| settle11 (collapse) | 0.00011 / 0.01815 / 0.00882 | 0.00013 / 0.01773 / 0.00944 |
| held, after settle | 0 / 0 / 0, mean 0.14488 | 0 / 0 / 0, mean 0.14461 |
| step, first frames mean / max | 0 / 0.00399 | 0 / 0.00397 |
| snap mean abs / max abs / mean luminance delta | 0.451 / 8 / −0.00014 | 0.591 / 10 / +0.00024 |

`picture_settled` became true at frame 305 on both paths. The held window is exact zero. The step's worst pixel is 8 display codes (legacy) and 10 (on). `over_share` at 8 codes is 0 and 0.00014.

The CPU path trace stopped at 16 spp with noise 0.479 (the 0.08 target was not met). `mean_rel` was 0.122 settled and 0.121 on the step (legacy), 0.118 and 0.119 (on), against the same reference mean 0.530. That noise is larger than the step-to-settled gap, so the trace does not separate them.

Frame 40, both paths: about 13k texels relit, about 300k rays, compose probe evaluate 12915, probe skip 0, denoiser `changed` 0, wavelet mode `screen`, dirty 0. Frame 302: evaluate 0, relight 0, the picture is the frozen one. Frame 325 (the step) is the only on-path skip, 386 of ~12900 pixels. Ray-cut holds (`diag[38]`) peak at 4 on the on path and stay 0 on the legacy path. Frame 326 evaluate is 12842 again on both, and the wavelet stays `screen` with dirty 0.

## Hall, bounce view

Default stress hall. Eye `[14.5, 1.7, 23.5]`, yaw `-0.3554`, pitch `0.08`. Same step. Bounce view, so a direct lamp does not fill the mean.

| Window | Legacy | ADR 0039 on |
|---|---|---|
| settle0 mean / max / popped | 0.00491 / 0.172 / 0.860 | 0.00482 / 0.213 / 0.884 |
| settle window, typical | mean ~0.004, max 0.15–0.22, popped ~0.75 | the same band, max 0.226 |
| held, after the wait | 0 / 0 / 0 | 0 / 0 / 0 |
| step, first frames mean / max / popped | 0.00007 / 0.028 / 0.0078 | 0.00011 / 0.030 / 0.026 |
| snap mean abs / max abs / over 8 / mean luminance delta | 0.947 / 29 / 0.014 / −0.00090 | 0.691 / 42 / 0.0013 / −0.00036 |

The held picture is exact zero on both paths. The step's mean luminance change stays under 0.001. The diff image is a black field with a few specks. One pixel reaches 42 display codes on the on path; the share of pixels past 8 codes is smaller on that path (0.13%) than on the legacy path (1.4%).

Through the flickering windows (about frames 3–120) the on path still evaluates 13747 probes and skips 361. Ray-cut holds stay 0. Relight stays about 41k texels and about 1.0M rays, matching the legacy path (evaluate 14108, skip 0). Wavelet dirty stays 0.0000 and the mode stays `screen`.

Holds start at frame 123 (`diag[38]` 530, `diag[39]` 33920), which is the same frame the full-screen probe evaluate collapses. By frame 200 evaluate is 0 and the picture delta is already 0, with about 1170 texels held. The camera step (frame 272) brings evaluate back to 12621 for one frame (skip 1611) while about 1070 texels stay held, then holds rise to about 1600 and about 90k rays. The wavelet stays `screen`.

## Which term moves

The flicker tracks the legacy surface-cache relight and the screen-probe evaluate. Both stay at a full pass for the whole flickering window, then collapse together, and the held frames are exact copies. The denoiser's `changed` count stays 0. The wavelet dirty stays 0, including across the camera step, so the filtered field is kept (`lighting_view_held` / the idle push). ReSTIR is off, so there is no reservoir history in these runs.

SH texel detail covers an empty fine texel whose neighbour read is lit, and only while the light is quiet (`lighting_sc_light_quiet`, params bit 10). Own samples still read the probes. SH texel ray cut holds a fine texel only when the live cell read agrees and `lighting_lamp_settling()` is false. That function stays true while the drain flag is set. The `texel_ray_cut` proof names a settling lamp as a case that is not held. During the flicker the drain is open, so the hold count is 0 and coverage skips a few hundred pixels of about fourteen thousand. The probes and the relight remain the picture. After the drain closes, the settle freeze has already stopped those passes, and the new path's holds show up on a picture that is already still.

A camera step clears the freeze (`still` goes to 0) and the probe evaluate returns. In these two scenes the returned estimate matches the frozen picture within the numbers above. The field is not dropped.

## This branch

Lavapipe, same scripts, same size, High and Medium with the four bits on. The picture step is `LIGHTING_SETTLE_PICTURE_ALPHA` (1/32). Legacy numbers are the table above.

Room, full view:

| Window | Legacy | This branch |
|---|---|---|
| settle0 mean / max / popped | 0.00337 / 0.06079 / 0.561 | 0.00065 / 0.03388 / 0.11479 |
| settle2 | 0.00342 / 0.13868 / 0.526 | 0.00055 / 0.01934 / 0.13299 |
| settle11 | 0.00011 / 0.01815 / 0.00882 | 0.00012 / 0.01453 / 0.00806 |
| held, after settle | 0 / 0 / 0, mean 0.14488 | 0 / 0 / 0, mean 0.14441 |
| step, first frames mean / max | 0 / 0.00399 | 0.00001 / 0.00399 |
| snap mean abs / max abs / mean luminance delta | 0.451 / 8 / −0.00014 | 0.476 / 9 / −0.00021 |

`picture_settled` was frame 305. The path trace stopped at 16 spp, noise 0.479. `mean_rel` was 0.119 settled and 0.118 on the step, reference mean 0.530.

Hall, bounce view:

| Window | Legacy | This branch |
|---|---|---|
| settle0 mean / max / popped | 0.00491 / 0.172 / 0.860 | 0.00130 / 0.02851 / 0.16896 |
| held, after the wait | 0 / 0 / 0 | 0 / 0 / 0 |
| step, first frames mean / max / popped | 0.00007 / 0.028 / 0.0078 | 0.00014 / 0.02724 / 0.03625 |
| snap mean abs / max abs / over 8 / mean luminance delta | 0.947 / 29 / 0.014 / −0.00090 | 0.708 / 43 / 0.00201 / −0.00057 |

settle0 max and popped drop on both scenes. The room step max matches the legacy max. The room snap max is 9 display codes against 8, and the mean absolute difference is 0.476 against 0.451. The hall step pop share is 0.036 against 0.0078. The hall snap max is 43 against 29, in the same place as the earlier bits-on run (42) that still evaluated probes through the drain. Held frames are exact zeros.

## What this leaves

The baseline flicker sits in the legacy relight and the screen-probe display for the frames the old predicate left on that path. This branch changes that predicate. While `lighting_lamp_settling()` is true, compose's `lighting_settle_own_covered` accepts a texel that already has samples when the neighbour read accepts, compose returns that cell and does not evaluate probes, and `lighting_compact_hold_texel_rays` no longer returns false just because a lamp is settling or the ring reaches the texel. The cell mean rays read still takes the set whole. The picture is a short step of those sets at `LIGHTING_SETTLE_PICTURE_ALPHA` (`LIGHTING_WRC_CHANGE_ALPHA`, 1/32). The one-set colour was louder than the legacy window. A step of 0.08 left the room settle0 max above that window. `lighting_sh_texel_detail_covers` stays strict so place does not inline the ring walk. The live read is the comparison. A disagreeing texel, a stale texel, a nearby move, and an object-local texel still keep their rays. Quiet frames still show own samples and still read probes. The ray-cut proof's settling-lamp row is now a hold. The numeric gates (blotch 0.014, cell energy 0.9, linger 0.05, lag 0.9, turn 0.403) are unchanged.

The big camera-step snap is not reproduced at 160×90 on lavapipe for this room or this hall pose. A 1080p run on the owner's machine can repeat the same two scripts; lavapipe frame time is not that measurement.
