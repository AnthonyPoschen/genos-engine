// gi-check (docs/systems/debugging.md): moving lamps and boxes, camera still and
// turning (mix), and the spinning green box (spin). Each segment: soak at the pose
// with every clock at 0 (<seg>/baseline.png), one motion (<seg>/move), everything
// back to its start (clocks 0, the pose) and held 30 frames (<seg>/hold), soaked
// (<seg>/end.png); then the scene state and pose of each moving frame, settled
// (<seg>/still/frame-k, every `stride` frames). Steps of 1/40 s, Anthony's ~40 fps.
// Filled in: __DIR__, __GI__, __SEGS__ ("mix" or "spin").
let dt = 1.0 / 40.0;
time(#{ fixed_dt: dt });
knob("time_of_day", 20.0);
knob("sun", "freeze");
knob("dynamic", 50.0);
knob("gi", __GI__);
fn settle() {
    for k in 0..4 { try { wait_settled(#{ timeout_frames: 900 }); break; } catch (e) { log(`settle ${k}: ${e}`); } }
}
fn pad(i) { if i < 10 { `000${i}` } else if i < 100 { `00${i}` } else { `0${i}` } }
fn still_all() {
    knob("boxes", "still");
    knob("lamps", "still");
    knob("lamp_clock", 0.0);
    knob("box_clock", 0.0);
}
fn segment(dir, lamps, boxes, turn, p0, y0, pitch, n, stride, dt) {
    let d2r = 0.0174533;
    let path = [];
    let h = n / 2;
    for i in 0..(n + 1) {
        let a = if turn { if i <= h { i } else { n - i } } else { 0 };
        path.push([p0[0], p0[1], p0[2], y0 + a * 3.0 * d2r, pitch]);
    }
    still_all();
    view("full");
    camera(#{ position: p0, yaw: y0, pitch: pitch });
    settle();
    capture(#{ path: `${dir}/baseline.png` });
    if lamps { knob("lamps", "move"); }
    if boxes { knob("boxes", "move"); }
    if turn { camera(#{ path: path, frames: n }); }
    flicker(#{ frames: n, threshold: 0.03, frames_dir: `${dir}/move` });
    still_all();
    camera(#{ position: p0, yaw: y0, pitch: pitch });
    flicker(#{ frames: 30, threshold: 0.03, frames_dir: `${dir}/hold` });
    settle();
    capture(#{ path: `${dir}/end.png` });
    let k = 0;
    while k < n {
        let c = (k + 1) * dt;
        if lamps { knob("lamp_clock", c); }
        if boxes { knob("box_clock", c); }
        let p = path[if turn { k + 1 } else { 0 }];
        camera(#{ position: [p[0], p[1], p[2]], yaw: p[3], pitch: p[4] });
        settle();
        capture(#{ path: `${dir}/still/frame-${pad(k)}.png` });
        k += stride;
    }
    still_all();
}
let segs = "__SEGS__";
if segs == "mix" {
    // Anthony's hall: lamps and boxes move with the camera still (the control, at
    // the middle of the turn), then the same while turning 3 deg a frame out 45 deg
    // and back.
    segment("__DIR__/still", true, true, false, [11.59, 1.7, 23.52], -0.215, -0.166, 30, 2, dt);
    segment("__DIR__/turn", true, true, true, [11.59, 1.7, 23.52], -1.0, -0.166, 30, 2, dt);
} else {
    // The spinning green box (orbit and spin) from a still camera, every frame.
    segment("__DIR__/green", false, true, false, [13.0, 1.6, 16.5], 0.0, -0.3, 24, 1, dt);
}
