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
// The first 60 frames are the refill after the checkpoint jump, not steady state.
let metrics = gi_metrics(#{ write: "s2-strafe", warmup: 60 });
let budgets = get_budgets(#{});
let p99 = metrics.summary.lighting_ms.p99;
let cap = budgets.lighting_cap_ms;
// The controller can only shrink budgeted work (rays, refreshes, denoise).
// Direct shading of the display alone costs more than the 8 ms High cap at
// 1080p on master, so p99 under the cap is not reachable yet (roadmap item).
// What this gate grades: when steady p99 is over the cap, the controller must
// have spent its whole range (scale at the floor), not sat at full budget.
log(`s2 steady lighting p99 ${p99} ms, cap ${cap} ms, controller_scale ${budgets.controller_scale}`);
check(p99 < cap + 0.0001 || budgets.controller_scale <= budgets.controller_floor * 1.001, "s2 lighting p99 under the cap, or the controller at its floor");
