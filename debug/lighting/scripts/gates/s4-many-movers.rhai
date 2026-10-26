// S4 many movers. The design asked for 64 moving boxes. The stress building
// places 3 movers in each section, and scale big is 4x4 sections, so this
// run is 48 moving boxes and 16 moving lamps. There is no 64-box layout.
// Hold the hall for 240 frames. Pass --gi-metrics.
// CSV: <out>/gi/s4-many-movers.csv
time(#{ fixed_dt: 1.0 / 60.0 });
knob("sun", "freeze");
knob("scale", "big");
knob("lights", 16);
knob("dynamic", 100);
knob("boxes", "move");
knob("lamps", "move");
jump_checkpoint(#{ name: "hall" });
log("S4 scale big: 16 sections x 3 movers = 48 boxes, 16 lamps. Design asked for 64 boxes.");
wait(8);
gi_metrics(#{ reset: true });
wait(240);
wait(2);
gi_metrics(#{ write: "s4-many-movers" });
