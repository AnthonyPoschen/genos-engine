// gi-check (docs/systems/debugging.md): the five GI diagnosis views, each compared
// with a CPU path-traced reference of that exact moment. Filled in by
// `genos-stress gi-check`: __MOVING__, __NOISE__, __MAXSPP__, __SECS__, __ONLY__, __GI__.
//
// Settled: clock paused, the GI settles, then the picture is compared.
// Moving: the stress scene's real motion. Per view: settle with lamps home and
// boxes still, padded to PAD frames so every build reaches the moving window at the
// same scene time; then lamps (50 % dynamic) and boxes move for MOVE frames at a
// fixed 60 Hz step, camera still. The clock pauses on the last moving frame, whose
// picture is captured at once and compared with a reference of that pose.
let moving = __MOVING__;
time(#{ fixed_dt: 0.0166667 });
if !moving { time(#{ pause: true }); }
knob("sun", "freeze");
lighting(#{ tier_ms: 20.0 });
knob("gi", __GI__);
const PAD = 600;
const MOVE = 120;
let views = [
    ["room-a", 12.0, #{ position: [7.2, 1.7, 0.9], yaw: -2.556, pitch: -0.12 }],
    ["corner", 12.0, #{ position: [3.5, 1.6, 3.5], look_at: [0.2, 0.4, 0.2] }],
    ["contact", 12.0, #{ position: [7.0, 1.4, 6.8], look_at: [5.0, 0.3, 3.6] }],
    ["lamp-wall", 22.0, #{ position: [12.5, 1.7, 20.0], look_at: [12.5, 1.0, 8.0] }],
    ["hall", 12.0, #{ position: [14.5, 1.7, 23.5], yaw: -0.3554, pitch: 0.08 }],
];
let only = "__ONLY__";
for v in views {
    let name = v[0];
    if only != "" && !only.contains(name) { continue; }
    knob("time_of_day", v[1]);
    let c = ();
    let s = #{ settled: false, frames: -1, wall_ms: -1 };
    if moving {
        time(#{ pause: false });
        knob("dynamic", 0.0);
        knob("boxes", "still");
        camera(v[2]);
        try { s = wait_settled(#{ timeout_frames: PAD }); } catch (e) { log(`settle: ${e}`); }
        let spent = if type_of(s.frames) == "f64" { s.frames.to_int() } else { s.frames };
        if spent < 0 { spent = PAD; }
        if spent < PAD { wait(PAD - spent); }
        knob("dynamic", 50.0);
        knob("boxes", "move");
        wait(MOVE);
        time(#{ pause: true });
        capture(#{ name: `${name}-moving`, width: 320, height: 180 });
        c = compare(#{ prefix: name, width: 320, spp: 64, noise: __NOISE__, max_spp: __MAXSPP__, seconds: __SECS__, live: `${name}-moving` });
    } else {
        camera(v[2]);
        for k in 0..8 {
            try { s = wait_settled(#{ timeout_frames: 900 }); break; } catch (e) { log(`settle retry ${k}: ${e}`); }
        }
        c = compare(#{ prefix: name, width: 320, spp: 64, noise: __NOISE__, max_spp: __MAXSPP__, seconds: __SECS__, frames: 2 });
    }
    capture(#{ path: `${name}-albedo.png`, mode: "albedo", width: 320, height: 180 });
    capture(#{ path: `${name}-normal.png`, mode: "normal", width: 320, height: 180 });
    capture(#{ path: `${name}-depth.png`, mode: "depth", width: 320, height: 180 });
    capture(#{ path: `${name}-direct.png`, mode: "direct", width: 320, height: 180 });
    log(`RESULT ${name} mean_rel ${c.mean_rel} bias ${c.bias} settled ${s.settled} frames ${s.frames} refspp ${c.spp} refnoise ${c.noise} trace_s ${c.trace_seconds}`);
}
