# QUIET: Compose redraws blocks whose inputs changed

- **Date:** 2026-10-12
- **Result:** pushed (compose only; filter and compact already have their own quiet paths)
- **Commit / patch:** this commit

## Idea
The compose pass keeps the previous picture and redraws an 8×8 block only when an input that block reads has changed since the last compose.

## Motivation
Compose was about 2.0 ms in a still 1920×1080 frame with a static camera and static lighting, a fixed cost on top of the passes that had already gone quiet.

## Paper basis
SVGF and Lumen composite every pixel every frame. Skipping unchanged pixels is the same idea as the dirty-page a-trous already in this lighting (S77D, S78X): recompute where an input moved, keep the stored value elsewhere. No paper specifies this compose skip.

## What was implemented
Producers compare the value they write with the value they stored and append the 8×8 block to a GPU list (one stamp per block, so a block is listed once):

- direct light, when the surface point, the packed direct light, or the emission differs from last frame
- the probe filter, when the SH it writes differs
- the surface-cache push, when the packed filtered field differs, and only for blocks that read that texel last time they were composed

Compose is an indirect dispatch of that list. A frame with an empty list launches no groups. The picture remains in the first half of the output buffer. Emission for the frame sits in the second half, so the direct pass does not overwrite the picture. The CPU does not read the list.

The camera, the resize, and a picture with more blocks than the list (65,535, the minimum Vulkan workgroup-count limit) take the full pass, which also clears the texel-to-block rects and records them again. `GENOS_LIGHTING_COMPOSE_ALL=1` forces the full pass. The surface-cache path is the one that skips; other world modes compose the whole picture.

While the camera matches last frame's, a filtered field stays the value pixels show after S72V10's two-frame stamp would have expired. The push idle and a frozen frame do not rewrite that stamp, and letting the field fall through to the world radiance cache on a timer would change a still picture whose field bytes did not change. A push that writes a different field still marks the blocks.

The G-buffer is cleared at the start of the frame. Compose used to clear the pixels it visited; skipped pixels would otherwise keep last frame's direct light in the record the raster mins into.

There is no rotating fraction of the image. A steady frame whose producers write the same values does no compose work. The checks are the comparisons at the writes, which those passes already perform.

Filter and compact are not given new quiet paths here. An on-screen texel is kept from being dropped or freed while compose is not restamping it (the camera has not moved, so the same texels are on screen). The cache pass uses that same expiry, so a relit texel is not freed on the last-read timer during a still camera.

## Numbers
Not measured here. Still-scene compose time, moving-scene compose time, and the G-buffer clear belong on the owner GPU (RTX 4070, stress, 1920×1080).

## Screenshots
not recorded

## Outcome and why
The mechanism is in. Whether a settled still frame is actually near zero is the measurement below.

## Lessons / when to revisit
Measure compose in a settled still frame and in a moving frame. If still compose is not near zero, the producers are still marking blocks: probe SH writes, direct-light mismatches, or filtered-field changes. `GENOS_LIGHTING_COMPOSE_ALL=1` is the full-compose comparison.

## Deviation
SVGF and Lumen shade every pixel every frame. This does not. Holding the filtered field for the whole still camera, rather than for two frames from the push stamp, departs from S72V10. It does not depart from a paper; the two-frame window was ours, and it assumed the push rewrote the stamp every frame.
