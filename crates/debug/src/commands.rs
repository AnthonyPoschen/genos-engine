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
        spec("status", "Frame, pause state, view, camera, and whether the lighting has settled (stable, still_frames, picture_settled)."),
        spec("knobs", "List every knob the example publishes, with range and step. With name and value, set one first."),
        spec("lights", "Every light: position, colour, range, distance, in_view, screen_share, impact (colour luminance x screen share), on."),
        spec(
            "light",
            "Edit a light: add (position, color, intensity), or index with on, color, intensity, position, path [[x,y,z],..] + frames (+ loop), remove, reset, solo; solo_off clears a solo.",
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
            "What the picture shows: mode full, direct, bounce, nobounce (direct), albedo, normal, depth, light:I, texels.",
        ),
        spec(
            "capture",
            "Save the picture: path (relative to the shot dir), mode (a view), width/height (resampled), name (keep in memory for diff), region [x,y,w,h] in 0..1 for the luminance in the reply.",
        ),
        spec("captures", "Save several views in consecutive frames: modes [..], prefix, width, height."),
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
            "Trace a reference (as `reference`, or reuse a stored one: reference: name), then capture the live picture (mode, default full; frames: N waits N frames after the trace starts; or live: a capture name/PNG) and compare in linear luminance on block x block averages (default 2; the reference's noise falls with the block side). Saves <prefix>-reference.png, -live.png, -heatmap.png (red too bright, blue too dark, full at 50% error). Replies mean_abs, mean_rel (share of the light that is wrong), p95_abs, p95_rel, bias (+ too bright), ref_mean, live_mean and regions (grid, default [3,3], row-major mean_rel and bias). region limits it.",
        ),
        spec("timing", "Frame time breakdown over the last frames (default 60): fps, frame, draw CPU, GPU."),
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
            "The debug output directory in force (text form). --out wins over GENOS_UNFILLED_DIR, which wins over the scene value. The default is genos-debug/unfilled under the working directory.",
        ),
        spec(
            "set_debug_settings",
            "Partial debug-output update. Key out is the unfilled directory. A text field overlays key=value lines. Writes the scene. --out and GENOS_UNFILLED_DIR still overlay the value used for the next save.",
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
            "get_menu_settings",
            "The debug menu in force (text form): visible (shown or hidden), tab (page title), and collapsed (folded section keys). Shown by default. --menu wins over GENOS_MENU, which wins over the scene value. tab and collapsed are the stored fields.",
        ),
        spec(
            "set_menu_settings",
            "Partial debug-menu update. Keys are visible (shown or hidden), tab, collapsed (comma-separated section keys), fold (add one key), and open (remove one key). A text field overlays key=value lines. Writes the scene. --menu and GENOS_MENU still overlay visible.",
        ),
        spec("quit", "Stop the example with exit code (default 0)."),
        spec(
            "list_scenes",
            "Names of the scenes this binary can load.",
        ),
        spec(
            "load_scene",
            "Load a scene by name. Drops light and object edits and clears the lighting cache. checkpoint (optional) jumps after the load; otherwise the first checkpoint is used.",
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
