// gi-check (docs/systems/debugging.md): the trail a moving box leaves. Camera,
// lamps and the other boxes still. Moving pass: settle with the box at A, then it
// slides to B over MOVE frames (about 1.8 m/s) and stays; every frame of the
// bounce view is saved to move/ for MOVE + HOLD frames, one pose a frame at a
// fixed 60 Hz step with no pause between frames. Reference pass: the box stands at
// every STRIDE-th of those poses until the GI settles; the bounce view goes to
// still/ and the depth view to depth/. gi-check scores each moving frame against
// the settled one inside the space the box left in the last 10 frames.
// Filled in: __DIR__, __GI__, __STRIDE__.
time(#{ fixed_dt: 1.0 / 60.0 });
knob("time_of_day", 12.0);
knob("sun", "freeze");
knob("boxes", "still");
knob("dynamic", 0.0);
knob("gi", __GI__);
const MOVE = 40;
const HOLD = 30;
const STRIDE = __STRIDE__;
let target = [5.0, 0.3, 3.6];
camera(#{ position: [7.0, 1.4, 6.8], look_at: target });
// The box nearest the view's target.
let best = -1;
let best_d = 1.0e9;
let at = [0.0, 0.0, 0.0];
for o in objects() {
    let p = o.position;
    let d = (p[0] - target[0]) * (p[0] - target[0]) + (p[2] - target[2]) * (p[2] - target[2]);
    if o.size < 1.5 && d < best_d { best = o.index; best_d = d; at = p; }
}
let a = [at[0], at[1], at[2]];
let b = [at[0] + 1.1, at[1], at[2] - 0.45];
object(#{ index: best, position: a });
for k in 0..8 { try { wait_settled(#{ timeout_frames: 900 }); break; } catch (e) { log(`settle ${k}: ${e}`); } }
view("bounce");
object(#{ index: best, path: [a, b], frames: MOVE });
let f = flicker(#{ frames: MOVE + HOLD, threshold: 0.03, frames_dir: "__DIR__/move" });
let n = MOVE + HOLD;
let i = 0;
while i < n {
    let t = if i >= MOVE { 1.0 } else { i.to_float() / MOVE.to_float() };
    object(#{ index: best, position: [a[0] + (b[0] - a[0]) * t, a[1], a[2] + (b[2] - a[2]) * t] });
    try { wait_settled(#{ timeout_frames: 600 }); } catch (e) { log(`settle ${i}: ${e}`); }
    let name = `frame-${if i < 10 { "000" } else if i < 100 { "00" } else { "0" }}${i}.png`;
    capture(#{ path: `__DIR__/still/${name}`, width: 320, height: 180 });
    capture(#{ path: `__DIR__/depth/${name}`, mode: "depth", width: 320, height: 180 });
    i += STRIDE;
}
log(`TRAIL box ${best} from ${a} to ${b} over ${MOVE} frames, held ${HOLD}`);
