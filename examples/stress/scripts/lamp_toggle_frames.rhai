// Dump a lamp toggle for layering-gate --capture.
// Frame 0000 is before the change. Later frames are after the lamp goes off.
// Fixed step 1/60 s. Sun frozen, boxes still. No path trace: dim_error stays
// NOT-IMPLEMENTED. Warmup is short so lavapipe can finish; the grade is the
// toggle itself, not a soaked cache.
time(#{ fixed_dt: 0.0166667 });
knob("time_of_day", 22.0);
knob("sun", "freeze");
knob("boxes", "still");
knob("dynamic", 0.0);
camera(#{ position: [12.5, 1.7, 20.0], look_at: [12.5, 1.0, 8.0] });
wait(30);
let lamps = lights();
let target = lamps.filter(|l| l.in_view && !l.directional);
require(target.len() > 0, "a lamp is in view");
let lamp = target[0];
fn pad(i) {
    if i < 10 {
        `000${i}`
    } else if i < 100 {
        `00${i}`
    } else {
        `0${i}`
    }
}
capture(#{ path: "frames/frame-0000.png" });
light(#{ index: lamp.index, color: [0.0, 0.0, 0.0] });
let i = 1;
let n = 36;
while i <= n {
    capture(#{ path: `frames/frame-${pad(i)}.png` });
    i += 1;
}
log(`dumped ${n + 1} frames after turning lamp ${lamp.index} off`);
