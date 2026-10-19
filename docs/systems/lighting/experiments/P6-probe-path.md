# P6: Screen probes (16 px tiles, 64 rays/frame in 8 strata), kept hits re-shaded every frame, Lumen-style probe filter, hashed light cache in push mode

- **Date:** 2026-10-10/11
- **Result:** pushed (later default off; `GENOS_LIGHTING_WORLD=0`)
- **Commit / patch:** 6ce7fd48 and earlier (ae829f41, a6432375, b5fa8eb7, 1b1101e2)

## Idea
Screen probes (16 px tiles, 64 rays/frame in 8 strata), kept hits re-shaded every frame, Lumen-style probe filter, hashed light cache in push mode

## Motivation
not recorded

## Paper basis
Lumen screen probes [W21]; DDGI/SHaRC for the light cache; ReSTIR GI for visibility reuse

## What was implemented
See README sections Screen probes, Kept hits, Probe filter, Light cache.

## Numbers
Settled mean 0.033-0.042 per view; room 48 px swim 0.0025 (MISS), structure 0.0143 (MISS), close-up 0.0106 PASS; lights-only return 0.0001-0.0002 PASS. fps/ms not recorded.

Numbers context: the scene, GPU and resolution are not recorded for these figures unless stated above. For S76B-D onward, still/moving ms and fps are the bisect-log (steps.md) still and moving runs; pop is the share of pixel-frames stepping more than 0.01 from their 9-frame average (definition from the gi-check fill run), at frames 20-80 / 80-320; brightness is smoke mean luminance at pose a ("lum a"). Anything not given here was not recorded.

## Screenshots
not recorded (under ~/.cache/gi2diag/perf on zanven-pc)

## Outcome and why
Bounce swims while the camera turns because the probe grid slides with the screen (Anthony); replaced as default by the surface cache at S54K.

## From the Engine chat (Anthony's feedback, reasons, numbers stated)
See E02-E09 for the Oct 10-11 fixes on this path.

## Lessons / when to revisit
Screen-anchored data swims; world-anchored texels fixed that. Probe path still exists as an A/B mode.
