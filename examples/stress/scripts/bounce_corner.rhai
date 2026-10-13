// Bounce lag, for the eye. Windowed on the owner's PC, 1920x1080.
//
// What to watch: the corner where the two walls meet, low and away from the
// lamp. Not the lamp bulb.
//
// Direct light and the shadow edge must move with the lamp, on the same frame.
// A shadow that arrives a frame late fails the run.
// The bounce in that corner, the light no lamp reaches directly, may keep the
// old tint for a few frames after the lamp stops. It should then catch the
// new place. A corner that stays on the old tint fails the run.
//
// Shots land in the --report directory (use ~/genos-perf/bounce-corner).
fn dist2(a, b) {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    dx * dx + dy * dy + dz * dz
}

time(#{ fixed_dt: 0.0166667 });
knob("sun", "freeze");
knob("boxes", "still");
knob("dynamic", 0.0);
camera(#{ position: [2.4, 1.55, 2.3], look_at: [0.25, 0.55, 0.25] });
wait(#{ frames: 45 });
let eye = camera().position;
check_lt(dist2(eye, [2.4, 1.55, 2.3]), 0.01, "the camera stayed at the corner");
view("direct");
shot("before-direct");
view("bounce");
shot("before-bounce");
view("full");
let before = shot("before-full");
// A flat grey wall has almost no spread. The corner, the floor and the lamp do.
check_gt(before.std, 0.02, "the corner picture is not a flat wall");
let lamps = lights();
require(lamps.len() == 1, "the corner has one lamp");
let lamp = lamps[0];
let start = lamp.position;
light(#{
    index: lamp.index,
    path: [[1.6, 1.55, 1.5], [0.85, 1.55, 1.15]],
    frames: 20
});
wait(#{ frames: 10 });
let mid = lights()[0].position;
check_gt(dist2(start, mid), 0.04, "the lamp moved");
view("direct");
shot("moving-direct");
view("bounce");
shot("moving-bounce");
view("full");
shot("moving-full");
wait(#{ frames: 18 });
view("direct");
shot("stopped-direct");
view("bounce");
shot("stopped-bounce");
view("full");
shot("stopped-full");
wait(#{ frames: 8 });
view("bounce");
shot("settled-bounce");
view("direct");
shot("settled-direct");
view("full");
let settled = shot("settled-full");
check_gt(settled.std, 0.02, "the settled corner is not a flat wall");
let stopped = lights()[0].position;
check_lt(dist2(stopped, [0.85, 1.55, 1.15]), 0.0025, "the lamp stopped at the end of the path");
log(`lamp ${start} -> ${mid} -> ${stopped}`);
