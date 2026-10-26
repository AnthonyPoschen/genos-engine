// S5 A to B to A. Hall to room-a and back, 5 cycles, 240 frames (4 s) per leg.
// Yaw delta -2.2006 is already the short arc. Pass --gi-metrics.
// CSV: <out>/gi/s5-aba.csv
time(#{ fixed_dt: 1.0 / 60.0 });
knob("sun", "freeze");
knob("boxes", "move");
knob("lamps", "move");
knob("dynamic", 50);
jump_checkpoint(#{ name: "hall" });
wait(8);
gi_metrics(#{ reset: true });
let frames_leg = 240;
let path = [];
for cycle in 0..5 {
    for i in 0..(frames_leg + 1) {
        let t = i.to_float() / frames_leg.to_float();
        path.push([
            14.5 + (7.2 - 14.5) * t,
            1.7,
            23.5 + (0.9 - 23.5) * t,
            -0.3554 + (-2.556 - -0.3554) * t,
            0.08 + (-0.12 - 0.08) * t
        ]);
    }
    for i in 0..(frames_leg + 1) {
        let t = i.to_float() / frames_leg.to_float();
        path.push([
            7.2 + (14.5 - 7.2) * t,
            1.7,
            0.9 + (23.5 - 0.9) * t,
            -2.556 + (-0.3554 - -2.556) * t,
            -0.12 + (0.08 - -0.12) * t
        ]);
    }
}
let frames = path.len - 1;
camera(#{ path: path, frames: frames });
wait(frames);
wait(2);
gi_metrics(#{ write: "s5-aba" });
