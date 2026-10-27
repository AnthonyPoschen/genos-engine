#![expect(
    clippy::must_use_candidate,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]

//! The command catalogue the MCP port lists. Each command takes one JSON object.

use genos_mcp::command::CommandSpec;
use genos_mcp::json;

fn spec(name: &str, description: &str) -> CommandSpec {
    CommandSpec {
        name: name.into(),
        description: description.into(),
        schema: json::object([
            ("type", json::string("object")),
            ("additionalProperties", json::bool(true)),
        ]),
    }
}

/// Every command [`crate::Tools`] runs. Shared by MCP, `POST /cmd/<name>` and scripts.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec("status", "Frame, pause state, view, camera, and whether the lighting has settled (stable, still_frames, picture_settled, change_records, deferred_spheres)."),
        spec("knobs", "List every knob the example publishes, with range and step. With name and value, set one first."),
        spec("lights", "Every light: position, colour, range, distance, in_view, screen_share, impact (colour luminance x screen share), on."),
        spec(
            "light",
            "Edit a light: add (position, color, intensity), or index with on, color, intensity, position, path [[x,y,z],..] + frames (+ loop), remove, reset, solo; solo_off clears a solo and does not need an index.",
        ),
        spec("objects", "Every solid: index, position, yaw, size, colour, edited."),
        spec("object", "Edit solid index: position, yaw, spin (rad/s), freeze (true/false), path [[x,y,z],..] + frames (+ loop), reset."),
        spec("camera", "Set position [x,y,z], yaw, pitch, look_at [x,y,z], or path [[x,y,z,yaw,pitch],..] over frames. Returns the pose."),
        spec("time", "pause (bool), fixed_dt (s; 0 or real_time: true for wall time)."),
        spec("step", "Advance a paused scene by frames (default 1) and reply after them."),
        spec("wait", "Reply after frames (default 1). Scene time runs unless paused."),
        spec(
            "wait_settled",
            "Reply when the lighting has settled and a frame drawn from its held light is on screen, for quiet_frames (default 3), or after timeout_frames (default 600). Reply says settled true/false, frames, wall_ms and still_frames.",
        ),
        spec(
            "view",
            "What the picture shows: mode full, direct, bounce, nobounce (direct), albedo, normal, depth, light:I, texels, variance, tail.",
        ),
        spec(
            "capture",
            "Save the picture: path (relative to the shot dir), mode (a view), width/height (resampled), name (keep in memory for diff), region [x,y,w,h] in 0..1 for the luminance in the reply. ui (default false) leaves out the debug menu, the menu tab, the frame-rate plate, and tool overlays.",
        ),
        spec("captures", "Save several views in consecutive frames: modes [..], prefix, width, height. ui (default false) leaves out the debug menu, the menu tab, the frame-rate plate, and tool overlays."),
        spec("luminance", "Mean, min and max linear luminance and black share of region [x,y,w,h] (0..1) in mode."),
        spec(
            "diff",
            "Per-pixel diff: a and b (capture names or PNG paths), or mode_a and mode_b drawn now. threshold (0..255, default 8), gain, out (diff PNG), region.",
        ),
        spec(
            "flicker",
            "Frame-to-frame change over frames (default 60): mean_delta, max_delta, pops (changes over threshold, default 0.03 luminance), popped_share, mean_range. region, mode, heatmap (PNG path), frames_dir (save every frame).",
        ),
        spec(
            "reference",
            "Ground truth: path-trace the scene as drawn this frame from the camera pose on the CPU (unlimited bounce, next-event direct light, no probes), off the frame thread. Traces in passes of spp paths per pixel (default 64) until the mean relative noise of a pixel is at most noise (default 0.03), max_spp (default 16384) or seconds (default 300) run out; bounces (default inf; 0 = direct only), width/height (default the picture, at most 320 wide), name (stored as, default reference), prefix (files <prefix>-reference.png). Pause first: the trace is of one moment. A trace is kept in cache (default target/reference-cache or $GENOS_REFERENCE_CACHE; \"off\" never) and reused for the same scene, pose, size and bounces when it is as converged as asked. Replies with the path, spp done, noise, trace_seconds, coverage.",
        ),
        spec(
            "compare",
            "Trace a reference (as `reference`, or reuse a stored one: reference: name), then capture the live picture (mode, default full; frames: N waits N frames after the trace starts; or live: a capture name/PNG) and compare in linear luminance on block x block averages (default 2; the reference's noise falls with the block side). Saves <prefix>-reference.png, -live.png, -heatmap.png (red too bright, blue too dark, full at 50% error). Replies mean_abs, mean_rel (share of the light that is wrong), p95_abs, p95_rel, bias (+ too bright), ref_mean, live_mean and regions (grid, default [3,3], row-major mean_rel and bias). region limits it. ui (default false) leaves out the debug menu, the menu tab, the frame-rate plate, and tool overlays.",
        ),
        spec("timing", "Frame time breakdown over the last frames (default 60): fps, frame, draw CPU, GPU."),
        spec(
            "gi_metrics",
            "GI pass times and work counters since the last reset. Passes: direct, atrous, compose, trace, filter, gather, resolve, and lighting. Counters: rays, cache_entries_refreshed, lamps_shaded. resolve is gather + filter + compose and is not added again into lighting. reset true clears the window and <out>/gi/frames.csv. write NAME saves <out>/gi/NAME.csv (p50, p99) and NAME-frames.csv, or NAME-2.csv and NAME-2-frames.csv when that name is already there. Requires --gi-metrics at launch. GENOS_GI_METRICS applies only when that argument was omitted. One frame behind the draw.",
        ),
        spec(
            "get_lighting_settings",
            "The lighting settings in force (text form) and the last collected GPU pass: name, milliseconds, and dispatch count. One frame behind the draw that just finished.",
        ),
        spec(
            "set_lighting_settings",
            "Partial lighting update. Keys match the settings text (direct, shadows, gi, bounces, probes, …). A text field overlays key=value lines. The scene keeps the result; a renderer override is cleared. Applies on the next draw.",
        ),
        spec(
            "list_presets",
            "potato, day-cycle, gi-1bounce, low, medium, and high, each with the settings text that preset writes.",
        ),
        spec(
            "apply_preset",
            "Replace the scene lighting with preset name (potato, day-cycle, gi-1bounce, low, medium, high) and clear a renderer override.",
        ),
        spec(
            "get_debug_settings",
            "The debug output directory in force (text form) and whether captures keep the UI. --out wins over GENOS_UNFILLED_DIR, which wins over the scene value. The default directory is genos-debug/unfilled under the working directory. ui defaults to false: a capture leaves out the debug menu, the menu tab, the frame-rate plate, and tool overlays.",
        ),
        spec(
            "set_debug_settings",
            "Partial debug-output update. Key out is the unfilled directory. Key ui is on or off (default off) and is the capture default. A capture command's ui argument overrides it for that shot. A text field overlays key=value lines. Writes the scene. --out and GENOS_UNFILLED_DIR still overlay the directory used for the next save. --ui forces ui on.",
        ),
        spec(
            "get_fps_counter",
            "The frame-rate overlay settings in force (text form): enabled and rate_hz. Off by default, 4 Hz. --fps-counter and --fps-hz win over GENOS_FPS_COUNTER and GENOS_FPS_HZ, which win over the scene value.",
        ),
        spec(
            "set_fps_counter",
            "Partial frame-rate overlay update. Keys are enabled (on/off) and rate_hz (0.25 to 30). A text field overlays key=value lines. Writes the scene. --fps-counter, --fps-hz, and the GENOS_FPS_* mirrors still overlay the value the readout uses.",
        ),
        spec(
            "get_event_loop",
            "The event-loop stall mark in force (text form): longest_stall_ms, the longest gap between window pumps since the last clear. The frame writes it. The budget is 100 ms.",
        ),
        spec(
            "set_event_loop",
            "Partial event-loop update. Key longest_stall_ms (alias stall_ms) is zero or more. Writing 0 clears the high-water mark. A text field overlays key=value lines. Writes the scene. The next frame adopts a lower value.",
        ),
    ]
    .into_iter()
    .chain(schedule_specs())
    .chain(reflection_specs())
    .chain(hdr_specs())
    .chain(fog_specs())
    .chain(host_specs())
    .chain(material_specs())
    .collect()
}

