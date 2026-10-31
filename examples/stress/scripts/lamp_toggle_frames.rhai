// Dump a lamp toggle for layering-gate --capture.
// Frames 0000-0007 request weight 1, after a 600-frame soak.
// Frames 0008-0120 request weight 0.
// The shell writes weights.txt (this host has no file write).
// expected-off-reference.png and expected-on-reference.png are the
// path traces at those two weights. The grader lerps them.
// Fixed step 1/60 s. Sun frozen, boxes still.
time(#{ fixed_dt: 0.0166667 });
knob("time_of_day", 22.0);
knob("sun", "freeze");
knob("boxes", "still");
knob("dynamic", 0.0);
camera(#{ position: [12.5, 1.7, 20.0], look_at: [12.5, 1.0, 8.0] });
wait(600);
let lamps = lights();
let target = lamps.filter(|l| l.in_view && !l.directional);
require(target.len() > 0, "a lamp is in view");
let lamp = target[0];
fn pad(i) {
    if i < 10 {
        `000${i}`
    } else if i < 100 {
        `00${i}`
    } else {
        `0${i}`
    }
}
let i = 0;
while i < 8 {
    capture(#{ path: `frames/frame-${pad(i)}.png` });
    i += 1;
}
light(#{ index: lamp.index, color: [0.0, 0.0, 0.0] });
let n = 120;
while i <= n {
    capture(#{ path: `frames/frame-${pad(i)}.png` });
    i += 1;
}
let trace = #{
    cache: "off",
    width: 320,
    height: 180,
    spp: 4,
    max_spp: 1024,
    seconds: 600,
    noise: 0.025,
    bounces: 64
};
trace.prefix = "expected-off";
let off_trace = reference(trace);
log(`off trace spp ${off_trace.spp} noise ${off_trace.noise} seconds ${off_trace.trace_seconds}`);
light(#{ index: lamp.index, color: lamp.color });
trace.prefix = "expected-on";
let on_trace = reference(trace);
log(`on trace spp ${on_trace.spp} noise ${on_trace.noise} seconds ${on_trace.trace_seconds}`);
log(`dumped ${n + 1} frames, lamp ${lamp.index} off from frame 8`);
