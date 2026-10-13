# S73L — Unfilled pixels on screen

Anthony can see lighting fill in on a turn. This is the counter for that. It does not change pre-fill, the estimator, or what a texel stores.

A composed surface pixel in surface-cache mode is unfilled when the bilinear weights of finished sets (`floor` of the texel's own sample count) stay under one set. A missing corner counts as zero finished sets. `pct` is that count over the shaded pixels composed this frame. Quiet frames compose only changed blocks, so the share is not the whole picture. A turn changes the camera, and compose covers the frame.

Off, the compose path tests one uniform and does no atomics. On, each shaded pixel adds to a shaded counter and, when unfilled, to a second counter and a bit of a row-major mask. The CPU scans the mask for the longer of the horizontal and vertical runs.

The counters sit after the probe-wake tail (words 392 and 393). The mask starts at stats word 396, so it does not alias the control block or the wake frontier. The device buffer keeps room for a 4096×2160 mask whether or not the detector is on, so a hotkey can enable it without reallocating.

`--unfilled` turns the count on. `--unfilled-tint` turns the magenta tint on (bit 25 of the debug word) and turns the count on. `--unfilled=0` and `--unfilled-tint=0` force those off. F8 and Shift+F8 still toggle them after launch, and so do the stress knobs `unfilled` and `unfilled_tint` (MCP `knobs` and Rhai `knob`). `GENOS_LIGHTING_UNFILLED` and `GENOS_LIGHTING_UNFILLED_TINT` apply only when the matching argument was omitted.

Events land in `./genos-debug/unfilled/<timestamp>/` when `--unfilled-dump <percent>` is set (`0` saves any positive share). That directory is `DebugSettings.out` on the scene (`EngineSettings`). `--out <dir>` overrides it for the process. `GENOS_UNFILLED_DIR` mirrors the same field and applies only when `--out` was omitted. The stress panel shows the directory in force and a default button writes `genos-debug/unfilled`. MCP `get_debug_settings` and `set_debug_settings` read and write the text form. Rhai `debug_settings("out=...")` overlays it while the scene loads. `debug` stays Rhai's printer. The process does not write under `$HOME`. Startup and each save drop folders older than `--unfilled-max-age-secs` (default two days), then the oldest until `--unfilled-max-folders` and `--unfilled-max-bytes` both hold, on whichever directory is in force. `--unfilled-session-cap` limits one process. `GENOS_LIGHTING_UNFILLED_DUMP`, `GENOS_UNFILLED_MAX_AGE_SECS`, `GENOS_UNFILLED_MAX_FOLDERS`, `GENOS_UNFILLED_MAX_BYTES`, and `GENOS_UNFILLED_SESSION_CAP` mirror those arguments. The flags are listed in `genos-stress --help`.

## Networking

- Replicated state: none. The output directory is local to the process that draws.
- Authority: the client that owns the view.
- Prediction / rollback: none.
- Determinism: the directory does not change the picture.
- Headless server: it does not write unfilled events. It can store the path.

`--gpu-times` and `GENOS_GPU_TIMES` do not enable the detector. Leave `--unfilled` off for a `LIGHTING_MS` line with no unfilled sample. With the detector on, that line gains ` | UNFILLED px=N pct=X.XX max_run=M`, and the same line is printed on its own each frame.

On the pre-fill branch's lavapipe turn (320×180, settle 96, a 0.5 rad turn of 24 frames) the last turn frame was 0.92% (483 pixels, max run 35). That assertion is not in this change.