fn schedule_specs() -> Vec<CommandSpec> {
    vec![
        spec(
            "get_budgets",
            "Preset work caps, the live controller scale, and the schedule knobs in force (schedule, controller, budget_scale, guard_edge, floor_level, floor_converged, near_radius, cache_loosen). lighting_cap_ms is the millisecond ceiling for this preset. controller is the on/off for that scale.",
        ),
        spec(
            "set_budgets",
            "Partial schedule update. Keys: schedule (budget or global), controller (on or off), budget_scale, guard_edge, cache_loosen, floor_level, floor_converged, near_radius. Writes the scene and clears a renderer override. Applies on the next draw.",
        ),
        spec(
            "get_schedule_stats",
            "The last lighting schedule push: controller scale, lighting cap, direct/atrous/trace gates, tile rect, starvation band, lit size, crop, spawn cap, coarse slots, and whether detail levels are on.",
        ),
        spec(
            "lighting_lod_snapshot",
            "Detail levels for the last schedule push. enabled is budget mode with world-cache mode 3. levels_m is 0.25, 0.5, 1, 2, 4. base_m is the 0.125 lattice. spawn_cap and coarse_slots are the frame's caps.",
        ),
    ]
}

fn reflection_specs() -> Vec<CommandSpec> {
    vec![
        spec(
            "get_reflection_settings",
            "The reflection settings in force (text form): enabled, rough_cutoff, mirror_cutoff, mirror_views, rays, shore_cap, caustics, tracer, and water. --reflections, --reflection-rays, --mirror-views, --water, --caustics, and --reflection-tracer win over GENOS_REFLECTIONS, GENOS_REFLECTION_RAYS, GENOS_MIRROR_VIEWS, GENOS_WATER, GENOS_CAUSTICS, and GENOS_REFLECTION_TRACER, which win over the scene value.",
        ),
        spec(
            "set_reflection_settings",
            "Partial reflection update. Keys are enabled, rough_cutoff, mirror_cutoff, mirror_views, rays, shore_cap, caustics (off, image, cache), tracer (off, software, ray_query), and water. A text field overlays key=value lines. Writes the scene. The command-line arguments and GENOS_REFLECTION_* mirrors still overlay the value the picture uses.",
        ),
        spec(
            "get_reflection_stats",
            "The last reflection pass: ran, rays, views, gpu_ns, and vram_bytes. gpu_ns is the timestamp query around that pass. vram_bytes is the reflection images and grade buffers. ms_frac and vram_frac stay NOT-IMPLEMENTED: lavapipe time is not the 4070 or 1060 cap. Settings stay on get_reflection_settings.",
        ),
    ]
}

