// gi-check (docs/systems/debugging.md): running up to a lit wall (Anthony,
// 2026-10-12: "as I get closer to high-quality lit walls it goes to coarse grain,
// then if I stop it eventually goes to high quality"). The hall, evening, lamps on;
// clock fixed, sun frozen, lamps and boxes still, so only the camera moves. The run
// crosses the surface cache's texel bands (25 cm past 8 m, 12.5 cm to 8 m, 6.25 cm
// inside 2.5 m) on the far wall and the floor.
// 1. fresh.png: the end pose settled first (bounce view), what the soaked end must
//    match; then 2000 frames at the start (texels only the end pose read expire,
//    GI2_CACHE_LIFE 1800) and settle: baseline.png.
// 2. Run 9 m at 5 m/s (8.33 cm a frame, 108 frames) toward the wall, every frame
//    saved (move/frame-i is at path pose i + 1); hold 120 frames (hold/); settle
//    (end.png).
// 3. Poses of the run settled (still/frame-k at path pose k + 1): the first 5
//    (onset) and every 6th.
// gi-check approach_rows: grain of the moving frames against the settled pose
// (metrics::blotch at 2 px), frames until the held picture is back to settled, and
// the soaked end against fresh.png. Filled in: __DIR__.
time(#{ fixed_dt: 1.0 / 60.0 });
knob("time_of_day", 20.0);
knob("sun", "freeze");
knob("boxes", "still");
knob("dynamic", 0.0);
let P = [11.59, 1.7, 23.52];
let y0 = -1.0;
let pitch = -0.25;
let N = 108;
let step = 9.0 / N.to_float();
fn settle() {
    for k in 0..4 { try { wait_settled(#{ timeout_frames: 900 }); break; } catch (e) { log(`settle ${k}: ${e}`); } }
}
let path = [];
for i in 0..(N + 1) {
    let d = i.to_float() * step;
    path.push([P[0] + d * y0.sin(), P[1], P[2] - d * y0.cos(), y0, pitch]);
}
let e = path[N];
view("full");
camera(#{ position: [e[0], e[1], e[2]], yaw: y0, pitch: pitch });
settle();
view("bounce");
capture(#{ path: "__DIR__/fresh.png" });
view("full");
camera(#{ position: P, yaw: y0, pitch: pitch });
wait(#{ frames: 2000 });
settle();
view("bounce");
capture(#{ path: "__DIR__/baseline.png" });
camera(#{ path: path, frames: N });
flicker(#{ frames: N, threshold: 0.03, frames_dir: "__DIR__/move" });
flicker(#{ frames: 120, threshold: 0.03, frames_dir: "__DIR__/hold" });
settle();
capture(#{ path: "__DIR__/end.png" });
fn pad(i) { if i < 10 { `000${i}` } else if i < 100 { `00${i}` } else { `0${i}` } }
let k = 0;
while k < N {
    if k < 5 || k % 6 == 5 {
        let p = path[k + 1];
        view("full");
        camera(#{ position: [p[0], p[1], p[2]], yaw: p[3], pitch: p[4] });
        settle();
        view("bounce");
        capture(#{ path: `__DIR__/still/frame-${pad(k)}.png` });
    }
    k += 1;
}
