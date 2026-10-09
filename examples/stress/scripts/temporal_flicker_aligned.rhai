// GI v2 gate (docs/systems/gi-v2-design.md, "temporal_flicker.rhai"); also run on the
// gi-v2-baseline tag for the head-to-head. Per stress pose: (1) everything frozen after
// settling: the picture must not change at all; (2) lamps and boxes moving, camera
// still: frame-to-frame change in the region that holds no moving lamp or box.
// Logged per pose: mean_delta, max_delta (linear luminance) and popped_share.
//
// Aligned variant: each pose's settle wait is padded to SETTLE_PAD frames, so every
// lighting path reaches each moving window at the same scene time with the lamps and
// boxes in the same places. In temporal_flicker.rhai a path that settles faster
// starts its moving windows earlier and sees other lamp positions, so its moving
// numbers measure a different scene.
time(#{ fixed_dt: 0.0166667 });
knob("time_of_day", 12.0);
knob("sun", "freeze");

let poses = [
    ["hall", #{ position: [14.5, 1.7, 23.5], yaw: -0.3554, pitch: 0.08 }],
    ["room-a", #{ position: [7.2, 1.7, 0.9], yaw: -2.556, pitch: -0.12 }],
    ["doorway", #{ position: [12.0, 1.6, 20.5], look_at: [22.0, 1.2, 20.5] }],
    ["outside", #{ position: [12.5, 1.7, 34.0], look_at: [12.5, 1.5, 22.0] }],
];
const SETTLE_PAD = 900;
let worst_frozen = 0.0;
let worst_moving = 0.0;
for p in poses {
    let name = p[0];
    knob("boxes", "still");
    knob("dynamic", 0.0);
    camera(p[1]);
    let s = wait_settled(#{ timeout_frames: SETTLE_PAD });
    let spent = if type_of(s.frames) == "f64" { s.frames.to_int() } else { s.frames };
    if spent < SETTLE_PAD { wait(SETTLE_PAD - spent); }
    let f = flicker(#{ frames: 120, threshold: 0.004 });
    log(`${name} frozen: settled ${s.settled} in ${s.frames} | mean_delta ${f.mean_delta} max_delta ${f.max_delta} popped_share ${f.popped_share}`);
    if f.max_delta > worst_frozen { worst_frozen = f.max_delta; }
    knob("dynamic", 50.0);
    knob("boxes", "move");
    wait(60);
    // The upper third: walls and ceiling, away from boxes on the floor.
    let m = flicker(#{ frames: 120, threshold: 0.004, region: [0.0, 0.0, 1.0, 0.33] });
    log(`${name} moving: mean_delta ${m.mean_delta} max_delta ${m.max_delta} popped_share ${m.popped_share}`);
    if m.mean_delta > worst_moving { worst_moving = m.mean_delta; }
}
knob("dynamic", 0.0);
knob("boxes", "still");
log(`worst frozen max_delta ${worst_frozen}, worst moving mean_delta ${worst_moving}`);
check_lt(worst_frozen, 0.004, "a frozen scene does not flicker (under 1 display code)");
