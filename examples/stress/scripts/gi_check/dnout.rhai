// gi-check (S67B): day to night on open ground (Anthony, 2026-10-12: "the outside
// ground still looked like day and had to shift to night slowly"). Ground west of
// the building in the noon sun, settled; look away (the building), switch to
// midnight, pan back: frames 1..480 after the pan (t{n}.png) against the night
// pose settled (ref.png). gi-check dnout_rows: first-frame error and frames until
// it stays under 2 %. Sun frozen, boxes and lamps still. Filled in: __DIR__.
time(#{ fixed_dt: 1.0 / 60.0 });
knob("sun", "freeze");
knob("boxes", "still");
knob("lamps", 1.0);
knob("dynamic", 0.0);
knob("time_of_day", 12.0);
fn settle() {
    for k in 0..4 { try { wait_settled(#{ timeout_frames: 900 }); break; } catch (e) { log(`settle ${k}: ${e}`); } }
}
camera(#{ position: [-4.0, 1.7, 10.0], yaw: 1.5708, pitch: -0.5 });
settle();
capture(#{ path: "__DIR__/day.png" });
camera(#{ position: [-4.0, 1.7, 10.0], yaw: -1.5708, pitch: -0.1 });
wait(#{ frames: 120 });
knob("time_of_day", 0.0);
wait(#{ frames: 60 });
camera(#{ position: [-4.0, 1.7, 10.0], yaw: 1.5708, pitch: -0.5 });
let f = 0;
for n in [1, 4, 8, 16, 30, 60, 120, 240, 480] {
    wait(#{ frames: n - f });
    f = n;
    capture(#{ path: `__DIR__/t${n}.png` });
}
settle();
wait(#{ frames: 600 });
capture(#{ path: "__DIR__/ref.png" });
