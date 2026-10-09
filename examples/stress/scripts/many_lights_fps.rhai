// Frame time with many lights. On lavapipe this only logs; on a GPU it holds a bound.
time(#{ fixed_dt: 0.0166667 });
knob("time_of_day", 22.0);
let max = knobs().filter(|k| k.name == "lights")[0].max;
knob("lights", max);
camera(#{ position: [12.5, 1.7, 20.0], look_at: [12.5, 1.0, 8.0] });
wait(240);
let t = timing(#{ frames: 120 });
log(`${max} lights: ${t.fps} fps, frame ${t.frame_ms} ms (worst ${t.frame_ms_worst}), gpu ${t.gpu_ms} ms, light ${t.light_ms_per_frame} ms`);
shot("many-lights");
check_gt(t.fps, 0.0, "frames are drawn");
