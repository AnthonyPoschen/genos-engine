// A still night scene kept popping on the outer wall: each upper-cell
// refresh replaced its bins with one to four rays each, and the cells feed
// each other's bounce, so the picture never settled. Hold everything still,
// let it settle, and require no frame step to move a pixel by much.
time(#{ fixed_dt: 0.0166667 });
knob("time_of_day", 1.45);
knob("sun_speed", 0.0);
knob("boxes", "still");
knob("lamps", "still");
camera(#{ position: [5.43335, 6.04157, 44.46607], yaw: 0.19336, pitch: -0.181 });
wait(900);
let f = flicker(#{ frames: 120, threshold: 4.0 });
log(`still: mean_delta ${f.mean_delta}, max_delta ${f.max_delta}`);
check_lt(f.max_delta, 0.05, "a still scene's GI does not pop");