fn hdr_specs() -> Vec<CommandSpec> {
    vec![
        spec(
            "get_hdr_settings",
            "The HDR settings in force (text form). --exposure-mode, --ev100, --compensation, --iso, --aperture, --shutter, --adapt-down, --adapt-up, --hdr-display, --bloom, --bloom-intensity, --bloom-levels, and --output win over GENOS_HDR_*. --out wins over GENOS_HDR_OUT. A GENOS_HDR_* mirror applies only when that argument was omitted. --display selects the transform only when its value is agx, aces, or legacy; otherwise it is the Wayland display name.",
        ),
        spec(
            "set_hdr_settings",
            "Partial HDR update. Keys match the settings text (exposure_mode, ev100, compensation, iso, aperture, shutter, adapt_down, adapt_up, histogram_bins, display, bloom, bloom_intensity, bloom_levels, output, legacy_bytes). A text field overlays key=value lines. Writes the scene. The arguments and GENOS_HDR_* mirrors still overlay the value the next draw uses.",
        ),
        spec(
            "get_hdr",
            "HDR probe. ev100, exposure, and scales (the last 8 exposure scales). grade is scene. still_pop is the share of subsampled scene luma that moved by more than 0.01. histogram is the last bin counts, also written to <out>/hdr-output/histogram.txt. surfaces lists the swapchain (format, color_space) in use. chosen is the output mode. resolved is the mode `resolve_swapchain` picked. pq_white and scrgb_white are `swapchain_nits` of graphics white. hdr_ms and post_ms_device are bloom plus the display pass, one frame behind, zero when the device has no timestamps. hdr_vram_bytes is the post buffers in bytes. post_vram_device is that sum in MiB. The 1.50 ms and 96 MiB ceilings are the 4070 budget, not a lavapipe score.",
        ),
    ]
}

