// A shadowed outer wall facing sunlit ground took about twice the bounce it
// should: the cascade blended neighbouring 90-degree direction bins, so the
// sunlit ground below the horizon was read in the empty directions above it.
// With the sky off, the wall's light is sun -> ground -> wall only. Grade a
// patch of it against the path-traced reference.
time(#{ fixed_dt: 0.0166667, pause: true });
knob("time_of_day", 13.09);
knob("sun_speed", 0.0);
knob("boxes", "still");
knob("lamps", "still");
knob("lights", 0);
knob("sky", 0);
camera(#{ position: [6.6371, 1.8212, 30.1017], look_at: [6.0, 0.35, 25.15] });
wait_settled(#{ timeout_frames: 900 });
let c = compare(#{ prefix: "wall", width: 320, spp: 64, max_spp: 256, noise: 0.03, seconds: 240, block: 2, region: [0.47, 0.46, 0.06, 0.08] });
log(`wall patch: bias ${c.bias}, ref ${c.ref_mean}, live ${c.live_mean}`);
check_lt(abs(c.bias), 0.5, "the shadowed wall's ground bounce is within half of the reference");
