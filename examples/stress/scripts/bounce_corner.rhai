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
time(#{ fixed_dt: 0.0166667 });
knob("sun", "freeze");
knob("boxes", "still");
knob("dynamic", 0.0);
camera(#{ position: [2.4, 1.55, 2.3], look_at: [0.25, 0.55, 0.25] });
wait(#{ frames: 45 });
view("direct");
shot("before-direct");
view("bounce");
shot("before-bounce");
view("full");
shot("before-full");
let lamps = lights();
require(lamps.len() > 0, "the corner has a lamp");
let lamp = lamps[0];
light(#{
    index: lamp.index,
    path: [[1.6, 1.55, 1.5], [0.85, 1.55, 1.15]],
    frames: 20
});
wait(#{ frames: 10 });
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
shot("settled-full");
