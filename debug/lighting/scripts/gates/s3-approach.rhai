// S3 approach. Lit wall at x=12.5, z=8 (lamp-wall look target).
// Start 8 m out (z=16), walk to 0.3 m (z=8.3) at 2 m/s, then back.
// 7.7 m is 231 frames at 1/60. Yaw 0 looks down -Z. Pitch aims at z=8, y=1.
// Pass --gi-metrics. CSV: <out>/gi/s3-approach.csv, or s3-approach-2.csv when that file is already there.
time(#{ fixed_dt: 1.0 / 60.0 });
knob("sun", "freeze");
knob("boxes", "move");
knob("lamps", "move");
knob("dynamic", 50);
let pitch0 = -0.08727771294946145;
let pitch1 = -1.1659045405098132;
let leg = 231;
let path = [];
for i in 0..(leg + 1) {
    let t = i.to_float() / leg.to_float();
    path.push([12.5, 1.7, 16.0 + (8.3 - 16.0) * t, 0.0, pitch0 + (pitch1 - pitch0) * t]);
}
for i in 0..(leg + 1) {
    let t = i.to_float() / leg.to_float();
    path.push([12.5, 1.7, 8.3 + (16.0 - 8.3) * t, 0.0, pitch1 + (pitch0 - pitch1) * t]);
}
wait(8);
gi_metrics(#{ reset: true });
let frames = path.len - 1;
camera(#{ path: path, frames: frames });
wait(frames);
wait(2);
let metrics = gi_metrics(#{ write: "s3-approach" });
let budgets = get_budgets(#{});
check_lt(metrics.summary.lighting_ms.p99, budgets.lighting_cap_ms + 0.0001, "s3 lighting p99 stays under the preset cap");
