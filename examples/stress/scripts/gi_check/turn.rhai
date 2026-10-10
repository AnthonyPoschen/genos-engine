// gi-check (docs/systems/debugging.md): the camera-turn check. Anthony's pose in
// the hall (evening, lamps on), bounce view; clock fixed, sun frozen, lamps and
// boxes still, so the settled picture at the pose is what any frame there should
// show. First the pose settled from a fresh start: ref.png. Then each segment
// settles at its start, moves for 15 frames to end exactly at the pose (turning at
// 6 degrees a frame from either side, at 3 a frame, or walking 5 cm a frame as the
// control) and holds still for 30 frames; every frame is saved (<seg>/move,
// <seg>/hold). gi-check scores the arrival frame and the last held frame against
// ref.png (turn_rows). Filled in: __DIR__, __GI__.
time(#{ fixed_dt: 1.0 / 60.0 });
knob("time_of_day", 20.0);
knob("sun", "freeze");
knob("boxes", "still");
knob("dynamic", 0.0);
knob("gi", __GI__);
let P = [11.59, 1.7, 23.52];
let y0 = -1.0;
let pitch = -0.166;
let d2r = 0.0174533;
let K = 15;
fn settle() {
    for k in 0..4 { try { wait_settled(#{ timeout_frames: 900 }); break; } catch (e) { log(`settle ${k}: ${e}`); } }
}
view("full");
camera(#{ position: P, yaw: y0, pitch: pitch });
settle();
view("bounce");
capture(#{ path: "__DIR__/ref.png" });
for seg in [["turn6", 6.0], ["turn6r", -6.0], ["turn3", 3.0], ["walk", 0.0]] {
    let name = seg[0];
    let s = seg[1];
    let path = [];
    for i in 0..(K + 1) {
        let j = K - i;
        if name == "walk" {
            path.push([P[0] - j * 0.05 * y0.sin(), P[1], P[2] + j * 0.05 * y0.cos(), y0, pitch]);
        } else {
            path.push([P[0], P[1], P[2], y0 - j * s * d2r, pitch]);
        }
    }
    let p0 = path[0];
    view("full");
    camera(#{ position: [p0[0], p0[1], p0[2]], yaw: p0[3], pitch: p0[4] });
    settle();
    view("bounce");
    camera(#{ path: path, frames: K });
    flicker(#{ frames: K + 1, threshold: 0.03, frames_dir: `__DIR__/${name}/move` });
    flicker(#{ frames: 30, threshold: 0.03, frames_dir: `__DIR__/${name}/hold` });
}
