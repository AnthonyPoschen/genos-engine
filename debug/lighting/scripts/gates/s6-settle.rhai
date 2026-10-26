// S6 settle. Boxes still, lamps still, no moving lamps, sun frozen.
// Hold 600 frames at the hall. Pass --gi-metrics.
// CSV: <out>/gi/s6-settle.csv
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
gi_metrics(#{ write: "s6-settle" });
