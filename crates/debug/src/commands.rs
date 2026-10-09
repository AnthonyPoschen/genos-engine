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
        spec("status", "Frame, pause state, view, camera, and the probe counts (stable when no brick is due)."),
        spec(
            "lighting",
            "Read or change the lighting: tier_ms (probe budget), near_rays (-1/shader for the default), view_ms, bounces (0,1,2,inf), notice_band, spacing (m or auto), and the priority weights near, feed, margin, edge, stale.",
        ),
        spec("knobs", "List every knob (example and lighting.*) with range and step. With name and value, set one first."),
        spec(
            "probes",
            "Per-brick report: state (unlit/changing/refining/steady), age s, priority and its terms (seen, near_term, stale_term), spacing, skip reason. Filters: state, visible, near [x,y,z] + radius; sort priority|age|depth; limit; values true adds each live probe's face luminances (top and first cube) and samples.",
        ),
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
            "Reply when on-screen bricks have no passes left and the picture has blended their light in, for quiet_frames (default 3), or after timeout_frames (default 600). strict waits for every brick. Reply says settled true/false, frames, wall_ms and light_gpu_ms (GPU time of the light builds in the wait).",
        ),
        spec(
            "view",
            "What the picture shows: mode full, direct, bounce, nobounce, bounce:N/full:N (bounce limit 0,1,2,inf), near, far, albedo, normal, depth, light:I, probes:state|age|priority|spacing|changing. visible_only for probe markers.",
        ),
        spec(
            "capture",
            "Save the picture: path (relative to the shot dir), mode (a view), width/height (resampled), settle (wait for the light first), name (keep in memory for diff), region [x,y,w,h] in 0..1 for the luminance in the reply.",
        ),
        spec("captures", "Save several views in consecutive frames: modes [..], prefix, width, height, settle."),
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
            "Trace a reference (as `reference`, or reuse a stored one: reference: name), then capture the live picture (mode, default full; frames: N waits N frames after the trace starts; settle; or live: a capture name/PNG) and compare in linear luminance on block x block averages (default 2; the reference's noise falls with the block side). Saves <prefix>-reference.png, -live.png, -heatmap.png (red too bright, blue too dark, full at 50% error). Replies mean_abs, mean_rel (share of the light that is wrong), p95_abs, p95_rel, bias (+ too bright), ref_mean, live_mean and regions (grid, default [3,3], row-major mean_rel and bias). region limits it.",
        ),
        spec("timing", "Frame time breakdown over the last frames (default 60): fps, frame, draw CPU, GPU, light build passes, probe rays."),
        spec("quit", "Stop the example with exit code (default 0)."),
    ]
}
