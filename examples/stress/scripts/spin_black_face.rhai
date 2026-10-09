// 2e49ee3: a spinning box showed a black face. Spin a box in view and require its
// faces to stay lit in every frame of the turn.
time(#{ fixed_dt: 0.0166667 });
knob("time_of_day", 12.0);
knob("sun", "freeze");
knob("boxes", "still");
let boxes = objects().filter(|o| o.height <= 1.2 && o.size <= 1.5);
require(boxes.len() > 0, "the scene has a box");
let b = boxes[0];
let p = b.position;
camera(#{ position: [p[0], p[1] + 1.6, p[2] + 3.0], look_at: [p[0], p[1] + 0.4, p[2]] });
wait_settled(#{ timeout_frames: 600 });
object(#{ index: b.index, spin: 2.0 });
let worst = 0.0;
for i in 0..12 {
    wait(5);
    let l = luminance(#{ region: [0.4, 0.4, 0.2, 0.25] });
    if l.black_share > worst { worst = l.black_share; }
    if i % 4 == 0 { shot(`spin-${i}`); }
}
log(`worst black share on the box ${worst}`);
check_lt(worst, 0.05, "no face of the spinning box goes black");
