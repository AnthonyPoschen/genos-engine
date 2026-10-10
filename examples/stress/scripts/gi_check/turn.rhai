// gi-check (docs/systems/debugging.md): the camera-turn check. Anthony's pose in
// the hall (evening, lamps on), bounce view; clock fixed, sun frozen, lamps and
// boxes still, so the settled picture at the pose is what any frame there should
// show. First the pose settled from a fresh start: ref.png. Then each segment
// settles at its start, moves for 15 frames to end exactly at the pose (turning at
// 3 degrees a frame, Anthony's real rate, or walking 5 cm a frame) and holds still
// for 30 frames; every frame is saved (<seg>/move,
// <seg>/hold), then holds until settled again (<seg>/end.png: every movement
// test ends on a soaked still frame). gi-check scores the arrival frame and the last held frame against
// ref.png (turn_rows). Last the soak / move / return segment (debugging.md, moving
// tests): from the settled pose (ref.png is its baseline) turn out 15 frames at 3
// degrees a frame and straight back, hold 30 (back/move, back/hold); then the
// first 5 poses of that turn, each settled (onset/frame-000i), for the onset pop.
// Filled in: __DIR__, __GI__.
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
for seg in [["turn", 3.0], ["walk", 0.0]] {
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
    settle();
    capture(#{ path: `__DIR__/${name}/end.png` });
}
let back = [];
for i in 0..31 {
    let a = if i <= 15 { i } else { 30 - i };
    back.push([P[0], P[1], P[2], y0 + a * 3.0 * d2r, pitch]);
}
view("full");
camera(#{ position: P, yaw: y0, pitch: pitch });
settle();
view("bounce");
camera(#{ path: back, frames: 30 });
flicker(#{ frames: 31, threshold: 0.03, frames_dir: "__DIR__/back/move" });
flicker(#{ frames: 30, threshold: 0.03, frames_dir: "__DIR__/back/hold" });
settle();
capture(#{ path: "__DIR__/back/end.png" });
for i in 1..6 {
    let p = back[i];
    camera(#{ position: [p[0], p[1], p[2]], yaw: p[3], pitch: p[4] });
    settle();
    capture(#{ path: `__DIR__/onset/frame-000${i}.png` });
}
