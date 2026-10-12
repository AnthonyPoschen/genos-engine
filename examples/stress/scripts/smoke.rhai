// Smoke test of the debug tools: every view, a light edit, a diff and a flicker window.
time(#{ pause: true });
knob("time_of_day", 12.0);
knob("sun", "freeze");
camera(#{ position: [12.5, 1.7, 20.0], look_at: [12.5, 1.0, 8.0] });
// GENOS_LIGHTING_SC_SETTLE (make verify sets 32) is the still-frame count, with a
// floor of 32. timeout_s covers a software GPU at a couple of seconds a frame.
let settled = wait_settled(#{ timeout_frames: 160, timeout_s: 400 });
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

// Picture reuse, after the views. Back on the full picture, a still screen skips
// the present, a dirty rectangle is scissored, and a moved solid makes the
// verification drop the cached picture.
view("full");
let skipped_at = frame_reuse().skipped;
let last = frame_reuse();
let saw_skip = false;
for n in 0..100 {
    wait(#{ frames: 1, timeout_s: 30 });
    last = frame_reuse();
    if last.skipped > skipped_at {
        saw_skip = true;
        break;
    }
}
log(`reuse before skip drawn ${last.scene_drawn} skipped ${last.skipped} invalidated ${last.invalidated} checks ${last.checks} changed ${last.changed} tests ${last.tests} fails ${last.fails} verify_ok ${last.verify_ok}`);
require(saw_skip, "a still screen was not skipped");

let ui = frame_reuse();
overlay(#{ x: 8, y: 8, w: 48, h: 28 });
wait(#{ frames: 4, timeout_s: 60 });
let repaired = frame_reuse();
log(`reuse after UI drawn ${repaired.scene_drawn} ui_repaired ${repaired.ui_repaired} full_restore ${repaired.full_restore} rects ${repaired.rects} skipped ${repaired.skipped}`);
require(repaired.ui_repaired > ui.ui_repaired, "dirty UI was not scissored onto the cached scene");
require(repaired.rects > ui.rects, "a UI repair redrew no rectangle");
overlay(#{});

let before = frame_reuse();
let objs = objects();
let moved = objs[0].position;
object(#{ index: 0, position: [moved[0] + 1.0, moved[1], moved[2]] });
wait(#{ frames: 6, timeout_s: 60 });
let dropped = frame_reuse();
log(`reuse after move drawn ${dropped.scene_drawn} invalidated ${dropped.invalidated} checks ${dropped.checks} changed ${dropped.changed} tests ${dropped.tests} fails ${dropped.fails}`);
require(dropped.scene_drawn > before.scene_drawn, "moving a solid did not redraw the scene");
require(dropped.invalidated > before.invalidated, "verification did not drop the cached picture");
