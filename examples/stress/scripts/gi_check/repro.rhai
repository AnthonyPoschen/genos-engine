// gi-check (docs/systems/debugging.md): the moving-camera brightness repro. Lamps
// and boxes still, sun frozen. Settle at pose P and save the bounce view; then
// three short clips that each end on P, every frame saved: a wiggle of 1e-5 rad
// (nothing moves on screen, but each frame's camera differs), a 1.2 m slide and a
// 20 degree turn. With nothing to see change, the last frame of each must match the
// settled picture: a gather that mixes strata badly draws it brighter (2026-10-10:
// 1.05, 1.22, 1.31 on the gi2_frame hall). Filled in: __DIR__, __GI__.
time(#{ fixed_dt: 1.0 / 60.0 });
knob("time_of_day", 12.0);
knob("sun", "freeze");
knob("boxes", "still");
knob("dynamic", 0.0);
knob("gi", __GI__);
let p = [7.0, 1.4, 6.8];
let yaw = -0.558;
let pitch = -0.284;
const N = 24;
fn settle() {
    for k in 0..8 { try { wait_settled(#{ timeout_frames: 900 }); break; } catch (e) { log(`settle ${k}: ${e}`); } }
}
camera(#{ position: p, yaw: yaw, pitch: pitch });
settle();
view("bounce");
capture(#{ path: "__DIR__/still.png", width: 320, height: 180 });
for clip in ["jitter", "slide", "turn"] {
    let path = [];
    for i in 0..N {
        let k = (N - 1 - i).to_float();
        let s = if i % 2 == 0 { 1.0 } else { -1.0 };
        let q = if clip == "jitter" { [p[0], p[1], p[2], yaw + s * 0.00001] }
            else if clip == "slide" { [p[0] - 0.05 * k, p[1], p[2], yaw] }
            else { [p[0], p[1], p[2], yaw - 0.015 * k] };
        path.push([q[0], q[1], q[2], q[3], pitch]);
    }
    // The last pose is P itself.
    path[N - 1] = [p[0], p[1], p[2], yaw, pitch];
    camera(#{ path: path, frames: N });
    flicker(#{ frames: N, threshold: 0.03, frames_dir: `__DIR__/${clip}` });
    camera(#{ position: p, yaw: yaw, pitch: pitch });
    settle();
}
log("REPRO done");
