# Lighting debug standard

Lighting is the reference for a debug binary and for observability. Another system's `debug/<system>` binary matches this set. This is the inventory of what is in the repository.

## GI dump

`GENOS_LIGHTING_DUMP=N` and `GENOS_LIGHTING_DUMP_DIR` copy the cache, cache keys, work, G-buffer, and output at the end of frames N and N+1 (`crates/render/src/lighting_gpu.rs`). Files are `f{F}_{name}.bin` plus `f{F}_layout.json`.

`tools/lighting/dump.py DIR N` splits those buffers into one file per component and channel: surface-cache and world-cache CSVs and slot atlases (value, sets, variance, TDL direct, last-use age, dirty-stamp age), probe irradiance, own strata, a kept-source CSV, per-pixel direct light, and N to N+1 diffs (slot delta atlases, a top-down map of flips over 5 %, a probe changed mask and count, a pixel-direct delta).

Isolate toggles attribute a change to one component: `GENOS_LIGHTING_TOGGLES`, or the file named by `GENOS_LIGHTING_TOGGLES_FILE` (re-read every frame). The toggles are `freeze_cache`, `direct_only`, `bounce_only`, and `freeze_probes`. The note is [S78S](experiments/S78S-counters-still-churn.md).

## Stats counters

- `GENOS_LIGHTING_SC_TRACE=1` prints one `lightingchg frame N c [...]` line per frame from `gstats.chg` (slots `CHG_*` in `crates/render/shaders/lighting_cache.glsl`). `tools/lighting/chg.py RUN_DIR` labels the mean over frames 300–600. The same readback prints `lightingdg` diag words.
- `GENOS_LIGHTING_DEBUG` or `GENOS_LIGHTING_SC_TRACE` prints `lightingatrous` (dirty fraction, mode `list`, `dense`, or `screen`, threshold, filtered texels) for the a-trous fallback. Mode `screen` also prints `lightingdenoise` and writes `./genos-debug/denoise/latest.txt` plus `frames.log` (pixels, coarse, changed, filtered, mode, ran). `GENOS_LIGHTING_ATROUS_DIRTY` is the fraction (default 0.25; `0` always dense, `1` lists up to the cap) for the hash wavelet. `Renderer::lighting_atrous_stats` reads the same words, including `screen_pixels`, `coarse_pixels`, and `changed_pixels`. `GENOS_LIGHTING_DENOISE_LAUNCH` is `hash` or `screen` (preset high defaults to `screen`; the other presets default to `hash`). `--denoise-launch` and that variable override the preset. `GENOS_LIGHTING_DENOISE_SCREEN` is the on/off alias. `GENOS_LIGHTING_DENOISE_ADAPTIVE` and `GENOS_LIGHTING_DENOISE_VARIANCE` drop settled wavelet iterations on the hash wavelet. Off on every preset. Half-res and tiles were removed (S79).
- `--lighting-debug` prints `lighting probes rays … per_probe …`. `GENOS_LIGHTING_DEBUG` is the mirror and applies only when that argument was omitted. It reads GPU counters back, so it stays off fps runs. With it, `./genos-debug/sun/catch.log` appends one `frame N catch C catch_left L` line per frame.
- `GENOS_LIGHTING_SC_TRACE` also prints `lightingsc` (surface-cache claims and relights).
- `--gpu-times N` prints `GPU_MS` every N frames with the picture size, the present mode, and separate `nonlight` and `lighting` times (`setup`, `gbuffer`, `aa`, `overlay`, `copy`). `nonlight` + `lighting` ≈ `frame`. `GENOS_GPU_TIMES` is the mirror and applies only when that argument was omitted. `GENOS_GPU_DISPATCH=1` with it prints `LIGHTING_DISPATCH` (env-only; no CLI mirror yet). It does not turn the unfilled detector on. A clean `LIGHTING_MS` line leaves `--unfilled` off.
- `--gi-metrics` records one sample per collected lighting frame. A bare flag turns it on. `--gi-metrics=off` turns it off. `GENOS_GI_METRICS` applies only when that argument was omitted (`0`, `off`, `false`, and `no` are off). The passes are `direct`, `atrous`, `compose`, `trace`, `filter`, `gather`, and `resolve`. `resolve` is gather + filter + compose and is not added again into `lighting`. `trace` is the screen-probe trace plus cache rays. `direct` is the direct dispatch plus hit light. The work counters are `rays` (screen-probe rays, cache rays, and a finished async cache round), `cache_entries_refreshed`, and `lamps_shaded` (fresh shadow rays from the lamp-pick diag words). The diag bit is set only while metrics are recording. The shade math is unchanged. Files go to `<out>/gi/`: `frames.csv` is the live window, and MCP `gi_metrics` with `write` saves `<name>.csv` (p50, p99) and `<name>-frames.csv`. `--out` overrides the root. Nothing is written under `$HOME`. Query pools are created at device open, so the flag cannot be turned on mid-run. A device with no timestamp queries logs `GI metrics: this device has no timestamp queries` and records nothing. The sample is one frame behind the draw. The first frame in a flight slot is skipped. Overhead is measured on the RTX 4070. Lavapipe timing is not a result.
- `--unfilled` counts composed surface pixels whose bilinear surface-cache corners average below one finished set. Each frame prints `UNFILLED px=N pct=X.XX max_run=M`. `pct` is among shaded pixels composed that frame. A quiet frame only composes changed 8×8 blocks, so the share there is not the whole picture; a turn composes every pixel. The cost when this is off is the uniform test and no atomics. `--unfilled-tint` turns on a magenta tint and the count. `--unfilled=0` and `--unfilled-tint=0` force those off. U still toggles the count in the stress and camera programs. F8 is the same key. Shift-U still toggles the tint. Shift-F8 is the same key. `GENOS_DEBUG_KEYS` replaces those chords. The stress knobs `unfilled` and `unfilled_tint` (MCP `knobs`, Rhai `knob`, and the stress panel) edit the same switches after launch. `GENOS_LIGHTING_UNFILLED` and `GENOS_LIGHTING_UNFILLED_TINT` are mirrors: each applies only when its argument was omitted.
- `--unfilled-dump <percent>` saves an event when `pct` goes above that number (`0` saves any positive share). Each event is `shot.png`, `overlay.png`, `camera.json`, `counts.json`, and `frame.txt` under `./genos-debug/unfilled/<timestamp>/`. `--out <dir>` overrides that directory. `GENOS_UNFILLED_DIR` mirrors `DebugSettings.out` (MCP `get_debug_settings` / `set_debug_settings`, Rhai `debug_settings("out=...")`, and the stress panel's Out row) and applies only when `--out` was omitted. Pictures are downscaled so the longest edge is 480. `--unfilled-session-cap` (default 32) limits one process. Across runs, startup and each save delete folders older than `--unfilled-max-age-secs` (default 2 days), then the oldest until `--unfilled-max-folders` (default 200) and `--unfilled-max-bytes` (default 500 MiB) both hold. `GENOS_LIGHTING_UNFILLED_DUMP`, `GENOS_UNFILLED_SESSION_CAP`, `GENOS_UNFILLED_MAX_AGE_SECS`, `GENOS_UNFILLED_MAX_FOLDERS`, and `GENOS_UNFILLED_MAX_BYTES` mirror those arguments. A non-numeric directory is left alone. One event that by itself exceeds the byte cap is kept.
- `Renderer::settle_stats` returns `SettleStats { settled, still_frames }`. The MCP `timing` command returns fps, frame, draw CPU, and GPU.

## MCP debug endpoints

One catalogue, three doors: MCP `tools/call`, `POST /cmd/<name>`, and the Rhai script functions. The list is `crates/debug/src/commands.rs`. The running notes are [Debugging](../debugging.md).

`status`, `knobs`, `lights`, `light`, `objects`, `object`, `camera`, `time`, `step`, `wait`, `wait_settled`, `view`, `capture`, `captures`, `luminance`, `diff`, `flicker`, `reference`, `compare`, `timing`, `gi_metrics`, `quit`, `list_scenes`, `load_scene`, `list_checkpoints`, `jump_checkpoint`.

`gi_metrics` returns the latest sample and p50/p99 for the passes and counters above. `reset` clears the window and `frames.csv`. `write` saves the summary. The command errors when `--gi-metrics` was off at launch.

`knobs` lists and sets live tunables. `light` and `object` edit one entity, including `reset`. `capture`, `diff`, `flicker`, and `compare` write pictures; `compare` and `flicker` write heatmaps.

## Stuck and pop checks

- Pop share: `examples/stress/src/gi_check/mod.rs` (`FILL_POP_STEP` 0.01, the share of pixel-frames that step more than that from a 9-frame average), `examples/stress/scripts/gi_check/fill.rhai`.
- Onset pop and return: the `turn` rows in `gi_check/mod.rs`, `examples/stress/scripts/gi_check/turn.rhai`.
- Corner-walk pop: the walk row in `gi_check/mod.rs`.
- Lights that move and come back: `lampret` in `examples/stress/scripts/gi_check/mix.rhai`, scored in `gi_check/mod.rs` (end against the soaked baseline; leftover light).
- `examples/stress/scripts/sun_fast_pop.rhai`.
- `a_turn_toward_the_lamp_does_not_pop_the_bounce` and `a_small_step_does_not_flash_a_wall` in `crates/render/tests/gpu_frame.rs`.
- `a_pop_counts_once_per_pixel_change` in `crates/debug/src/image.rs`.

## Lighting binary

`debug/lighting` (package `genos-lighting`) is the lighting debug binary. Scenes are data files in `debug/scenes/`, parsed by `debug/scene-lib`. The files are `stress`, `bounce_corner`, `hell`, `room`, `camera`, and `sponza`. `room` and `camera` point at the existing Rhai paths; those scripts stay where the tests load them.

```sh
make lighting SCENE=sponza
cargo run --release -p genos-lighting -- --scene sponza --gpu-times 30
```

`make lighting` is the benchmark: release, `--scene` from `SCENE` (default `stress`), and `--gpu-times 30`. The same arguments work on `cargo run`. `GENOS_SCENE`, `GENOS_LIGHTING_PRESET`, `GENOS_GPU_TIMES`, `GENOS_GI_METRICS`, and `GENOS_LIGHTING_DEBUG` apply only when the matching argument was omitted.

Phase 0 gate scripts live in `debug/lighting/scripts/gates/`. Run them on this binary, port 8766, not on `genos-stress`:

```sh
genos-lighting --headless --menu hidden --scene stress --checkpoint hall \
  --preset high --gi-metrics --size 1920x1080 \
  --script debug/lighting/scripts/gates/s1-spin.rhai \
  --report genos-debug/gi/s1-spin
```

Repeat for `s2-strafe`, `s3-approach`, `s4-many-movers`, `s5-aba`, and `s6-settle`. `--script` fixes the step at 1/60 and does not turn metrics on by itself.

The lighting MCP server listens on port 8766. `--mcp-port N` overrides it. `GENOS_MCP_PORT` applies only when that argument was omitted. `genos-stress` stays on 8765.

`--unfilled` selects the existing texels view (`ViewMode::Texels`). Slots with fewer than four sets are dim. There is no separate unfilled detector.

Screenshots, dumps, and logs go to `./genos-debug/shots/`, `./genos-debug/logs/`, and `./genos-debug/dumps/` unless `--out` is set. A capture leaves out the debug menu, the menu tab, the frame-rate plate, and tool overlays unless `ui` is true. `--ui` keeps them in a `--frames` PNG. `DebugSettings.ui` is the same switch, default false. When `GENOS_LIGHTING_DUMP` is set, dumps go to `<out>/dumps` if `--out` was passed or if `GENOS_LIGHTING_DUMP_DIR` was not already set. Nothing defaults to `$HOME`.

Page Up and Page Down switch scenes. Keys 1–9 jump checkpoints. Home and 0 reset to the active checkpoint. The panel is at most one third of the window wide. Labels wrap on spaces inside that width. A word stays whole, and glyphs that do not fit are left out. The title is the scene name. The pages are Scene, Pipeline, Lamps, GI, Denoise, Shadows, Perf, and Info. Each tab is as wide as its word. Tabs that fit share a row, and the rest wrap onto another row of the same height, so a label is never cut off. The scene description is the Info page. That mix is `debug/lighting/menu.txt`: `tab | section key | title | open or closed | setting`. A tab of `-` sits outside the strip. A section key of `-` has no fold. The Lamps page keeps an open fold of that name. GI keeps an open Probes fold and a closed World cache fold. The other pages are short enough to skip a fold. `GENOS_MENU_LAYOUT` points at another file of the same shape. A restart picks up the edit. No code change is required to move a setting. A press on a section title folds it. The folded names are written to `<out>/menu/collapsed.txt`. A press on a page writes its name to `<out>/menu/tab.txt`. The next launch reads both. Those names, the page, and whether the menu is shown are `MenuSettings` on the scene. `--menu hidden` hides it. `GENOS_MENU` applies only when that argument was omitted. `get_menu_settings` and `set_menu_settings` read and write the same fields. `fold` adds one section key and `open` removes one. A fold sent with a page change stays, including when that section is not on the new page. `]` and `[` change the page. Tab and shift-Tab do that while the pointer is over the menu. The panel draws scene, checkpoint, view, shadow cell cache, and ambient fill as dropdowns. Ordered grades are sliders: preset, bounce count, max lights, direct refresh frames, direct scale, GI refresh frames, ray budget shift, screen probe rays, probe stale frames, cache stale frames, denoiser iterations, and denoiser variance. Those words are the glossary terms. The config, MCP, and CLI names stay the key names. Each on/off feature switch is a checkbox in a three-column grid. A long label wraps inside its cell. The frame-rate plate is a checkbox on the FPS row, and its update rate is the numeric slider on that row. The wheel policy defaults to hover and the key policy defaults to click-to-focus. Perf sets both from dropdowns named Wheel focus and Key focus. A click gives the menu the keys. The first grave releases those keys and leaves the menu up. The next grave hides it. F1 hides it either way. The title bar hide button and the corner menu tab do the same with the pointer. F toggles the plate. F3 is the same key. The panel does not print a key list. `GENOS_DEBUG_KEYS` replaces those chords. While the menu has the keys, arrow keys move the focused control and a focused slider steps. Enter picks the highlighted choice or presses the focused control. Escape closes an open list, then releases the keys. The wheel scrolls when the wheel policy allows it. The default is hover, so the pointer has to be over the panel.

`load_scene` drops edits and calls `Renderer::reset_lighting_cache`. A deterministic replay and a saved setup are a follow-up. They are not in this binary.

## Headless harness

`make verify` and `make reference` run `genos-stress --proof --no-panel`. A script report is `genos-stress --proof --no-panel --size 320x180 --script FILE --report DIR`. `genos-stress gi-check` is the picture gate (`examples/stress/src/gi_check/mod.rs`). `make lighting-ref-check` scores the catalog against the CPU path trace and writes `./genos-debug/refcheck/<timestamp>/` ([the reference check](reference-check.md)). `MODE=lavapipe-small` is the small wiring run and uses the same frame indices as the owner run; it changes the picture size and the sample count. On a machine with no GPU this runs on lavapipe under a headless Weston, as [Debugging](../debugging.md) describes.

`tools/lighting/headless.sh` is not in this tree. There is no `gi_headless` test module. The GPU tests that cover lighting headless are `crates/render/tests/lighting_frame.rs` (including `lighting_moving_camera_keeps_its_brightness` on lavapipe), `crates/render/tests/gpu_frame.rs`, and `examples/stress/tests/` (`picture.rs`, `gpu_reference.rs`, `mesh_reference.rs`). Experiment notes name `z10/dump.sh`, `z10/stuck.sh`, and `perf/chg.py` on the owner's machine; those scripts are not in the repository. The decoders that are in the repository are `tools/lighting/dump.py` and `tools/lighting/chg.py`.

## Overlays and debug draws

- Picture modes in `crates/render/src/debug_view.rs`: `full`, `direct`, `bounce`, `albedo`, `normal`, `depth`, `light`, `texels`. The MCP `view` command selects one.
- `examples/stress` draws the lighting panel from `genos-debug` knobs, a frame-time graph (`FrameGraph` in `examples/stress/src/main.rs`), and `Tools::overlay` plus `Renderer::set_overlay_lines`.
- `profile_overlay` and `inspect_region` in `crates/ui/src/profile.rs`. The camera's basic and detailed modes draw the profile graph.
- Heatmaps from `compare` (`<prefix>-heatmap.png`) and `flicker`.

## Live tunables

Stress knobs in `examples/stress/src/knobs.rs`: time of day, sun speed, sun freeze, sky, boxes, lamps, lamp clock, box clock, unfilled count, unfilled tint, and the lamp and scale controls the panel shows. Lighting knobs write `Scene.lighting` through `set_key`: `lighting_preset`, `lighting_bounces`, a switch per feature, and the quality knobs (`lighting_direct_lights`, `lighting_direct_refresh`, `lighting_direct_scale`, `lighting_shadow_filter`, `lighting_gi_refresh`, `lighting_budget_shift`, `lighting_probe_rays`, `lighting_probe_stale`, `lighting_world_stale`, `lighting_denoise_iters`, `lighting_denoise_adaptive`, `lighting_denoise_variance`, `lighting_denoise_launch`, `lighting_ambient`). The same knobs are MCP `knobs` and Rhai `knob`. `get_lighting_settings`, `set_lighting_settings`, `list_presets`, and `apply_preset` are the settings commands; the panel's `set_lighting` still edits sun, sky, and boxes. See [toggles.md](toggles.md) and [the settings principles](../../engine/settings-principles.md).

Env knobs for an A/B, listed in [Lighting](README.md): `GENOS_LIGHTING_WORLD`, `GENOS_LIGHTING_SC_FLAGS`, `GENOS_LIGHTING_SC_LOCAL`, `GENOS_LIGHTING_SC_SETTLE`, `GENOS_LIGHTING_WORLD_CAP`, `GENOS_LIGHTING_TILE`, `GENOS_LIGHTING_CACHE_HZ`, `GENOS_LIGHTING_CACHE_PICKS`, `GENOS_LIGHTING_FILL`, `GENOS_LIGHTING_YOUNG_ONLY`, `GENOS_LIGHTING_MOVED_DROP`, `GENOS_LIGHTING_URGENT`, `GENOS_LIGHTING_BORROW`, `GENOS_LIGHTING_ASYNC`, `GENOS_LIGHTING_ATROUS_DIRTY`, `GENOS_LIGHTING_DENOISE_ADAPTIVE`, `GENOS_LIGHTING_DENOISE_VARIANCE`.
