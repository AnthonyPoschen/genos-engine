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

/// The sections of the catalogue, in the order `list_commands` gives them.
pub const SECTIONS: [&str; 6] = ["scene", "lighting", "render", "capture", "editor", "mesh"];

/// The section a command belongs to, by its name. `lighting` is lights, GI and
/// the lighting settings. `render` is reflections, HDR, fog, particles and
/// materials. `scene` is what is loaded and how time runs. `capture` reads the
/// picture. `editor` is the program's own knobs and windows. `mesh` is the
/// mesh viewer and mesh-workbench.
pub fn section_of(name: &str) -> &'static str {
    let has = |words: &[&str]| words.iter().any(|word| name.contains(word));
    if name.starts_with("mesh_") {
        "mesh"
    } else if has(&["reflection", "hdr", "fog", "particle", "material"]) || name.starts_with("fx") {
        "render"
    } else if has(&["lighting", "preset", "gi_metrics", "level0", "cone"]) || name == "lights" || name == "light" {
        "lighting"
    } else if has(&["capture", "luminance", "flicker", "reference", "compare"]) || ["view", "diff", "timing"].contains(&name) {
        "capture"
    } else if has(&["debug_settings", "fps_counter", "event_loop", "menu_settings", "list_commands"]) || name == "knobs" {
        "editor"
    } else {
        "scene"
    }
}

/// The name a client sees: the section, a dot, and the command's name without
/// the section's own word (`lighting.get_settings` for `get_lighting_settings`,
/// `mesh.view` for `mesh_view`). A name that already has a dot is kept
/// (`fx.spawn`). The registered name stays as a deprecated alias, and is the
/// name a script uses: a Rhai name has no dot.
pub fn public_name(name: &str) -> String {
    if name.contains('.') {
        return name.to_string();
    }
    let section = section_of(name);
    let rest = name
        .strip_prefix(&format!("{section}_"))
        .map(str::to_string)
        .or_else(|| {
            // The section as a whole word inside the name: `load_scene`, not `list_scenes`.
            let word = format!("_{section}");
            let at = name.find(&word)?;
            let after = &name[at + word.len()..];
            (after.is_empty() || after.starts_with('_')).then(|| format!("{}{after}", &name[..at]))
        })
        .filter(|rest| !rest.is_empty())
        .unwrap_or_else(|| name.to_string());
    format!("{section}.{rest}")
}

/// (public name, registered name) for every command: [`genos_mcp::command::publish`].
pub fn public_names() -> Vec<(String, String)> {
    specs().into_iter().map(|spec| (public_name(&spec.name), spec.name)).collect()
}

/// Every command [`crate::Tools`] runs. Shared by MCP, `POST /cmd/<name>` and
/// scripts. Each description starts with its section in brackets, so a flat
/// tool list still reads in groups.
pub fn specs() -> Vec<CommandSpec> {
    let mut specs = plain_specs();
    specs.push(spec(
        "list_commands",
        "The command catalogue grouped by section (scene, lighting, render, capture, editor, mesh): each command's name and description. section (optional) gives one section only.",
    ));
    for spec in &mut specs {
        spec.description = format!("[{}] {}", section_of(&spec.name), spec.description);
        if let Some(schema) = mesh_schema(&spec.name) {
            spec.schema = schema;
        }
    }
    specs
}

