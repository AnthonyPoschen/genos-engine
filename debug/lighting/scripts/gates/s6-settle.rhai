// S6 settle. Boxes still, lamps still, no moving lamps, sun frozen.
// Hold 600 frames at the hall. Pass --gi-metrics.
// CSV: <out>/gi/s6-settle.csv, or s6-settle-2.csv when that file is already there.
time(#{ fixed_dt: 1.0 / 60.0 });
knob("sun", "freeze");
knob("boxes", "still");
knob("lamps", "still");
knob("dynamic", 0);
jump_checkpoint(#{ name: "hall" });
wait(8);
gi_metrics(#{ reset: true });
wait(600);
wait(2);
// The first 60 frames are the refill after the checkpoint jump, not steady state.
let metrics = gi_metrics(#{ write: "s6-settle", warmup: 60 });
let budgets = get_budgets(#{});
let p99 = metrics.summary.lighting_ms.p99;
let cap = budgets.lighting_cap_ms;
// The controller can only shrink budgeted work (rays, refreshes, denoise).
// Direct shading of the display alone costs more than the 8 ms High cap at
// 1080p on master, so p99 under the cap is not reachable yet (roadmap item).
// What this gate grades: when steady p99 is over the cap, the controller must
// have spent its whole range (scale at the floor), not sat at full budget.
log(`s6 steady lighting p99 ${p99} ms, cap ${cap} ms, controller_scale_min ${metrics.summary.controller_scale_min}`);
check(p99 < cap + 0.0001 || metrics.summary.controller_scale_min <= budgets.controller_floor * 1.001, "s6 lighting p99 under the cap, or the controller reached its floor in the window");
