// Sun at four times its speed, seen from the yard: dusk into night and dawn into day
// must be one continuous change. A pop is one frame that moves the picture's mean
// light by a large share of the whole change.
time(#{ fixed_dt: 1.0 / 60.0 });
knob("boxes", 1.0);
knob("dynamic", 0.0);
knob("sun_speed", 4.0);
camera(#{ position: [12.5, 6.0, 48.0], look_at: [12.5, 1.5, 25.0] });
for leg in [["dusk", 16.5], ["dawn", 3.5]] {
    knob("time_of_day", leg[1]);
    wait(#{ frames: 60 });
    let f = flicker(#{ frames: 420, frames_dir: `frames-${leg[0]}` });
    let m = f.means;
    let worst = 0.0;
    let at = 0;
    for i in 1..m.len() {
        let d = (m[i] - m[i - 1]).abs();
        if d > worst { worst = d; at = i; }
    }
    let range = if f.mean_range > 1.0e-4 { f.mean_range } else { 1.0e-4 };
    let share = worst / range;
    log(`${leg[0]}: mean ${m[0]} -> ${m[m.len() - 1]}, range ${f.mean_range}, largest step ${worst} at frame ${at} (${share} of the range)`);
    // The world volume still steps at each build (about 10% of a fast dusk); it does
    // not blend yet. Before it followed the sun, the yard dropped 22% in one frame.
    check_lt(share, 0.12, `${leg[0]}: no frame moves the mean light by 12% of the whole change`);
}
