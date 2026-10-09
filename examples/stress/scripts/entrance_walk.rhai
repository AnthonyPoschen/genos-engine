// Walk up to the main (south) entrance from 60 m out, then step in and out across
// its threshold. The lamps keep moving inside. What the probes hold must not depend
// on where the camera stands: no frame may move the indirect light in the doorway
// view by more than the lamps' own motion does. The probes used to leave the lamps
// inside out of the bounce while the camera was outside and far: a 6x step at 27 m.
// A 2.4x step at about 32.4 m was the window outgrowing the shader's brick table:
// passes counted on the CPU never ran.
time(#{ fixed_dt: 1.0 / 60.0 });
knob("sun", "freeze");
knob("time_of_day", 12.0);
knob("boxes", 1.0);
knob("lights", 5.0);
knob("dynamic", 50.0);
let x = 12.0;
let look = [x, 1.2, 18.0];
let region = [0.2, 0.2, 0.6, 0.6];
camera(#{ position: [x, 1.6, 85.0], look_at: look });
wait(#{ frames: 240 });
fn bounce_at(z, look, region) {
    camera(#{ position: [12.0, 1.6, z], look_at: look });
    wait(#{ frames: 1 });
    luminance(#{ mode: "bounce", region: region }).luminance
}
fn worst_step(series) {
    let worst = 0.0;
    for i in 1..series.len() {
        let a = series[i - 1];
        let b = series[i];
        let top = if a > b { a } else { b };
        let r = (a - b).abs() / (if top > 1.0e-4 { top } else { 1.0e-4 });
        if r > worst { worst = r; }
    }
    worst
}
// The approach: 0.1 m a frame (6 m/s) from 85 m to the threshold.
let walk = [];
let z = 85.0;
while z > 25.6 {
    walk.push(bounce_at(z, look, region));
    z -= 0.1;
}
// Across the threshold and back, 0.2 m a step, five times.
let steps = [];
for k in 0..5 {
    for z in [25.5, 25.3, 25.1, 24.9, 24.7, 24.5, 24.7, 24.9, 25.1, 25.3] {
        steps.push(bounce_at(z, look, region));
    }
}
let w = worst_step(walk);
let s = worst_step(steps);
log(`approach: ${walk.len()} frames, bounce ${walk[0]} -> ${walk[walk.len() - 1]}, largest one-frame step ${w}`);
log(`threshold: bounce ${steps}, largest one-frame step ${s}`);
check_lt(w, 0.15, "walking up to the entrance moves the indirect light smoothly");
check_lt(s, 0.10, "stepping across the entrance does not switch the indirect light");
