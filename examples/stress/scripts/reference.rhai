// Ground truth: the live picture against a CPU path trace of the same frozen scene
// from the same pose (unlimited bounces, no probes). This is how lighting quality is
// judged. Look at the PNGs as well as the numbers: <name>-reference.png,
// <name>-live.png and <name>-heatmap.png (red: live too bright, blue: too dark).
// The first run traces for minutes; later runs reuse target/reference-cache.
time(#{ pause: true });
knob("sun", "freeze");
camera(#{ position: [12.5, 1.7, 20.0], look_at: [12.5, 1.0, 8.0] });

fn judge(name, hours) {
    knob("time_of_day", hours);
    let s = wait_settled(#{ timeout_frames: 900 });
    log(`${name}: settled ${s.settled} after ${s.frames} frames`);
    let c = compare(#{ prefix: name, width: 160, spp: 64, noise: 0.03, seconds: 600, frames: 30 });
    log(`${name}: mean_rel ${c.mean_rel} p95_rel ${c.p95_rel} bias ${c.bias} | reference ${c.spp} spp, noise ${c.noise}, ${c.trace_seconds} s`);
    check_lt(c.noise, 0.05, `${name}: the reference is converged enough to judge by`);
    c
}

// Bounds sit just above today's error; tighten them as the lighting improves.
let noon = judge("noon", 12.0);
check_lt(noon.mean_rel, 0.10, "noon: under 10% of the light is wrong");
check_lt(noon.p95_rel, 0.30, "noon: 95% of the picture within 30%");
check_lt(noon.bias.abs(), 0.10, "noon: exposure within 10%");

// The engine's direct light against a direct-only trace: checks the reference itself.
let direct = compare(#{ prefix: "noon-direct", mode: "direct", bounces: 0, spp: 4, max_spp: 4, width: 320, block: 1 });
check_lt(direct.mean_rel, 0.05, "direct light matches the direct-only trace");

let night = judge("night", 22.0);
log(`night regions: ${night.regions}`);
check_lt(night.mean_rel, 0.5, "night: under half of the light is wrong");
