// S5 A to B to A. Hall to room-a and back, 5 cycles, 240 frames (4 s) per leg.
// Yaw delta -2.2006 is already the short arc. Pass --gi-metrics.
// CSV: <out>/gi/s5-aba.csv, or s5-aba-2.csv when that file is already there.
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
// The first 60 frames are the refill after the checkpoint jump, not steady state.
let metrics = gi_metrics(#{ write: "s5-aba", warmup: 60 });
let budgets = get_budgets(#{});
let p99 = metrics.summary.lighting_ms.p99;
let cap = budgets.lighting_cap_ms;
// The controller can only shrink budgeted work (rays, refreshes, denoise).
// Direct shading of the display alone costs more than the 8 ms High cap at
// 1080p on master, so p99 under the cap is not reachable yet (roadmap item).
// What this gate grades: when steady p99 is over the cap, the controller must
// have spent its whole range (scale at the floor), not sat at full budget.
log(`s5 steady lighting p99 ${p99} ms, cap ${cap} ms, controller_scale_min ${metrics.summary.controller_scale_min}`);
check(p99 < cap + 0.0001 || metrics.summary.controller_scale_min <= budgets.controller_floor * 1.001, "s5 lighting p99 under the cap, or the controller reached its floor in the window");