/// The JSON schema of a mesh tool's arguments. The viewer's own tools name
/// every field and refuse another. A family tool takes one mesh-workbench
/// command object: its `command` name is required and its other fields are
/// mesh-workbench's, which publishes no schema for them (`mesh_help` reads
/// them).
pub fn mesh_schema(name: &str) -> Option<json::Value> {
    let text = match name {
        "mesh_context" => r#"{"type":"object","additionalProperties":false,"properties":{"screenshot":{"type":"boolean","description":"false leaves the picture out. Default true: the reply's screenshot gives the saved file's path."},"edits":{"type":"integer","minimum":0,"maximum":32,"description":"How many of the latest edits to list. Default 8."}}}"#,
        "mesh_preview" => r#"{"type":"object","additionalProperties":false,"properties":{"commands":{"type":"array","items":{"type":"object","properties":{"command":{"type":"string","description":"The mesh-workbench command name."}},"required":["command"],"additionalProperties":true},"minItems":1,"maxItems":128,"description":"The edits to prepare as one transaction and show."},"guards":{"type":"object","description":"mesh-workbench transaction guards, for example {\"no_new_defects\": true}."},"commit":{"type":"boolean","description":"true commits the transaction that is shown."},"discard":{"type":"boolean","description":"true drops the preview and shows the live mesh."}}}"#,
        "mesh_commands" => r#"{"type":"object","additionalProperties":false,"properties":{"command":{"type":"string","description":"One mesh-workbench command name: reply with its fields, its reference text and a schema."},"family":{"type":"string","enum":["mesh_object","mesh_select","mesh_material","mesh_rig","mesh_animation","mesh_query","mesh_document","mesh_edit"],"description":"List only that tool's commands."}}}"#,
        "mesh_view" => r#"{"type":"object","properties":{"pick":{"type":"string","enum":["face","object","vertex"],"description":"What a click selects."},"hide":{"type":"array","items":{"type":"integer","minimum":0},"description":"Object ids to hide in the viewer."},"show":{"type":"array","items":{"type":"integer","minimum":0},"description":"Object ids to show again."},"solo":{"type":"array","items":{"type":"integer","minimum":0},"description":"Show only these object ids."},"show_all":{"type":"boolean","description":"true shows every object."},"shade":{"type":"string","enum":["shaded","textured","material","wireframe","uv_checker","vertex_colors","vertex_color_shaded","object_ids","face_ids","collection_ids","face_orientation","tessellation","normals","depth","silhouette"],"description":"orientation and topology (mesh-workbench's names) are also accepted for face_orientation and wireframe."},"look":{"type":"string","enum":["+x","-x","+y","-y","+z","-z","frame"],"description":"The side the camera stands on, or frame the mesh."},"replay":{"description":"A recorded state: next, previous, live, play, stop, or {session, event}.","oneOf":[{"type":"string","enum":["next","previous","live","play","stop"]},{"type":"object","properties":{"session":{"type":"string"},"event":{"type":"integer","minimum":0}},"additionalProperties":false}]},"turntable":{"type":"boolean","description":"Turn the camera about the mesh."},"pose":{"description":"rest, stop, or {clip, time} / {clip, play: true}.","oneOf":[{"type":"string","enum":["rest","stop"]},{"type":"object","properties":{"clip":{"type":"string"},"time":{"type":"number"},"play":{"type":"boolean"}},"required":["clip"],"additionalProperties":false}]},"bones":{"type":"boolean","description":"Draw the rig's joints and bones."},"surfaces":{"type":"boolean","description":"false hides the mesh surfaces (bones only)."},"solo_match":{"type":"string","description":"Show only objects whose name contains this text."},"hide_match":{"type":"string","description":"Hide objects whose name contains this text."},"solo_collection":{"type":"integer","minimum":0,"description":"A collection id."},"hide_collection":{"type":"integer","minimum":0,"description":"A collection id."},"event":{"type":"integer","minimum":1,"description":"Return what command N of the session sent and its reply."},"grid":{"type":"boolean","description":"false hides the ground grid."},"shadow":{"type":"boolean","description":"false hides the contact shadow."},"panel":{"type":"boolean","description":"false closes the Objects and Replay panel on the picture; true opens it."},"labels":{"type":"boolean","description":"false hides the vertex and face ID labels of the wireframe (topology) shade."},"command_filter":{"type":"string","description":"Text that narrows the Replay panel's commands by name. Empty lists the latest."},"workspace":{"type":"string","description":"Open another mesh-workbench workspace: a directory, absolute or under the current directory. One with no project.json is an empty workspace."},"projection":{"type":"string","enum":["perspective","orthographic"],"description":"orthographic is a parallel projection: no perspective."},"filter":{"type":"string","description":"Text that narrows the window's Objects panel. Empty lists everything."}},"additionalProperties":false}"#,
        "mesh_selection" => r#"{"type":"object","properties":{"objects":{"type":"array","items":{"type":"integer","minimum":0},"description":"Replace the selected objects."},"faces":{"type":"array","items":{"type":"integer","minimum":0},"description":"Replace the selected faces."},"vertices":{"type":"array","items":{"type":"integer","minimum":0},"description":"Replace the selected vertices."},"note":{"type":"string","description":"What the person wants done with the selection."},"clear":{"type":"boolean","description":"true empties the selection first."},"grow":{"type":"string","enum":["connected","loop"],"description":"connected adds everything joined to the selection; loop adds the ring of four-sided faces through two selected neighbouring faces."}},"additionalProperties":false}"#,
        "mesh_pick" => r#"{"type":"object","properties":{"screen":{"type":"array","items":{"type":"number","minimum":0,"maximum":1},"minItems":2,"maxItems":2,"description":"[x, y], origin top left. Default the centre."},"mode":{"type":"string","enum":["face","object","vertex"]},"select":{"type":"string","enum":["set","add","none"]},"box":{"type":"array","items":{"type":"number"},"minItems":4,"maxItems":4,"description":"[x0, y0, x1, y1] in screen coordinates: select everything inside by mode."},"lasso":{"type":"array","items":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2},"minItems":3,"description":"A closed path of [x, y] screen points: select everything inside by mode."},"through":{"type":"boolean","description":"With box or lasso: true also selects what is behind another surface. Default false."}},"additionalProperties":false}"#,
        "mesh_help" => r#"{"type":"object","properties":{"topic":{"type":"string","description":"A help topic, for example command-api."},"all":{"type":"boolean","description":"true lists every topic."},"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1}},"additionalProperties":false}"#,
        "mesh_command" => r#"{"type":"object","properties":{"command":{"description":"One mesh-workbench command object, or a list of up to 1024.","oneOf":[{"type":"object","properties":{"command":{"type":"string","description":"The mesh-workbench command name."}},"required":["command"],"additionalProperties":true},{"type":"array","items":{"type":"object","properties":{"command":{"type":"string","description":"The mesh-workbench command name."}},"required":["command"],"additionalProperties":true},"minItems":1,"maxItems":1024}]}},"additionalProperties":false,"required":["command"]}"#,
        "mesh_object" | "mesh_select" | "mesh_material" | "mesh_rig" | "mesh_animation" | "mesh_query" | "mesh_document"
        | "mesh_edit" => r#"{"type":"object","properties":{"command":{"type":"string","description":"The mesh-workbench command name."}},"required":["command"],"additionalProperties":true}"#,
        _ => return None,
    };
    json::parse(text).ok()
}

