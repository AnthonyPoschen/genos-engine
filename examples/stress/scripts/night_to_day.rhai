// After a night inside, stepping outdoors at noon must show day light, not night.
time(#{ fixed_dt: 0.0166667 });
knob("sun", "freeze");
knob("boxes", "still");
knob("dynamic", 0.0);
knob("time_of_day", 22.0);
camera(#{ position: [12.5, 1.7, 14.0], look_at: [12.5, 1.0, 8.0] });
wait_settled(#{ timeout_frames: 900 });
shot("night-inside");
knob("time_of_day", 12.0);
camera(#{ position: [12.5, 1.7, 34.0], look_at: [12.5, 1.0, 20.0] });
let s = wait_settled(#{ timeout_frames: 900 });
shot("noon-outside");
let l = luminance(#{});
log(`outside at noon ${l.luminance}, settled ${s.settled} in ${s.frames} frames`);
check_gt(l.luminance, 0.15, "outdoors at noon is bright");
let c = compare(#{ prefix: "noon-outside", width: 160, noise: 0.04, seconds: 400 });
log(`against ground truth: mean_rel ${c.mean_rel} bias ${c.bias}`);
check_lt(c.bias.abs(), 0.25, "outdoor exposure within 25% of ground truth");
