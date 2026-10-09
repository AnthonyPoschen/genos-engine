// Ground truth at the stress views that stress different light: the hall (sun through
// the skylight), room A (lit by bounce alone, colour bleed), a doorway into room B,
// the building from outside, and room A's dark corner. Look at every
// <view>-reference.png / -live.png / -heatmap.png as well as the numbers.
time(#{ pause: true });
knob("sun", "freeze");
knob("time_of_day", 12.0);
lighting(#{ tier_ms: 20.0 });

let views = [
    ["hall", #{ position: [14.5, 1.7, 23.5], yaw: -0.3554, pitch: 0.08 }],
    ["room-a", #{ position: [7.2, 1.7, 0.9], yaw: -2.556, pitch: -0.12 }],
    ["doorway", #{ position: [12.0, 1.6, 20.5], look_at: [22.0, 1.2, 20.5] }],
    ["outside", #{ position: [12.5, 1.7, 34.0], look_at: [12.5, 1.5, 22.0] }],
    ["corner", #{ position: [3.5, 1.6, 3.5], look_at: [0.2, 0.4, 0.2] }],
];
for v in views {
    let name = v[0];
    camera(v[1]);
    let s = wait_settled(#{ timeout_frames: 900 });
    let c = compare(#{ prefix: name, width: 160, spp: 64, noise: 0.03, seconds: 600, frames: 30 });
    log(`${name}: mean_rel ${c.mean_rel} p95_rel ${c.p95_rel} bias ${c.bias} | settled ${s.settled} | ref ${c.spp} spp noise ${c.noise} ${c.trace_seconds} s`);
    log(`${name} regions: ${c.regions}`);
    check_lt(c.noise, 0.06, `${name}: reference converged enough to judge by`);
}
