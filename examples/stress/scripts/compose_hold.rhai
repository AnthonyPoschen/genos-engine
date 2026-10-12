// Quiet compose: two held frames, then a camera move.
// Short on purpose. A settled still scene should match; early frames may still
// differ while the cache is filling. A camera move must change the picture.
time(#{ pause: true });
knob("time_of_day", 12.0);
knob("sun", "freeze");
knob("boxes", "still");
knob("dynamic", 0.0);
camera(#{ position: [12.5, 1.7, 20.0], look_at: [12.5, 1.0, 8.0] });
step(#{ frames: 8 });
capture(#{ name: "still-a", keep_only: true });
step(#{ frames: 1 });
capture(#{ name: "still-b", keep_only: true });
let still = diff(#{ a: "still-a", b: "still-b" });
log(`still mean_abs ${still.mean_abs}`);
camera(#{ position: [18.0, 2.4, 14.0], look_at: [12.5, 1.0, 8.0] });
step(#{ frames: 2 });
capture(#{ name: "moved", keep_only: true });
let moved = diff(#{ a: "still-b", b: "moved" });
log(`moved mean_abs ${moved.mean_abs}`);
check_gt(moved.mean_abs, 0.001, "camera move changes the picture");
let lum = luminance(#{});
check_gt(lum.luminance, 0.01, "the picture is not black");
