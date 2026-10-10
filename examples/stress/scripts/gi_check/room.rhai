// gi-check (docs/systems/debugging.md): Anthony's make-run spot in the default room
// (genos-stress --scene, scenes/room.rhai: one lamp, nothing moves). Soak at the
// pose (baseline.png), turn on the spot at 3 deg/frame to -16 deg, to -57 deg and
// back (move/), hold 30 (hold/), soak (end.png); then each moving frame's pose
// settled (still/frame-k). Last three settled poses against CPU path-traced
// references (room-a at the start pose, room-b and room-c at -36 and -39 deg):
// the still-frame blotch, and the floor of the world swim (gi-check room_rows).
// Then -36 deg settled, and settled again half a probe (0.23 deg) further round
// (grid-0, grid-1): how much of the picture is redrawn when only the probe grid
// slides.
// Last, a close-up (close-0): 1.5 m on from the start, toward the bounce-lit
// corridor wall and its foot on the floor, against a clean reference: whether a
// near surface keeps its definition (contact, corner gradients) or is blurred.
// Filled in: __DIR__, __GI__, __SECS__.
time(#{ fixed_dt: 1.0 / 60.0 });
knob("gi", __GI__);
let P = [0.664, 1.699, 9.629];
let y0 = -0.679;
let pitch = -0.12;
let d2r = 0.0174533;
let STRIDE = 1;
fn settle() { for k in 0..4 { try { wait_settled(#{ timeout_frames: 900 }); break; } catch (e) { log(`settle ${k}: ${e}`); } } }
fn pad(i) { if i < 10 { `000${i}` } else if i < 100 { `00${i}` } else { `0${i}` } }
// yaw waypoints in degrees
let a0 = y0 / d2r;
let marks = [a0, -16.0, -57.0, a0];
let path = [[P[0], P[1], P[2], y0, pitch]];
for m in 1..marks.len() {
    let from = marks[m - 1];
    let to = marks[m];
    let n = ((to - from).abs() / 3.0).ceiling().to_int();
    for i in 1..(n + 1) {
        let a = from + (to - from) * i.to_float() / n.to_float();
        path.push([P[0], P[1], P[2], a * d2r, pitch]);
    }
}
let N = path.len() - 1;
log(`room turn frames ${N}`);
view("full");
camera(#{ position: P, yaw: y0, pitch: pitch });
settle();
view("full");
capture(#{ path: "__DIR__/baseline.png" });
compare(#{ prefix: "room-a", width: 320, spp: 64, noise: 0.01, max_spp: 4194304, seconds: __SECS__, frames: 2 });
camera(#{ path: path, frames: N });
flicker(#{ frames: N, threshold: 0.03, frames_dir: "__DIR__/move" });
camera(#{ position: P, yaw: y0, pitch: pitch });
flicker(#{ frames: 30, threshold: 0.03, frames_dir: "__DIR__/hold" });
settle();
capture(#{ path: "__DIR__/end.png" });
let k = 0;
while k < N {
    let p = path[k + 1];
    camera(#{ position: [p[0], p[1], p[2]], yaw: p[3], pitch: p[4] });
    settle();
    capture(#{ path: `__DIR__/still/frame-${pad(k)}.png` });
    k += STRIDE;
}
for r in [["room-b", -36.0], ["room-c", -39.0]] {
    camera(#{ position: P, yaw: r[1] * d2r, pitch: pitch });
    settle();
    compare(#{ prefix: r[0], width: 320, spp: 64, noise: 0.01, max_spp: 4194304, seconds: __SECS__, frames: 2 });
}
camera(#{ position: P, yaw: -36.0 * d2r, pitch: pitch });
settle();
capture(#{ path: "__DIR__/grid-0.png" });
camera(#{ position: P, yaw: -36.23 * d2r, pitch: pitch });
settle();
capture(#{ path: "__DIR__/grid-1.png" });
// Close-up definition: the corridor wall and its foot on the floor, near.
camera(#{ position: [-0.22, 1.2, 8.42], yaw: -36.0 * d2r, pitch: -0.3 });
settle();
compare(#{ prefix: "close-0", width: 320, spp: 64, noise: 0.01, max_spp: 4194304, seconds: __SECS__, frames: 2 });
