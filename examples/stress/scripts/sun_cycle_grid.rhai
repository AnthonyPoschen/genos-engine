// Sun sweep checkerboard repro (Anthony, 28 Oct: a fixed grid of patches shows for
// seconds at sunrise and sunset). Runs one FULL day (sunrise and sunset included)
// at 1x, 10x and 100x sun speed and saves every frame; scripts/lighting/
// sun_cycle_grid.sh scores them with grid_metric.py (worst frame of the cycle).
// 100x is the main repro: about 5 degrees of sun per frame, every frame.
// Frames per day = day_seconds (default 120) * 60 / speed. Needs the default
// --day-seconds 120.
time(#{ fixed_dt: 1.0 / 60.0 });
knob("boxes", 1.0);
knob("dynamic", 0.0);
camera(#{ position: [12.5, 6.0, 48.0], look_at: [12.5, 1.5, 25.0] });
for speed in [100.0, 10.0, 1.0] {
    knob("sun_speed", speed);
    knob("time_of_day", 5.0);
    wait(#{ frames: 30 });
    let n = (120.0 * 60.0 / speed + 0.5).to_int() + 30;
    // flicker gives up after 121 s, so the day is saved in 100-frame parts
    // (part-000, part-001, ...); grid_metric.py reads them in order as one run.
    let parts = (n + 99) / 100;
    for k in 0..parts {
        let left = if n - k * 100 < 100 { n - k * 100 } else { 100 };
        let tag = if k < 10 { `00${k}` } else if k < 100 { `0${k}` } else { `${k}` };
        flicker(#{ frames: left, frames_dir: `sun-x${speed.to_int()}/part-${tag}` });
    }
    log(`sun x${speed}: ${n} frames in ${parts} parts`);
}
