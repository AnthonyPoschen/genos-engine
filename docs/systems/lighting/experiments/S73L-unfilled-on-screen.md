# S73L — Unfilled pixels on screen

Anthony can see lighting fill in on a turn. This is the counter for that. It does not change pre-fill, the estimator, or what a texel stores.

A composed surface pixel in surface-cache mode is unfilled when the bilinear weights of finished sets (`floor` of the texel's own sample count) stay under one set. A missing corner counts as zero finished sets. `pct` is that count over the shaded pixels composed this frame. Quiet frames compose only changed blocks, so the share is not the whole picture. A turn changes the camera, and compose covers the frame.

Off, the compose path tests one uniform and does no atomics. On, each shaded pixel adds to a shaded counter and, when unfilled, to a second counter and a bit of a row-major mask. The CPU scans the mask for the longer of the horizontal and vertical runs.

The counters sit in the spare words after the lighting control block (words 140 and 141). The mask starts at stats word 144, so it does not alias the control block. The device buffer keeps room for a 4096×2160 mask whether or not the detector is on, so a hotkey can enable it without reallocating.

`GENOS_LIGHTING_UNFILLED`, F8, and the stress knob `unfilled` turn the count on. The tint is bit 25 of the debug word (Shift+F8, `GENOS_LIGHTING_UNFILLED_TINT`, knob `unfilled_tint`). Those two knobs are on the stress panel and are MCP `knobs` and Rhai `knob`. Dumps land in `~/genos-perf/unfilled/<timestamp>/` when `GENOS_LIGHTING_UNFILLED_DUMP` is set. Startup and each save drop folders older than two days (`GENOS_UNFILLED_MAX_AGE_SECS`), then the oldest until the folder count and the byte cap both hold.

`GENOS_GPU_TIMES` does not enable the detector. Leave `GENOS_LIGHTING_UNFILLED=0` for a `LIGHTING_MS` line with no unfilled sample. With the detector on, that line gains ` | UNFILLED px=N pct=X.XX max_run=M`, and the same line is printed on its own each frame.

On the pre-fill branch's lavapipe turn (320×180, settle 96, a 0.5 rad turn of 24 frames) the last turn frame was 0.92% (483 pixels, max run 35). That assertion is not in this change.