fn plain_specs() -> Vec<CommandSpec> {
    vec![
        spec("status", "Frame, pause state, view, camera, and whether the lighting has settled (stable, still_frames, picture_settled, cache_rays)."),
        spec("knobs", "List every knob the example publishes, with range and step. With name and value, set one first."),
        spec("lights", "Every light: position, colour, range, distance, in_view, screen_share, impact (colour luminance x screen share), on."),
        spec(
            "light",
            "Edit a light: add (position, color, intensity), or index with on, color, intensity, position, path [[x,y,z],..] + frames (+ loop), remove, reset, solo; solo_off clears a solo and does not need an index.",
        ),
        spec("cone_stats", "The cone-traced cascades' counts from the last frame, one entry per cascade: probes the hash held, cap (the probe table size), dropped_compact (probes past the cap, never traced), dropped_insert (pixels whose probe found no hash slot), cones_traced, cones_closed, seat_fallbacks (probes whose voxel the field had no surface for, seated at the cell centre); lamp_dropped, the lamp points the per-frame lamp hash had no slot for; and field, the resident voxel field (points, voxels, bricks, MiB, build seconds; its fine levels are clip volumes about the eye, built again when it moves 2 m). Empty when cone mode has not run. Waits for that frame."),
        spec("cone_probes", "One cascade's probes from the last frame (cascade, default 0): key, lod, cell, pixels, seat. around [x,y,z] and radius (default 2 m) keep the probes whose seat is that near. Waits for that frame. Reply only; writes no file."),
        spec("level0_voxels", "Level-0 cascade voxels within radius (default 0.75 m) of around [x,y,z]: cell, level, axis, position, radiance, filled ring slots, and the four parent cells at twice the size (found, position, weight, radiance). Reply only; writes no file."),
        spec("objects", "Every solid: index, position, yaw, size, colour, edited."),
        spec("object", "Edit solid index: position, yaw, spin (rad/s), freeze (true/false), path [[x,y,z],..] + frames (+ loop), roughness and metallic (0-1, reference trace only until the specular pass lands), reset. Unknown fields are an error."),
        spec("camera", "Set position [x,y,z], yaw, pitch, look_at [x,y,z], or path [[x,y,z,yaw,pitch],..] over frames. Returns the pose."),
        spec("time", "pause (bool), fixed_dt (s; 0 or real_time: true for wall time)."),
        spec("step", "Advance a paused scene by frames (default 1) and reply after them. accepted is the present count when the step was taken. frame is accepted plus frames."),
        spec("wait", "Reply after frames (default 1). Scene time runs unless paused. accepted is the present count when the wait was taken. frame is accepted plus frames. A status read from the previous command is one present behind accepted: that command returned while the next present was already open, and take() had already run."),
        spec(
            "wait_settled",
            "Reply when the lighting has settled and a frame drawn from its held light is on screen, for quiet_frames (default 3), or after timeout_frames (default 600). Reply says settled true/false, frames, wall_ms, still_frames, frame, and settled_frame. frames counts presents until settle and does not include then_frames. then_frames (optional) draws that many presents after the settle present before replying; with path, width, and height it saves the last of those presents. settled_frame is the settle present. frame is the saved present, so the pixels and the id are that same present. A capture command sent after this reply cannot name that present: it has already been drawn.",
        ),
        spec(
            "view",
            "What the picture shows: mode full, direct, bounce, nobounce (direct), albedo, normal, depth, light:I, texels, variance, path, voxels.",
        ),
        spec(
            "capture",
            "Save the picture: path (relative to the shot dir), mode (a view), width/height (resampled), name (keep in memory for diff), region [x,y,w,h] in 0..1 for the luminance in the reply. ui (default false) leaves out the debug menu, the menu tab, the frame-rate plate, and tool overlays. frame (optional) is a present index still to be drawn. The saved pixels are that present and the reply's frame is the same index. A frame already presented is an error. The targeted present with no pixels is an error and is not replaced by a later present.",
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
            "Ground truth: path-trace the scene as drawn this frame from the camera pose on the CPU (unlimited bounce, next-event direct light, no probes), off the frame thread. Traces in passes of spp paths per pixel (default 64) until the mean relative noise of a pixel is at most noise (default 0.03), max_spp (default 16384) or seconds (default 300) run out; bounces (default inf; 0 = direct only), width/height (default the picture, at most 320 wide), name (stored as, default reference), prefix (files <prefix>-reference.png), aov (full, default; or specular: also write <prefix>-reference-spec.png, the GGX lobe of solids given roughness/metallic by object, alone and without fog; such traces skip the cache and gpuref). A glTF scene is traced as its triangles (textured albedo and emission, alpha holes) together with the analytic shapes; the reply's geometry is then shapes+gltf. Pause first: the trace is of one moment. A trace is kept in cache (default target/reference-cache or $GENOS_REFERENCE_CACHE; \"off\" never) and reused for the same scene, pose, size and bounces when it is as converged as asked. measure (bool, default false; scripted/headless): exactly one frame (one lighting update) is drawn after the request, frames is ignored, then the frame thread joins the CPU trace, so window events wait until it ends. Replies with the path, spp done, noise, trace_seconds, coverage.",
        ),
        spec(
            "compare",
            "Trace a reference (as `reference`, or reuse a stored one: reference: name), then capture the live picture (mode, default full; frames: N waits N frames after the trace starts; measure: true takes the frame drawn after the request (after frames) as live, then joins the trace: one lighting update, window events wait; or live: a capture name/PNG) and compare in linear luminance on block x block averages (default 2; the reference's noise falls with the block side). Saves <prefix>-reference.png, -live.png, -heatmap.png (red too bright, blue too dark, full at 50% error). Replies mean_abs, mean_rel (share of the light that is wrong), p95_abs, p95_rel, bias (+ too bright), ref_mean, live_mean and regions (grid, default [3,3], row-major mean_rel and bias). region limits it. ui (default false) leaves out the debug menu, the menu tab, the frame-rate plate, and tool overlays.",
        ),
        spec("timing", "Frame time breakdown over the last frames (default 60): fps, frame, draw CPU, GPU."),
        spec(
            "gi_metrics",
            "GI pass times and work counters since the last reset. Passes: direct, atrous, compose, trace, filter, gather, resolve, and lighting. Counters: rays, cache_entries_refreshed, lamps_shaded. spec_ms is the specular pass and reads 0 until that pass exists (ADR 0024). resolve is gather + filter + compose and is not added again into lighting. reset true clears the window and <out>/gi/frames.csv. write NAME saves <out>/gi/NAME.csv (p50, p99) and NAME-frames.csv. Requires --gi-metrics at launch. GENOS_GI_METRICS applies only when that argument was omitted. One frame behind the draw.",
        ),
        spec(
            "get_lighting_settings",
            "The lighting settings in force (text form) and the last collected GPU pass: name, milliseconds, and dispatch count. One frame behind the draw that just finished.",
        ),
        spec(
            "set_lighting_settings",
            "Partial lighting update. Keys match the settings text (direct, shadows, gi, bounces, probes, …). A text field overlays key=value lines. The scene keeps the result; a renderer override is cleared. Applies on the next draw.",
        ),
        spec("list_presets", "fallback, day-cycle, gi-1bounce, low, medium, and high, each with the settings text that preset writes."),
        spec(
            "apply_preset",
            "Replace the scene lighting with preset name (fallback, day-cycle, gi-1bounce, low, medium, high) and clear a renderer override.",
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
    .chain(under_trusted_specs())
    .chain(reflection_specs())
    .chain(hdr_specs())
    .chain(fog_specs())
    .chain(particle_specs())
    .chain(animation_specs())
    .chain(host_specs())
    .chain(material_specs())
    .collect()
}

fn under_trusted_specs() -> Vec<CommandSpec> {
    vec![
        spec(
            "lighting_under_trusted",
            "Displaying picture pixels whose fine world-cache read was under a full set count (gstats.diag[57], LIGHTING_DIAG_UNDER_TRUSTED). A full count of a real zero is dark and is not included. One frame behind the draw.",
        ),
        spec(
            "lighting_cascade_gaps",
            "Visible surfaces last frame whose published bounce sample was empty (gstats.diag[11], LIGHTING_DIAG_CASCADE_GAP). A real zero has weight and is not counted. One frame behind the draw.",
        ),
        spec(
            "lighting_fill_compact_work",
            "Fill scheduler work from the last frame: serial is gstats.pad2 (the one-thread plan's iteration count) and hist_groups is gstats.pad3 (the histogram dispatch's group count). A slot-table scan is hundreds of thousands of serial iterations. One frame behind the draw.",
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
            "The HDR settings in force (text form). --exposure-mode, --ev100, --compensation, --iso, --aperture, --shutter, --adapt-down, --adapt-up, --adapt-strength, --highlight-percentile, --highlight-stops, --hdr-display, --bloom, --bloom-intensity, --bloom-levels, --output, and --hdr-transition win over GENOS_HDR_*. --out wins over GENOS_HDR_OUT. A GENOS_HDR_* mirror applies only when that argument was omitted. --display selects the transform only when its value is agx, aces, or legacy; otherwise it is the Wayland display name.",
        ),
        spec(
            "set_hdr_settings",
            "Partial HDR update. Keys match the settings text (exposure_mode, ev100, compensation, iso, aperture, shutter, adapt_down, adapt_up, adapt_strength, adapt_anchor_seconds, adapt_knee_stops, highlight_percentile, highlight_stops, histogram_bins, display, bloom, bloom_intensity, bloom_levels, output, transition, legacy_bytes). transition is the seconds a change of ev100, compensation, or the bloom intensity takes to reach the picture; 0 applies it on the next frame. A text field overlays key=value lines. Writes the scene. The arguments and GENOS_HDR_* mirrors still overlay the value the next draw uses.",
        ),
        spec(
            "save_hdr_to_scene",
            "Write the scene's HDR look into the scene file the loaded scene was read from, so the next start of that scene opens with it. Takes no fields. The file's hdr lines are replaced by one `hdr KEY VALUE` line for each of exposure_mode, ev100, compensation, iso, aperture, shutter, adapt_down, adapt_up, display, bloom, bloom_intensity, bloom_levels, and transition. output, histogram_bins and legacy_bytes are not written. Every other line of the file is kept. The value written is the scene's (set_hdr_settings or the Output page), without the arguments and the GENOS_HDR_* mirrors. Replies scene, path, and lines (the lines written). A scene with no file is refused.",
        ),
        spec(
            "get_hdr",
            "HDR probe. ev100, exposure, and scales (the last 8 exposure scales). grade is scene. frame_dt is the seconds the last draw stepped auto exposure and the look transition by. still_pop is the share of subsampled scene luma that moved by more than 0.01. histogram is the last bin counts, also written to <out>/hdr-output/histogram.txt. surfaces lists the swapchain (format, color_space) in use. chosen is the output mode. resolved is the mode `resolve_swapchain` picked. pq_white and scrgb_white are `swapchain_nits` of graphics white. hdr_ms and post_ms_device are bloom plus the display pass, one frame behind, zero when the device has no timestamps. hdr_vram_bytes is the post buffers in bytes. post_vram_device is that sum in MiB. The 1.50 ms ceiling is the 4070 budget and the Deck at 800p. The gate applies it on a hardware device. A software rasterizer prints a finite positive timestamp and is not that score. ADR 0027's per-preset ceilings (High 37.50 MiB) are the table in the HDR pipeline record; the gate checks those bytes at 1440p against ceiling × 1024², for SDR, HDR10, and scRGB. There is no 96 MiB ceiling.",
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

fn particle_specs() -> Vec<CommandSpec> {
    vec![
        spec(
            "get_particle_settings",
            "Stage-1 particle settings in text form: preset, enabled, max_particles, count, sort, lit, depth_collision, freeze, seed, origin, rate, life, rise, spread, size, albedo, soft_band. --fx-max, --fx-count, --fx-freeze, --fx-preset, and --fx-spawn win over GENOS_FX_MAX, GENOS_FX_COUNT, GENOS_FX_FREEZE, and GENOS_FX_PRESET, which win over the scene value. sort and lit are stored. Stage 1 does not sort or light. half_res is a deprecated alias and the draw is full resolution. max_particles is clamped to 4194240.",
        ),
        spec(
            "set_particle_settings",
            "Partial particle update. Keys match the settings text. A text field overlays key=value lines. Writes the scene. Launch flags and their mirrors overlay the fields they set on the next frame.",
        ),
        spec(
            "get_particle_stats",
            "Last particle step: fx.sim_ms, fx.sim_init_ms, fx.sim_death_ms, fx.sim_emit_ms, fx.sim_compact_ms, fx.sort_ms, fx.draw_ms, fx.alive, fx.spawned, fx.overdraw_px, fx.vram_mb, plus max_particles, count, and draw_instances. sim_ms and draw_ms are timestamps, or 0 before a frame. The four sim_* pieces sit inside sim_ms. sort_ms and overdraw_px stay 0. They are not a desktop-GPU score. vram_mb is decimal megabytes of the stage-1 buffers.",
        ),
        spec(
            "fx.stats",
            "Same counters as get_particle_stats. The design's name for the probe.",
        ),
        spec(
            "fx.set",
            "Set one stage-1 key. param is the settings key, value is the text. emitter may be stage1. A .gfx path is not parsed.",
        ),
        spec(
            "fx.list",
            "The one stage-1 emitter, named stage1. Curve files are a later stage.",
        ),
        spec(
            "fx.spawn",
            "Place the stage-1 emitter at x, y, z and turn it on. count is optional. Omitted, a count of 0 becomes 64 so the step dispatches. A file path is accepted and ignored: stage 1 has no .gfx parser.",
        ),
        spec(
            "fx_stats",
            "Same counters as get_particle_stats. Rhai name for fx.stats.",
        ),
        spec(
            "fx_set",
            "Same as fx.set. Rhai name: param is the settings key, value is the text.",
        ),
        spec(
            "fx_list",
            "Same as fx.list. Rhai name for the stage-1 emitter list.",
        ),
        spec(
            "fx_spawn",
            "Same as fx.spawn. Rhai name. Omitted count on a zero emitter becomes 64.",
        ),
    ]
}

fn animation_specs() -> Vec<CommandSpec> {
    vec![
        spec(
            "get_animation_settings",
            "The animation settings text: max_skinned, update_rate_far_hz, far_distance_m, dqs, ik. Low is 32, 15 Hz, 25 m, dqs off, ik off. The value lives on the character component. genos-animation --settings prints the Low row. Clip bytes are anim.clip_mb and are not in the 64 MB skin budget.",
        ),
        spec(
            "set_animation_settings",
            "Partial animation update. Keys are max_skinned, update_rate_far_hz, far_distance_m, dqs, and ik, with or without the animation. prefix. genos-animation --set key=value checks one knob and prints the text. The character keeps the value. Locomotion params are local. This command does not send a pose.",
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
            "The debug menu in force (text form): visible (shown or hidden), tab (page title), collapsed (folded section keys), wheel, keys, and place. wheel and keys are hover, modal, click, or passive. place is docked or floating: where the editor's top bar opens a category (a drawer on the right edge, or a panel that drags); the tabbed menu does not read it. Shown by default, with hover for the wheel and click for keys. --menu wins over GENOS_MENU, which wins over the scene value. tab, collapsed, wheel, and keys are the stored fields.",
        ),
        spec(
            "set_menu_settings",
            "Partial debug-menu update. Keys are visible (shown or hidden), tab, collapsed (comma-separated section keys), fold (add one key), open (remove one key), wheel and keys (hover, modal, click, or passive), and place (docked or floating). focus_wheel and focus_keys are the same as wheel and keys. A text field overlays key=value lines. Writes the scene. --menu and GENOS_MENU still overlay visible.",
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
        spec(
            "mesh_view",
            "The mesh-workbench workspace a mesh scene shows: workspace directory, loaded, revision, vertex and face counts, objects (id, name, origin, triangles, hidden), pick, shade, replay, sessions, editing (a mesh_command has opened the session), and lift (metres the picture raises the mesh so it stands on the ground). pick (face, object, or vertex) sets what a click selects. hide, show and solo take object ids, and show_all: true shows every object; hiding is local to the viewer. shade is shaded, wireframe (topology: the polygon edges, with each visible face's and vertex's ID as a label unless labels: false), tessellation (every triangle edge), object_ids, face_ids, collection_ids (mesh-workbench's ID colours), silhouette, face_orientation (orientation: red seen from behind), textured and uv_checker (a face with no UVs is magenta), vertex_colors, normals (RGB = (N + 1) / 2) and depth (255 nearest, 32 farthest), and the lit forms material (textured) and vertex_color_shaded. The ID shades, silhouette, textured, uv_checker, vertex_colors, normals and depth switch the view to the base colour (unlit), as mesh-workbench draws them. The reply lists collections (id, name, objects); solo_collection and hide_collection take a collection id. event: N returns what command N of the session sent and its reply. look is +x, -x, +y, -y, +z, -z (the side the camera stands on) or frame. replay: {session (an id, default latest), event} shows the recorded state after that command (0 is the session start) replay: \"next\" and \"previous\" step one command, and replay: \"live\" returns to the live mesh; the reply lists sessions with their latest commands. turntable: true turns the camera about the mesh. grid: false hides the ground grid. shadow: false hides the contact shadow, the soft dark patch on the ground under each object. projection is perspective or orthographic (a parallel projection). workspace opens another workspace directory (the reply lists the workspaces found on disk as workspaces). filter is the text that narrows the window's Objects panel (typed there with Shift and T), and command_filter the Replay panel's commands (Control and T). pose: {clip, time} shows the mesh at that time of an animation clip, pose: {clip, play: true} plays it in a loop, pose: \"stop\" holds it and pose: \"rest\" returns to rest; replay: \"play\" steps through the session every 0.7 s to the live mesh and replay: \"stop\" holds it; solo_match and hide_match take text and show only, or hide, the objects whose name contains it; bones: true draws the rig, and surfaces: false hides the mesh surfaces so the bones and the wireframe show; the reply lists clips (name, duration), pose, and rig_bones. Needs a mesh scene.",
        ),
        spec(
            "mesh_selection",
            "What the person selected in the mesh viewer, in mesh-workbench IDs: objects, object_names, faces, vertices, point (last picked surface point, mesh coordinates), note (what they typed), revision, and the camera in mesh coordinates. This is the viewer's selection, not the mesh-workbench vertex selection its move, scale and rotate use. objects, faces and vertices (lists of ids) replace that part; note sets the note; clear: true empties it first. grow: connected adds every face or vertex joined to the selection; grow: loop adds the ring of four-sided faces that runs through two selected neighbouring faces.",
        ),
        spec(
            "mesh_pick",
            "Pick the mesh under screen [x,y] (0..1, origin top left, default the centre). mode is face (default), object, or vertex. select is set (default: replace the selection with the hit), add, or none (report only). Replies hit (object, object_name, face, face_vertices, vertex, point) or null, and the selection. box: [x0,y0,x1,y1] or lasso: [[x,y],...] selects everything inside instead (faces by their centre, vertices, or objects with a face inside), only what the camera sees unless through: true, and replies added. In the window, Control and a left drag draws the box and Alt and a left drag the lasso; Shift adds.",
        ),
        spec(
            "mesh_object",
            "Mesh section. One mesh-workbench command for objects and collections: object_* and collection_* (create, rename, move, rotate, duplicate, instance, join, separate, delete, stage), and cube. The arguments are the command object itself, for example {\"command\": \"...\", ...}; a command of another family is refused and names its tool. mesh_help reads the parameters. Needs a mesh scene; the first call opens the session on the workspace.",
        ),
        spec(
            "mesh_select",
            "Mesh section. One mesh-workbench command for mesh-workbench's own vertex selection, which its move, scale and rotate act on: select and select_*. The person's selection in the viewer is mesh_selection. The arguments are the command object itself, for example {\"command\": \"...\", ...}; a command of another family is refused and names its tool. mesh_help reads the parameters. Needs a mesh scene; the first call opens the session on the workspace.",
        ),
        spec(
            "mesh_material",
            "Mesh section. One mesh-workbench command for UVs, textures and vertex colours (materials): uv_*, texture_* (import, create, assign, layers, masks, bake, maps), vertex_color_*, capture_uv, and their query_ and inspect_ commands. The arguments are the command object itself, for example {\"command\": \"...\", ...}; a command of another family is refused and names its tool. mesh_help reads the parameters. Needs a mesh scene; the first call opens the session on the workspace.",
        ),
        spec(
            "mesh_rig",
            "Mesh section. One mesh-workbench command for bones and skin weights: rig_set, rig_clear, rig_display_tails_set, skin_weights_set, query_rig. The arguments are the command object itself, for example {\"command\": \"...\", ...}; a command of another family is refused and names its tool. mesh_help reads the parameters. Needs a mesh scene; the first call opens the session on the workspace.",
        ),
        spec(
            "mesh_animation",
            "Mesh section. One mesh-workbench command for animation clips, tracks and keys: animation_* (set, update, remove, track_set, track_remove, keys_edit, ik_bake, sample) and query_animation. The arguments are the command object itself, for example {\"command\": \"...\", ...}; a command of another family is refused and names its tool. mesh_help reads the parameters. Needs a mesh scene; the first call opens the session on the workspace.",
        ),
        spec(
            "mesh_query",
            "Mesh section. One mesh-workbench command for reading the mesh without changing it: inspect, inspect_*, query_*, measure_clearance, capture (a rendered PNG), and help. The arguments are the command object itself, for example {\"command\": \"...\", ...}; a command of another family is refused and names its tool. mesh_help reads the parameters. Needs a mesh scene; the first call opens the session on the workspace.",
        ),
        spec(
            "mesh_document",
            "Mesh section. One mesh-workbench command for the document as a whole: undo, redo, transaction, save, load, import_glb, import_fbx, export, and the workflow_*, review_* and reference commands. The arguments are the command object itself, for example {\"command\": \"...\", ...}; a command of another family is refused and names its tool. mesh_help reads the parameters. Needs a mesh scene; the first call opens the session on the workspace.",
        ),
        spec(
            "mesh_edit",
            "Mesh section. One mesh-workbench command for geometry edits and generators: move, scale, rotate, extrude, inset, bevel, bridge, split, weld, dissolve, delete_faces, triangulate, bisect_plane, shade_smooth, sweep, revolve, loft, extrude_polygon, and every other command. The arguments are the command object itself, for example {\"command\": \"...\", ...}; a command of another family is refused and names its tool. mesh_help reads the parameters. Needs a mesh scene; the first call opens the session on the workspace.",
        ),
        spec(
            "mesh_help",
            "Mesh section. mesh-workbench's own help: with no field, the task index; all: true lists every topic; topic reads one (for example rigging-and-animation, uv-editing-and-vertex-painting, command-api); offset and limit page a long topic. Needs a mesh scene.",
        ),
        spec(
            "mesh_context",
            "Everything an agent needs to act on what the person asked for in the mesh viewer, in one reply: units (metres, Y up), intent (the note they typed), selection (objects, faces, vertices, the picked point, the camera in mesh coordinates), selected_faces (each face's object, vertex ids, centre and normal, up to 48), visible_objects (id, name, triangles, bounds), hidden_objects, view (shade, projection, pick, replay, preview), recent_edits (the latest commands that changed the mesh, each with its request, whether it ran, and the note it ran under), and screenshot (the picture as it is now, saved as a file; screenshot: false leaves it out). edits sets how many edits are listed. Needs a mesh scene.",
        ),
        spec(
            "mesh_preview",
            "Show an edit before it is committed. commands (a list of mesh-workbench commands, with optional guards) are prepared by mesh-workbench as one transaction and not committed; the reply is its report (diff, new defects, preview_id) and the picture shows the mesh as it would be. commit: true then commits that transaction as one undo step; discard: true drops it. Any other edit, or a second preview, replaces it. The result shown is made on a copy of the workspace under the output directory. Needs a mesh scene in genos-editor.",
        ),
        spec(
            "mesh_commands",
            "The mesh-workbench commands as its own help documents them. With no field: each command, the tool it goes to (mesh_object, mesh_edit, ...) and the names of its fields; family lists one tool's commands. command: NAME gives that command's reference text (each field's form and what it does) and a JSON schema that names its fields. Read from mesh-workbench's built-in reference, so it follows the linked version; field types are in the text. Needs a mesh scene in genos-editor.",
        ),
        spec(
            "mesh_command",
            "Run command (one mesh-workbench command object such as {\"command\":\"inspect\"}, or a list of up to 1024 that stops at the first refusal) on the mesh scene's workspace and return mesh-workbench's reply. The engine links mesh-workbench; the first call opens its session on the workspace, which then holds project.json and the recorded sessions. The picture reloads when the mesh changes.",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::{public_name, section_of, specs, SECTIONS};

    #[test]
    fn every_command_is_in_one_known_section_and_says_so() {
        let specs = specs();
        for spec in &specs {
            let section = section_of(&spec.name);
            assert!(SECTIONS.contains(&section), "{} has section {section}", spec.name);
            assert!(spec.description.starts_with(&format!("[{section}] ")), "{}", spec.name);
        }
        // Every public name is its section, a dot and a name, and no two agree.
        let names: Vec<String> = specs.iter().map(|spec| public_name(&spec.name)).collect();
        for (at, name) in names.iter().enumerate() {
            assert!(!names[..at].contains(name), "two commands are published as {name}");
            assert!(name.contains('.'), "{name}");
        }
        for (name, public) in [
            ("get_lighting_settings", "lighting.get_settings"),
            ("lighting_cascade_gaps", "lighting.cascade_gaps"),
            ("lights", "lighting.lights"),
            ("mesh_view", "mesh.view"),
            ("set_fog_settings", "render.set_fog_settings"),
            ("save_hdr_to_scene", "render.save_hdr_to_scene"),
            ("fx.spawn", "fx.spawn"),
            ("capture", "capture.capture"),
            ("load_scene", "scene.load"),
            ("list_scenes", "scene.list_scenes"),
        ] {
            assert_eq!(public_name(name), public);
        }
        for (name, section) in [
            ("set_lighting_settings", "lighting"),
            ("lights", "lighting"),
            ("apply_preset", "lighting"),
            ("lighting_cascade_gaps", "lighting"),
            ("set_fog_settings", "render"),
            ("fx.spawn", "render"),
            ("load_scene", "scene"),
            ("camera", "scene"),
            ("capture", "capture"),
            ("flicker", "capture"),
            ("knobs", "editor"),
            ("set_menu_settings", "editor"),
            ("mesh_rig", "mesh"),
            ("mesh_selection", "mesh"),
        ] {
            assert_eq!(section_of(name), section, "{name}");
        }
        // No section is empty.
        for section in SECTIONS {
            assert!(specs.iter().any(|spec| section_of(&spec.name) == section), "{section} is empty");
        }
    }
}
