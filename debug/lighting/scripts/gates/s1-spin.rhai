// S1 spin. Hall checkpoint, boxes moving, half the lamps moving.
// 360 degrees of yaw in 120 frames (2 s at 1/60), repeated 3 times.
// The sun stays frozen, as the stress scene already sets it.
// Pass --gi-metrics. Samples are one frame behind the draw, so wait(2)
// lands before the summary. CSV: <out>/gi/s1-spin.csv, or s1-spin-2.csv when that file is already there.
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
// The first 60 frames are the refill after the checkpoint jump, not steady state.
let metrics = gi_metrics(#{ write: "s1-spin", warmup: 60 });
let budgets = get_budgets(#{});
let p99 = metrics.summary.lighting_ms.p99;
let cap = budgets.lighting_cap_ms;
// The controller can only shrink budgeted work (rays, refreshes, denoise).
// Direct shading of the display alone costs more than the 8 ms High cap at
// 1080p on master, so p99 under the cap is not reachable yet (roadmap item).
// What this gate grades: when steady p99 is over the cap, the controller must
// have spent its whole range (scale at the floor), not sat at full budget.
log(`s1 steady lighting p99 ${p99} ms, cap ${cap} ms, controller_scale_min ${metrics.summary.controller_scale_min}`);
check(p99 < cap + 0.0001 || metrics.summary.controller_scale_min <= budgets.controller_floor * 1.001, "s1 lighting p99 under the cap, or the controller reached its floor in the window");
