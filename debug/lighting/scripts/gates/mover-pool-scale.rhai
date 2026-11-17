// Object-attached mover pool scaling. Reports pool use while many boxes move.
// Owner PC: tools/lighting/headless.sh (or a windowed debug/lighting run) with
//   --script debug/lighting/scripts/gates/mover-pool-scale.rhai --gi-metrics
// Counts: 10 / 100 use knob dynamic; 1000 needs a denser layout than scale big.
// Spin gate (limit 0.027) stays examples/stress gi-check spin — not this script.
time(#{ fixed_dt: 1.0 / 60.0 });
knob("sun", "freeze");
knob("scale", "big");
knob("lights", 8);
knob("dynamic", 100);
knob("boxes", "move");
jump_checkpoint(#{ name: "hall" });
log("mover-pool-scale: scale big dynamic 100 (≈48 movers). Watch pool/texel debug.");
wait(8);
gi_metrics(#{ reset: true });
wait(120);
wait(2);
gi_metrics(#{ write: "mover-pool-scale" });
