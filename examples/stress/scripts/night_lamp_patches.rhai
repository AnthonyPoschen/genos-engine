// At night most bounce comes from the small ceiling patch over each lamp,
// 0.4 m above it. An upper cell traced the same few directions on every
// refresh, so a ray that hit that patch kept a bright bin, and the floor under
// the cell showed a fixed bright patch. Hold the scene still and require no
// small floor patch of the bounce view to stand far above its surroundings.
time(#{ fixed_dt: 0.0166667, pause: true });
knob("time_of_day", 3.03333);
knob("sun_speed", 0.0);
knob("boxes", "still");
knob("lamps", "still");
knob("lamp_clock", 90.76662);
knob("box_clock", 90.76662);
knob("lights", 5);
camera(#{ position: [8.10221, 1.97847, 24.14327], yaw: -0.10364, pitch: -0.457 });
wait_settled(#{ timeout_frames: 900 });
wait(60);
view("bounce");
wait(30);
let worst = 0.0;
let gx = 0.40;
while gx < 0.75 {
    let gy = 0.50;
    while gy < 0.91 {
        let small = luminance(#{ region: [gx, gy, 0.03, 0.04] }).luminance;
        let big = luminance(#{ region: [gx - 0.06, gy - 0.08, 0.15, 0.20] }).luminance;
        if big > 0.0 && small / big > worst {
            worst = small / big;
        }
        gy += 0.04;
    }
    gx += 0.03;
}
log(`worst patch over surround: ${worst}`);
check_lt(worst, 1.25, "no fixed bright GI patch on the floor under a lamp-lit ceiling");
let f = flicker(#{ frames: 60, threshold: 4.0 });
log(`still: mean_delta ${f.mean_delta}, max_delta ${f.max_delta}`);
check_lt(f.max_delta, 0.05, "the bounce does not shimmer while still");
