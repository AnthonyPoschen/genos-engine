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
let metrics = gi_metrics(#{ write: "s6-settle" });
let budgets = get_budgets(#{});
check_lt(metrics.summary.lighting_ms.p99, budgets.lighting_cap_ms + 0.0001, "s6 lighting p99 stays under the preset cap");
