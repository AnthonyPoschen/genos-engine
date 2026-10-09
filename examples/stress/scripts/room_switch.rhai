// Teleporting between rooms: the new view must light up and settle, not stay dark.
time(#{ fixed_dt: 0.0166667 });
knob("time_of_day", 12.0);
knob("sun", "freeze");
knob("boxes", "still");
knob("dynamic", 0.0);
camera(#{ position: [12.5, 1.7, 20.0], look_at: [12.5, 1.0, 8.0] });
wait_settled(#{ timeout_frames: 900 });
let a = luminance(#{});
camera(#{ position: [21.0, 1.7, 23.5], look_at: [21.0, 1.0, 17.0] });
wait(2);
shot("switch-2-frames");
let s = wait_settled(#{ timeout_frames: 900 });
shot("switch-settled");
let b = luminance(#{});
log(`before ${a.luminance}, after ${b.luminance}, settled ${s.settled} in ${s.frames} frames`);
check(s.settled, "the new room settles within 900 frames");
check_gt(b.luminance, 0.02, "the new room is not black");
