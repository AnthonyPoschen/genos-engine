// S1 spin. Hall checkpoint, boxes moving, half the lamps moving.
// 360 degrees of yaw in 120 frames (2 s at 1/60), repeated 3 times.
// The sun stays frozen, as the stress scene already sets it.
// Pass --gi-metrics. Samples are one frame behind the draw, so wait(2)
// lands before the summary. CSV: <out>/gi/s1-spin.csv
time(#{ fixed_dt: 1.0 / 60.0 });
knob("sun", "freeze");
knob("boxes", "move");
knob("lamps", "move");
knob("dynamic", 50);
jump_checkpoint(#{ name: "hall" });
wait(8);
gi_metrics(#{ reset: true });
let turn = 6.283185307179586 / 120.0;
let path = [];
for i in 0..361 {
    let yaw = -0.3554 + i.to_float() * turn;
    path.push([14.5, 1.7, 23.5, yaw, 0.08]);
}
let frames = path.len - 1;
camera(#{ path: path, frames: frames });
wait(frames);
wait(2);
gi_metrics(#{ write: "s1-spin" });
