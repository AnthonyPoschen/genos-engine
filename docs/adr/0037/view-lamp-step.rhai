// ADR 0037 round 15 viewing test: hall, every point lamp steps
// full on -> off -> on -> x0.5 -> x1, 3 s each, repeated 5 times. Watch the near box.
time(#{ fixed_dt: 1.0 / 60.0 });
knob("boxes", "still"); knob("layout", "first"); knob("lights", 32); knob("dynamic", 25);
knob("lamps", "still"); knob("time_of_day", 22.0); knob("sun", "freeze");
fn points() { lights().filter(|l| !l.directional) }
fn set(on, k) { for l in points() { light(#{ index: l.index, on: on, intensity: k }); } }
set(true, 1.0);
let boxes = objects().filter(|o| o.height <= 1.2 && o.size <= 1.5);
let b = boxes[0]; let best = 1e9;
for o in boxes { let dx = o.position[0] - 12.5; let dz = o.position[2] - 20.0; if dx*dx+dz*dz < best { best = dx*dx+dz*dz; b = o; } }
let p = b.position;
camera(#{ position: [p[0], p[1] + 1.6, p[2] + 3.0], look_at: [p[0], p[1] + 0.4, p[2]] });
wait(600);
for r in 0..5 {
    log(`view: off`); set(false, 1.0); wait(180);
    log(`view: on`); set(true, 1.0); wait(180);
    log(`view: x0.5`); set(true, 0.5); wait(180);
    log(`view: x1`); set(true, 1.0); wait(180);
}
log("view: done");
