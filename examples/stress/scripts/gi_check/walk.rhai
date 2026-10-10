// gi-check (docs/systems/debugging.md): the corner walk. The camera path plays one
// pose per frame (0.1 m, up to 6 degrees a frame = 6 m/s) and every frame of the
// bounce view is saved, so no script readback gives the caches extra frames. Cold
// pass, then the same path warm; lamps and boxes still, sun frozen: only the camera
// moves. gi-check scores the cold frames against the warm ones (the pop). __DIR__ is
// the output folder.
time(#{ fixed_dt: 1.0 / 60.0 });
knob("time_of_day", 12.0);
knob("sun", "freeze");
knob("boxes", "still");
knob("dynamic", 0.0);
knob("gi", __GI__);
let pitch = -0.06;
let path = [];
let x = 2.0;
while x < 9.3 { path.push([x, 1.6, 9.3, 1.5708, pitch]); x += 0.1; }
for i in 0..15 {
    let a = (i + 1) * 0.10472;
    // direction (cos a, sin a) in x,z -> yaw with dir = (sin yaw, -cos yaw)
    let yaw = 1.5708 + a;
    path.push([9.3, 1.6, 9.3 + i * 0.05, yaw, pitch]);
}
let z = 10.05;
// Stop 2 m short of the hall's low partition (z 16.9-17.1, top at the 1.6 m eye
// height): walking on to z 18 put the eye through its top edge (frames 155-159).
while z < 15.0 { path.push([9.3, 1.6, z, 3.14159, pitch]); z += 0.1; }
let n = path.len();
camera(#{ position: [2.0, 1.6, 9.3], yaw: 1.5708, pitch: pitch });
wait_settled(#{ timeout_frames: 900 });
view("bounce");
camera(#{ path: path, frames: n });
let a = flicker(#{ frames: n, threshold: 0.03, region: [0.2, 0.2, 0.6, 0.6], frames_dir: "__DIR__/cold" });
camera(#{ position: [2.0, 1.6, 9.3], yaw: 1.5708, pitch: pitch });
wait(#{ frames: 30 });
camera(#{ path: path, frames: n });
let b = flicker(#{ frames: n, threshold: 0.03, region: [0.2, 0.2, 0.6, 0.6], frames_dir: "__DIR__/warm" });
log(`CORNER2 n ${n} cold max_delta ${a.max_delta} pops ${a.pops} | warm max_delta ${b.max_delta} pops ${b.pops}`);
