// gi-check (docs/systems/debugging.md): the quick flicker check. The doorway pose
// of temporal_flicker_aligned.rhai (the full tier's worst), settled, then lamps
// (50 % dynamic) and boxes move at a fixed 60 Hz step, camera still; after 30
// frames, 40 frames of the picture are saved to move/. gi-check scores the upper
// third (walls and ceiling, no box in it) by how much its light goes back and
// forth: per 4 x 4 block at 320 x 180, the sum of the frame-to-frame changes
// less the net change, per frame. Light that moves with a lamp drifts one way
// and scores near zero; kept rays that come and go flicker. Filled in: __DIR__.
time(#{ fixed_dt: 0.0166667 });
knob("time_of_day", 12.0);
knob("sun", "freeze");
knob("boxes", "still");
knob("dynamic", 0.0);
knob("gi", __GI__);
camera(#{ position: [12.0, 1.6, 20.5], look_at: [22.0, 1.2, 20.5] });
for k in 0..4 { try { wait_settled(#{ timeout_frames: 600 }); break; } catch (e) { log(`settle ${k}: ${e}`); } }
knob("dynamic", 50.0);
knob("boxes", "move");
wait(30);
let m = flicker(#{ frames: 40, threshold: 0.004, region: [0.0, 0.0, 1.0, 0.33], frames_dir: "__DIR__/move" });
log(`FLICK mean_delta ${m.mean_delta} max_delta ${m.max_delta} popped_share ${m.popped_share}`);
knob("dynamic", 0.0);
knob("boxes", "still");
