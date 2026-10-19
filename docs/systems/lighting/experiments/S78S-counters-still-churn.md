# S78S: Debug counters: what keeps still probes awake

- **Date:** 2026-10-12
- **Result:** pushed (debug tooling); finding below
- **Commit / patch:** S78S commit (this one); S78R/S78T code present but off

## Idea
Debug counters: what keeps still probes awake

## Motivation
not recorded

## Paper basis
not recorded

## What was implemented
Debug only; nothing here runs in the normal frame loop.
- **Change counters** (`GENOS_LIGHTING_SC_TRACE=1`): `gstats.chg[32]` after `diag[64]`
  (stats buffer 480 bytes). The host sets the enable word `chg[31]` with a buffer
  fill; the counts come back through the existing asynchronous stats readback, one
  `lightingchg frame N c [...]` line per frame. `perf/chg.py RUN_DIR` labels them
  (slots `CHG_*` in lighting_cache.glsl). They cover kept-hit light writes and
  changes, a histogram of the relative delta (<0.2 %, <1 %, <5 %, >5 %), the source
  of the hit's direct light (TDL reused / checked / lamps) and of its bounce (fine
  texel, coarse patch, parent, neighbours, none), kept records re-traced,
  surface-cache value writes with the same histogram, and per probe why its gather
  ran (S78R inputs, moved, foreign strata).
- **Dump** (`GENOS_LIGHTING_DUMP=N`, `GENOS_LIGHTING_DUMP_DIR`): the cache, cache
  keys, work, G-buffer and output buffers are copied into readback buffers at
  the end of frames N and N + 1 and written to disk a few frames later
  (`f{F}_{name}.bin` plus `f{F}_layout.json`). `perf/dump.py DIR N` splits them
  into one file per component and channel:
  - surface-cache fine and coarse CSVs plus slot atlases (value, sets, variance,
    TDL direct, last-use age, dirty-stamp age);
  - probe irradiance, own strata and a kept-source CSV;
  - per-pixel direct light;
  - N -> N + 1 diffs: slot delta atlases, a top-down map of >5 % flips, a
    probe changed mask and count, and a pixel direct delta. It also prints which
    stratum and ray the changed kept hits are.
  `z10/dump.sh BIN TAG FRAME [TOGGLES]` runs the still scene and decodes.
- **Isolate toggles** (`GENOS_LIGHTING_TOGGLES`, or live through the file named by
  `GENOS_LIGHTING_TOGGLES_FILE`, re-read every frame): `freeze_cache` (no
  surface- or world-cache value writes), `direct_only` (flag 2097152: no cache
  light at ray hits), `bounce_only` (no direct light at pixels or hits) and
  `freeze_probes` (kept-hit lights not rewritten). They ride in head 3.w bits
  12-15.

The first use (still scene, frames 400 and 401) found the following:
- Every kept-hit light that changed (4,449 a frame; 55 % of probes) is the S78K
  held probe's one check ray. It is relit fresh each frame, while the probe's
  other held hits keep light from their last relight, so the drift of the
  converging cache shows up as a "change" of one hit in every probe.
- Coarse-patch flips >5 % (162 a frame) are scattered, slots at about 37 sets
  each taking one more set: convergence noise, not a cascade.

## Numbers
Still scene/frame: ~145k kept hits rewritten, ~4,100 changed (>5%) across ~4,100 of 8,100 probes; ~740 texels written. Disabling the surface-cache read: changed hits ~6,500 -> ~300, quiet probes 7,780/8,093. Ignoring <2% change: quiet 20% -> 49% (timing spoiled by GPU contention).

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Main cause: probe hits read surface-cache texels that keep converging (decays, not a loop, but not settled by 600 frames). The coarse-patch mystery is solved by the dump (frames 400/401): every changed kept hit is the held probe's check ray (stratum = this frame's, k = lighting_hold_check_k: 100 %), relit fresh while the probe's other held hits keep their last relight, so cache drift shows as one changed hit per probe (4,449 probes, salt-and-pepper mask, no spatial clusters). Coarse slots flipping >5 % (162/frame) are scattered converging slots (~37 sets, +1 set): noise, not a cascade.

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
Anthony (t445u/t446u): dump GPU data (lighting maps as textures etc.) and make each component separable and isolatable. (t448u-t451u): papers vs our refire logic; always-on spread checks; no wakes from the shared field; ring-test group-split wakes (approved).

## Lessons / when to revisit
Fix: per-texel IC-style stop rule; no wakes from the shared field. Dump tool planned to find the coarse flips.
