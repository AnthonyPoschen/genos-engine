// A moving box under a low sun left a staircase of old shadow edges: its
// shadow reaches metres past the box, outside the mover's bounds, so the
// pixels it uncovered kept their dark area mean and brightened over many
// frames. Move the boxes, stop them, and require the frame they stop on to
// match the picture settled with them still.
time(#{ fixed_dt: 0.0166667 });
knob("time_of_day", 9.4);
knob("sun_speed", 0.0);
knob("lamps", "still");
knob("boxes", "move");
view("direct");
let p = [23.27701, 1.7, 22.15968];
for i in 0..250 {
    let o = objects()[12];
    camera(#{ position: p, look_at: [o.position[0], 0.4, o.position[2]] });
    wait(1);
}
knob("boxes", "still");
capture(#{ name: "moving", path: "box-moving.png" });
wait_settled(#{ timeout_frames: 600 });
wait(30);
capture(#{ name: "settled", path: "box-settled.png" });
let d = diff(#{ a: "moving", b: "settled", out: "box-diff.png", threshold: 10 });
log(`moving vs settled: mean_abs ${d.mean_abs}, over_share ${d.over_share}`);
check_lt(d.over_share, 0.025, "a moving box's sun shadow leaves no trail");
