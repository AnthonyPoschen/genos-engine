// gi-check (docs/systems/debugging.md): time to fill. How fast the picture gets to
// its settled light after a reveal and after a light change, camera and scene
// still (Anthony 2026-10-12: pop-in is fine if the screen fills near-immediately).
// The turn pose settled; then the camera cuts to the other way (surfaces never
// seen) and holds for 120 frames (reveal/), then settles (reveal_end/); then the
// sun moves (12:00 -> 15:00, frozen): 120 frames (light/), settled (light_end/).
// Filled in: __DIR__.
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
flicker(#{ frames: 1, threshold: 1.0, frames_dir: "__DIR__/reveal_end" });
knob("time_of_day", 15.0);
flicker(#{ frames: 120, threshold: 1.0, frames_dir: "__DIR__/light" });
settle();
flicker(#{ frames: 1, threshold: 1.0, frames_dir: "__DIR__/light_end" });
