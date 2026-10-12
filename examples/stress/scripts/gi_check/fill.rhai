// gi-check (docs/systems/debugging.md): convergence pop. Light filling in after a
// reveal and after a light change, camera and scene still, must ramp smoothly, not
// step texel by texel as relight sets commit. The turn pose settled; then the
// camera cuts to the other way (surfaces never seen) and holds for 120 frames
// (reveal/); then, settled again, the sun moves (12:00 -> 15:00, frozen) and 120
// frames are saved (light/). gi-check scores, per pixel, the gap between the
// frame and its 9-frame moving average (steps; a ramp scores ~0). Filled in: __DIR__.
time(#{ fixed_dt: 0.0166667 });
knob("time_of_day", 12.0);
knob("sun", "freeze");
knob("boxes", "still");
knob("dynamic", 0.0);
let P = [11.59, 1.7, 23.52];
let y0 = -1.0;
let pitch = -0.166;
fn settle() {
    for k in 0..4 { try { wait_settled(#{ timeout_frames: 900 }); break; } catch (e) { log(`settle ${k}: ${e}`); } }
}
view("full");
camera(#{ position: P, yaw: y0, pitch: pitch });
settle();
camera(#{ position: P, yaw: y0 + 3.14159, pitch: pitch });
flicker(#{ frames: 120, threshold: 1.0, frames_dir: "__DIR__/reveal" });
settle();
knob("time_of_day", 15.0);
flicker(#{ frames: 120, threshold: 1.0, frames_dir: "__DIR__/light" });
