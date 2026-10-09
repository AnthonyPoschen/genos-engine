// Walk across the outer edge of the ground (the yard ends at z = 33) toward the main
// entrance and back, with the lamps moving inside. Where the camera stands decides which
// bricks the picture uses, and so which update first, but never the light they settle
// on: no frame may step the indirect light in the view of the entrance.
// Both checks still fail: the indirect light steps 2x to 3x in one frame near z = 33.
time(#{ fixed_dt: 1.0 / 60.0 });
knob("sun", "freeze");
knob("time_of_day", 12.0);
knob("boxes", 1.0);
knob("lights", 5.0);
knob("dynamic", 50.0);
let look = [12.0, 1.2, 18.0];
let region = [0.2, 0.2, 0.6, 0.6];
camera(#{ position: [12.0, 1.6, 45.0], look_at: look });
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
// In: 0.1 m a frame (6 m/s) from 45 m to 30 m, then back out.
let inward = [];
let z = 45.0;
while z > 30.0 {
    inward.push(bounce_at(z, look, region));
    z -= 0.1;
}
let outward = [];
while z < 45.0 {
    outward.push(bounce_at(z, look, region));
    z += 0.1;
}
let a = worst_step(inward);
let b = worst_step(outward);
log(`in: bounce ${inward[0]} -> ${inward[inward.len() - 1]}, largest one-frame step ${a}`);
log(`out: bounce ${outward[0]} -> ${outward[outward.len() - 1]}, largest one-frame step ${b}`);
check_lt(a, 0.15, "walking in across the ground edge moves the indirect light smoothly");
check_lt(b, 0.15, "walking out across the ground edge moves the indirect light smoothly");
