// S2 strafe. 6 m/s lateral for 3 s (180 frames) through the hall, movers on.
// Yaw 0 looks down -Z. Right is (cos(yaw), sin(yaw)) at yaw -0.3554.
// 0.1 m per frame. Pass --gi-metrics. CSV: <out>/gi/s2-strafe.csv, or s2-strafe-2.csv when that file is already there.
time(#{ fixed_dt: 1.0 / 60.0 });
knob("sun", "freeze");
knob("boxes", "move");
knob("lamps", "move");
knob("dynamic", 50);
jump_checkpoint(#{ name: "hall" });
wait(8);
gi_metrics(#{ reset: true });
let dx = 0.09375073776652446;
let dz = -0.03479653960140816;
let path = [];
for i in 0..181 {
    let k = i.to_float();
    path.push([14.5 + k * dx, 1.7, 23.5 + k * dz, -0.3554, 0.08]);
}
let frames = path.len - 1;
camera(#{ path: path, frames: frames });
wait(frames);
wait(2);
let metrics = gi_metrics(#{ write: "s2-strafe" });
let budgets = get_budgets(#{});
check_lt(metrics.summary.lighting_ms.p99, budgets.lighting_cap_ms + 0.0001, "s2 lighting p99 stays under the preset cap");
