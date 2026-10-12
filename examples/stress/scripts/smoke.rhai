// Smoke test of the debug tools: every view, a light edit, a diff and a flicker window.
time(#{ pause: true });
knob("time_of_day", 12.0);
knob("sun", "freeze");
camera(#{ position: [12.5, 1.7, 20.0], look_at: [12.5, 1.0, 8.0] });
let settled = wait_settled(#{ timeout_frames: 400, timeout_s: 300 });
require(settled.settled, "lighting did not settle");
log(`settled ${settled.settled} after ${settled.frames} frames, ${settled.wall_ms} ms wall`);
for mode in ["full", "direct", "bounce", "nobounce", "albedo", "normal", "depth", "texels"] {
    shot(`view-${mode}`, mode);
}
let full = luminance(#{});
check_gt(full.luminance, 0.01, "the noon picture is not black");
let d = diff(#{ mode_a: "full", mode_b: "direct", out: "diff-full-direct.png" });
check_gt(d.mean_abs, 0.0, "bounce light changes the picture");
let lights = lights();
log(`lights ${lights.len()}`);
let f = flicker(#{ frames: 8 });
check_lt(f.max_delta, 0.2, "a paused scene does not flicker");
timing();
