// Spinning-object patchiness (Anthony, 28 Oct: a box spinning into and out of a
// lamp stays patchy for 3-5 s). Spin a box beside the hall lamps for 4 s, stop it,
// save every frame. grid_metric.py reports the worst grid score while spinning and
// the frames until the box matches its settled picture after the stop.
time(#{ fixed_dt: 1.0 / 60.0 });
knob("time_of_day", 22.0);
knob("sun", "freeze");
knob("boxes", "still");
let boxes = objects().filter(|o| o.height <= 1.2 && o.size <= 1.5);
require(boxes.len() > 0, "the scene has a box");
let b = boxes[0];
let p = b.position;
camera(#{ position: [p[0], p[1] + 1.6, p[2] + 3.0], look_at: [p[0], p[1] + 0.4, p[2]] });
// wait_settled gives up after 121 s on a software GPU; a fixed settle works on both.
wait(#{ frames: 180 });
object(#{ index: b.index, spin: 2.0 });
for k in 0..3 { flicker(#{ frames: 80, frames_dir: `spin-moving/part-${k}` }); }
object(#{ index: b.index, spin: 0.0 });
for k in 0..4 { flicker(#{ frames: 90, frames_dir: `spin-stop/part-${k}` }); }
log("spin frames saved");
