# Lighting debug standard

Lighting is the reference for a debug binary and for observability. Another system's `debug/<system>` binary matches this set. This is the inventory of what is in the repository.

## GI dump

`GENOS_LIGHTING_DUMP=N` and `GENOS_LIGHTING_DUMP_DIR` copy the cache, cache keys, work, G-buffer, and output at the end of frames N and N+1 (`crates/render/src/lighting_gpu.rs`). Files are `f{F}_{name}.bin` plus `f{F}_layout.json`.

`tools/lighting/dump.py DIR N` splits those buffers into one file per component and channel: surface-cache and world-cache CSVs and slot atlases (value, sets, variance, TDL direct, last-use age, dirty-stamp age), probe irradiance, own strata, a kept-source CSV, per-pixel direct light, and N to N+1 diffs (slot delta atlases, a top-down map of flips over 5 %, a probe changed mask and count, a pixel-direct delta).

Isolate toggles attribute a change to one component: `GENOS_LIGHTING_TOGGLES`, or the file named by `GENOS_LIGHTING_TOGGLES_FILE` (re-read every frame). The toggles are `freeze_cache`, `direct_only`, `bounce_only`, and `freeze_probes`. The note is [S78S](experiments/S78S-counters-still-churn.md).

## Stats counters

- `GENOS_LIGHTING_SC_TRACE=1` prints one `lightingchg frame N c [...]` line per frame from `gstats.chg` (slots `CHG_*` in `crates/render/shaders/lighting_cache.glsl`). `tools/lighting/chg.py RUN_DIR` labels the mean over frames 300–600. The same readback prints `lightingdg` diag words.
- `GENOS_LIGHTING_DEBUG` or `GENOS_LIGHTING_SC_TRACE` prints `lightingatrous` (dirty fraction, mode `list` or `dense`, threshold, filtered texels) for the a-trous fallback. `GENOS_LIGHTING_ATROUS_DIRTY` is the fraction (default 0.25; `0` always dense, `1` lists up to the cap). `Renderer::lighting_atrous_stats` reads the same words. `GENOS_LIGHTING_DENOISE_RES` is `full` or `half` (the even-lattice bounce filter). High leaves it unset.
- `GENOS_LIGHTING_DEBUG=1` prints `lighting probes rays … per_probe …`. It reads GPU counters back, so it stays off fps runs.
- `GENOS_LIGHTING_SC_TRACE` also prints `lightingsc` (surface-cache claims and relights).
- `GENOS_GPU_TIMES=1` prints `GPU_MS`. `GENOS_GPU_DISPATCH=1` with it prints `LIGHTING_DISPATCH`.
- `Renderer::settle_stats` returns `SettleStats { settled, still_frames }`. The MCP `timing` command returns fps, frame, draw CPU, and GPU.

## MCP debug endpoints

One catalogue, three doors: MCP `tools/call`, `POST /cmd/<name>`, and the Rhai script functions. The list is `crates/debug/src/commands.rs`. The running notes are [Debugging](../debugging.md).

`status`, `knobs`, `lights`, `light`, `objects`, `object`, `camera`, `time`, `step`, `wait`, `wait_settled`, `view`, `capture`, `captures`, `luminance`, `diff`, `flicker`, `reference`, `compare`, `timing`, `quit`.

`knobs` lists and sets live tunables. `light` and `object` edit one entity, including `reset`. `capture`, `diff`, `flicker`, and `compare` write pictures; `compare` and `flicker` write heatmaps.

## Stuck and pop checks

- Pop share: `examples/stress/src/gi_check/mod.rs` (`FILL_POP_STEP` 0.01, the share of pixel-frames that step more than that from a 9-frame average), `examples/stress/scripts/gi_check/fill.rhai`.
- Onset pop and return: the `turn` rows in `gi_check/mod.rs`, `examples/stress/scripts/gi_check/turn.rhai`.
- Corner-walk pop: the walk row in `gi_check/mod.rs`.
- Lights that move and come back: `lampret` in `examples/stress/scripts/gi_check/mix.rhai`, scored in `gi_check/mod.rs` (end against the soaked baseline; leftover light).
- `examples/stress/scripts/sun_fast_pop.rhai`.
- `a_turn_toward_the_lamp_does_not_pop_the_bounce` and `a_small_step_does_not_flash_a_wall` in `crates/render/tests/gpu_frame.rs`.
- `a_pop_counts_once_per_pixel_change` in `crates/debug/src/image.rs`.