fn fog_specs() -> Vec<CommandSpec> {
    vec![
        spec(
            "get_fog_settings",
            "The fog settings in force (text form): enabled, density, height, thickness, albedo, phase_g, gi, history, grid_width, grid_height, grid_slices, samples, and volume lines. Zero on a grid axis keeps the preset axis. Zero samples dispatches that grid whole. view is the debug page (extinction, inscatter, transmittance, history, age, budget, volumes) and is not in the text. --fog and the other fog flags win over GENOS_FOG and the other mirrors, which win over the scene value. History is stored and is not blended.",
        ),
        spec(
            "set_fog_settings",
            "Partial fog update. Keys match the settings text, plus view and clear_volumes. A text field overlays key=value lines. Writes the scene. Launch flags and their mirrors overlay the fields they set on the next frame.",
        ),
        spec(
            "get_fog_stats",
            "Fog counters: dispatches, samples_used, samples_capped, grid_width, grid_height, grid_slices, dispatched_width, dispatched_height, dispatched_slices, history_accepts, history_rejects, vram_bytes, pass_ms, and the open view line. samples_used is the samples setting (0 keeps every cell). The dispatched_* fields are the grid the frame runs. Accepts and rejects stay 0 because history is off. pass_ms is the last frame's lavapipe froxel timestamp, or 0 before that frame. It is not graded against an RTX 4070 ceiling.",
        ),
    ]
}

fn material_specs() -> Vec<CommandSpec> {
    vec![
        spec(
            "get_material_settings",
            "The material settings in force (text form): texture_quality, mip_bias, anisotropy, occlusion, emissive_promote, emissive_promote_flux, headroom_percent, headroom_floor_mb, dwell_frames. texture_quality does not follow VRAM size.",
        ),
        spec(
            "set_material_settings",
            "Partial material update. Keys match the settings text. A text field overlays key=value lines. residency_mb is a deprecated alias: it warns once and does not set a pool. Applies on the next draw.",
        ),
        spec(
            "get_material_stats",
            "Streamer counters for the frame in force: committed, budget, refused, evictions, thrash, holes, rate, stream_ms. budget is the texture target. rate is the bytes the last step admitted. stream_ms is the base frame time that step saw.",
        ),
    ]
}

fn host_specs() -> Vec<CommandSpec> {
    vec![
        spec(
            "get_menu_settings",
            "The debug menu in force (text form): visible (shown or hidden), tab (page title), collapsed (folded section keys), wheel, and keys. wheel and keys are hover, modal, click, or passive. Shown by default, with hover for the wheel and click for keys. --menu wins over GENOS_MENU, which wins over the scene value. tab, collapsed, wheel, and keys are the stored fields.",
        ),
        spec(
            "set_menu_settings",
            "Partial debug-menu update. Keys are visible (shown or hidden), tab, collapsed (comma-separated section keys), fold (add one key), open (remove one key), wheel, and keys (hover, modal, click, or passive). focus_wheel and focus_keys are the same as wheel and keys. A text field overlays key=value lines. Writes the scene. --menu and GENOS_MENU still overlay visible.",
        ),
        spec("quit", "Stop the example with exit code (default 0)."),
        spec(
            "list_scenes",
            "Names of the scenes this binary can load.",
        ),
        spec(
            "load_scene",
            "Load a scene by name. A large scene loads off the frame thread; the reply arrives when it is in place and the window keeps pumping. Drops light and object edits and clears the lighting cache. checkpoint (optional) jumps after the load; otherwise the first checkpoint is used.",
        ),
        spec(
            "list_checkpoints",
            "Checkpoint names for the scene that is loaded.",
        ),
        spec(
            "jump_checkpoint",
            "Move the camera to a checkpoint by name. That pose is the one Home resets to.",
        ),
    ]
}
