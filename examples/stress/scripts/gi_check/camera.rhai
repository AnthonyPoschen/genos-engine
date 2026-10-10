// gi-check (docs/systems/debugging.md): GI while the camera moves. Lamps and boxes
// still, sun frozen: only the camera moves, so a settled picture at each pose is
// what the moving picture should show. Pass 1 walks the corner path at 6 m/s (one
// pose a frame, turning up to 6 degrees a frame) and then turns on the spot (3
// degrees a frame), saving every frame of the bounce view to move/. Pass 2 stands
// at every STRIDE-th pose until the GI settles and saves that to still/. gi-check
// scores each moving frame against the still one (error, worst tile) and the
// frame-to-frame change beyond the still pictures' own change (flicker).
// Filled in: __DIR__, __GI__, __STRIDE__, __CLIP__ (only the first CLIP poses; 0: all).
time(#{ fixed_dt: 1.0 / 60.0 });
knob("time_of_day", 12.0);
knob("sun", "freeze");
knob("boxes", "still");
knob("dynamic", 0.0);
knob("gi", __GI__);
const STRIDE = __STRIDE__;
let pitch = -0.06;
let path = [];
let x = 2.0;
while x < 9.3 { path.push([x, 1.6, 9.3, 1.5708, pitch]); x += 0.1; }
for i in 0..15 {
    let a = (i + 1) * 0.10472;
    path.push([9.3, 1.6, 9.3 + i * 0.05, 1.5708 + a, pitch]);
}
let z = 10.05;
// Stop 2 m short of the hall's low partition (z 16.9-17.1, top at the 1.6 m eye
// height): walking on to z 18 put the eye through its top edge (frames 155-159).
while z < 15.0 { path.push([9.3, 1.6, z, 3.14159, pitch]); z += 0.1; }
for i in 0..40 { path.push([9.3, 1.6, 15.0, 3.14159 + (i + 1) * 0.05236, pitch]); }
const CLIP = __CLIP__;
if CLIP > 0 && CLIP < path.len() { path.truncate(CLIP); }
let n = path.len();
let p0 = path[0];
camera(#{ position: [p0[0], p0[1], p0[2]], yaw: p0[3], pitch: p0[4] });
wait_settled(#{ timeout_frames: 900 });
view("bounce");
camera(#{ path: path, frames: n });
let a = flicker(#{ frames: n, threshold: 0.03, frames_dir: "__DIR__/move" });
const STOP = 60;
let b = flicker(#{ frames: STOP, threshold: 0.03, frames_dir: "__DIR__/stop" });
let i = 0;
while i < n {
    let p = path[i];
    camera(#{ position: [p[0], p[1], p[2]], yaw: p[3], pitch: p[4] });
    try { wait_settled(#{ timeout_frames: 600 }); } catch (e) { log(`settle ${i}: ${e}`); }
    capture(#{ path: `__DIR__/still/frame-${if i < 10 { "000" } else if i < 100 { "00" } else { "0" }}${i}.png`, width: 320, height: 180 });
    if i == n - 1 { break; }
    i = if i + STRIDE > n - 1 { n - 1 } else { i + STRIDE };
}
log(`CAMERA n ${n} stride ${STRIDE} moving max_delta ${a.max_delta} pops ${a.pops}`);
