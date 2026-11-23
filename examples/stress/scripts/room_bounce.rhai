// A room lit only through an opening by a sunlit wall was too dark: GI rays
// stopped at 8 cell sizes and the far light came through 16 coarse parent
// bins. Grade the room's regions against a 64-bounce path-traced reference.
time(#{ fixed_dt: 0.0166667, pause: true });
knob("time_of_day", 12.2);
knob("sun_speed", 0.0);
knob("boxes", "still");
knob("lamps", "still");
knob("lights", 5);
camera(#{ position: [3.15969, 1.48957, 1.28635], yaw: 3.03737, pitch: -0.226 });
wait_settled(#{ timeout_frames: 900 });
wait(60);
reference(#{ name: "r", width: 320, spp: 64, max_spp: 256, noise: 0.03, seconds: 240, bounces: 64 });
let regions = #{
    floor_near: [0.1875, 0.833, 0.75, 0.156],
    floor_mid: [0.125, 0.667, 0.375, 0.139],
    left_wall: [0.031, 0.111, 0.219, 0.278],
    ceiling: [0.1875, 0.0, 0.625, 0.078],
    back_wall: [0.734, 0.222, 0.125, 0.444],
    outside_wall: [0.344, 0.222, 0.3125, 0.167],
};
for k in regions.keys() {
    let c = compare(#{ reference: "r", region: regions[k], block: 2, prefix: `rg-${k}` });
    let ratio = c.live_mean / c.ref_mean;
    log(`${k}: ref ${c.ref_mean} live ${c.live_mean} ratio ${ratio}`);
    check_lt(abs(ratio - 1.0), 0.35, `${k} is within 35% of the reference`);
}