## Headless harness

`make verify` and `make reference` run `genos-stress --proof --no-panel`. A script report is `genos-stress --proof --no-panel --size 320x180 --script FILE --report DIR`. `genos-stress gi-check` is the picture gate (`examples/stress/src/gi_check/mod.rs`). On a machine with no GPU this runs on lavapipe under a headless Weston, as [Debugging](../debugging.md) describes.

`tools/lighting/headless.sh` is not in this tree. There is no `gi_headless` test module. The GPU tests that cover lighting headless are `crates/render/tests/lighting_frame.rs` (including `lighting_moving_camera_keeps_its_brightness` on lavapipe), `crates/render/tests/gpu_frame.rs`, and `examples/stress/tests/` (`picture.rs`, `gpu_reference.rs`, `mesh_reference.rs`). Experiment notes name `z10/dump.sh`, `z10/stuck.sh`, and `perf/chg.py` on the owner's machine; those scripts are not in the repository. The decoders that are in the repository are `tools/lighting/dump.py` and `tools/lighting/chg.py`.

## Overlays and debug draws

- Picture modes in `crates/render/src/debug_view.rs`: `full`, `direct`, `bounce`, `albedo`, `normal`, `depth`, `light`, `texels`. The MCP `view` command selects one.
- `examples/stress` draws the lighting panel from `genos-debug` knobs, a frame-time graph (`FrameGraph` in `examples/stress/src/main.rs`), and `Tools::overlay` plus `Renderer::set_overlay_lines`.
- `profile_overlay` and `inspect_region` in `crates/ui/src/profile.rs`. The camera's basic and detailed modes draw the profile graph.
- Heatmaps from `compare` (`<prefix>-heatmap.png`) and `flicker`.

## Live tunables

Stress knobs in `examples/stress/src/knobs.rs`: time of day, sun speed, sun freeze, sky, boxes, lamps, lamp clock, box clock, and the lamp and scale controls the panel shows. Lighting knobs write `Scene.lighting` through `set_key`: `lighting_preset`, `lighting_bounces`, a switch per feature, and the quality knobs (`lighting_direct_lights`, `lighting_direct_refresh`, `lighting_direct_scale`, `lighting_shadow_filter`, `lighting_gi_refresh`, `lighting_budget_shift`, `lighting_probe_rays`, `lighting_probe_stale`, `lighting_world_stale`, `lighting_denoise_iters`, `lighting_denoise_res`, `lighting_ambient`). The same knobs are MCP `knobs` and Rhai `knob`. `get_lighting_settings`, `set_lighting_settings`, `list_presets`, and `apply_preset` are the settings commands; the panel's `set_lighting` still edits sun, sky, and boxes. See [toggles.md](toggles.md) and [the settings principles](../../engine/settings-principles.md).

Env knobs for an A/B, listed in [Lighting](README.md): `GENOS_LIGHTING_WORLD`, `GENOS_LIGHTING_SC_FLAGS`, `GENOS_LIGHTING_SC_LOCAL`, `GENOS_LIGHTING_SC_SETTLE`, `GENOS_LIGHTING_WORLD_CAP`, `GENOS_LIGHTING_TILE`, `GENOS_LIGHTING_CACHE_HZ`, `GENOS_LIGHTING_CACHE_PICKS`, `GENOS_LIGHTING_FILL`, `GENOS_LIGHTING_YOUNG_ONLY`, `GENOS_LIGHTING_MOVED_DROP`, `GENOS_LIGHTING_URGENT`, `GENOS_LIGHTING_BORROW`, `GENOS_LIGHTING_ASYNC`, `GENOS_LIGHTING_ATROUS_DIRTY`, `GENOS_LIGHTING_DENOISE_RES`.
