// Dragging a box left lit squares on the floor. Slide a box across the floor, stop,
// let it settle and require the floor it left to be calm and not patchy.
time(#{ fixed_dt: 0.0166667 });
knob("time_of_day", 12.0);
knob("sun", "freeze");
knob("boxes", "still");
knob("dynamic", 0.0);
knob("boxes", "still");
let boxes = objects().filter(|o| o.height <= 1.2 && o.size <= 1.5);
require(boxes.len() > 0, "the scene has a box");
let b = boxes[0];
let p = b.position;
camera(#{ position: [p[0], p[1] + 2.5, p[2] + 4.0], look_at: [p[0], 0.0, p[2]] });
wait_settled(#{ timeout_frames: 600 });
let before = capture(#{ name: "before", path: "floor-before.png" });
object(#{ index: b.index, path: [p, [p[0] + 2.0, p[1], p[2]], p], frames: 120 });
wait(130);
wait_settled(#{ timeout_frames: 600 });
let after = capture(#{ name: "after", path: "floor-after.png" });
let d = diff(#{ a: "before", b: "after", out: "floor-diff.png", threshold: 12 });
log(`back in place: mean_abs ${d.mean_abs}, over_share ${d.over_share}`);
check_lt(d.over_share, 0.02, "the floor looks the same once the box is back");
let f = flicker(#{ frames: 30, region: [0.2, 0.5, 0.6, 0.5] });
check_lt(f.popped_share, 0.01, "the floor does not pop after the drag");
