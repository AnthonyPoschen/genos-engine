// WRC drain, by eye (ADR 0037, tests/wrc_lamps_off.rs): the lamps-off night
// lit, then lamps fully off, back on, half power, double power, ~3 s each in
// wall time, so the old bounce light leaving can be judged on screen.
// Run (Anthony's PC, not CI):
//   cargo run --release -p genos-stress -- --script examples/stress/scripts/wrc_drain_view.rhai
// Measured on lavapipe (wrc_lamps_off): off 1 % of lit in 1 frame, moved away
// 42 frames (gate 12 = 200 ms at 60 fps); see the PR for x0.5 and x2.
let STEP = 180;
time(#{ fixed_dt: 0.0 });
knob("time_of_day", 22.0);
knob("sun", "freeze");
knob("boxes", "still");
knob("lamps", "still");
knob("dynamic", 0.0);
knob("lights", 32);
knob("layout", "spread");
knob("power", 1.0);
camera(#{ position: [14.5, 1.7, 23.5], yaw: -0.3554, pitch: 0.08 });
wait(#{ frames: 300 });
fn step(label, power, frames) {
    knob("power", power);
    log(`step ${label}: power ${power}`);
    wait(#{ frames: 12 });
    log(`${label} +12 frames: ${luminance(#{ region: [0.0, 0.0, 1.0, 1.0] })}`);
    wait(#{ frames: frames - 12 });
    log(`${label} settled: ${luminance(#{ region: [0.0, 0.0, 1.0, 1.0] })}`);
    shot(label);
}
shot("lit");
step("off", 0.0, STEP);
step("on", 1.0, STEP);
step("half", 0.5, STEP);
step("double", 2.0, STEP);
