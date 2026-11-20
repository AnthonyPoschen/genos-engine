// A moving sun shadow left a ragged, noisy trail behind its edge: the pixel
// area mean held the lit value and let it go at a different frame per pixel.
// Move the sun for one second, freeze it, and require the picture on that
// frame to match the picture settled at the same sun.
let t0 = 17.0;
time(#{ fixed_dt: 0.0166667 });
knob("sun_speed", 0.0);
knob("time_of_day", t0);
knob("lamps", "still");
knob("boxes", "still");
view("direct");
camera(#{ position: [11.91909, 2.04196, 26.98754], yaw: -1.22863, pitch: -0.937 });
wait_settled(#{ timeout_frames: 600 });
shot("start");
knob("sun_speed", 1.0);
wait(60);
knob("sun_speed", 0.0);
capture(#{ name: "moving", path: "sun-moving.png" });
wait_settled(#{ timeout_frames: 600 });
wait(30);
capture(#{ name: "settled", path: "sun-settled.png" });
let d = diff(#{ a: "moving", b: "settled", out: "sun-diff.png", threshold: 10 });
log(`moving vs settled: mean_abs ${d.mean_abs}, over_share ${d.over_share}`);
check_lt(d.over_share, 0.028, "a moving sun shadow leaves no trail behind its edge");
let f = flicker(#{ frames: 60, threshold: 10.0 });
log(`still sun: mean_delta ${f.mean_delta}, popped_share ${f.popped_share}`);
check_lt(f.popped_share, 0.002, "a still sun shadow does not flicker");
