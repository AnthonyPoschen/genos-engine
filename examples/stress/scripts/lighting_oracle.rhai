// Lighting harness and oracle (compute-rethink stage A).
// docs/systems/lighting/oracle-harness.md explains the metrics.
// Run it through scripts/lighting/oracle_run.sh, which fills in PASS and PROFILE
// and scores the pictures with scripts/lighting/oracle_metrics.py.
//
// PASS "temporal": every scenario runs uninterrupted and saves every frame
// (grid score, unchanged-work counter). No reference traces, so the live
// lighting gets exactly one drawn frame per scene step.
// PASS "oracle": for each reference frame, rewind the scenario by HIST frames,
// run those frames moving (so the live cache has a moving history), pause, save
// the live picture and depth, and path-trace the same frozen moment.
let PASS = "temporal";
let PROFILE = "quick";
let HIST = 30;
let REF_EVERY = if PROFILE == "full" { 1 } else { 12 };
let DAY = 120.0; // stress default --day-seconds 120

time(#{ fixed_dt: 1.0 / 60.0 });
knob("boxes", "still");
knob("lamps", "still");

fn pad(i) { if i < 10 { `000${i}` } else if i < 100 { `00${i}` } else if i < 1000 { `0${i}` } else { `${i}` } }

fn measure(dir, f) {
    time(#{ pause: true });
    capture(#{ path: `oracle/${dir}/f${pad(f)}-live.png` });
    capture(#{ path: `oracle/${dir}/f${pad(f)}-depth.png`, mode: "depth" });
    let r = reference(#{ prefix: `oracle/${dir}/f${pad(f)}`, spp: 16, max_spp: 256,
                         noise: 0.05, seconds: 30, bounces: 3 });
    log(`ORACLE ${dir} f=${f} spp=${r.spp} noise=${r.noise} s=${r.trace_seconds}`);
    time(#{ pause: false });
}

fn frames(dir, n, stride) {
    gi_metrics(#{ reset: true });
    let i = 0;
    while i < n {
        capture(#{ path: `temporal/${dir}/live-${pad(i)}.png` });
        if stride > 1 { wait(stride - 1); }
        i += stride;
    }
    gi_metrics(#{ write: dir });
}

fn lamps_on(on) {
    for l in lights() { if !l.directional { light(#{ index: l.index, on: on }); } }
}

// Sun cycle: one full rotation (sunrise and sunset) from 05:00.
fn sun(speed, pass, hist, every, day) {
    let dir = `sun-x${speed.to_int()}`;
    let n = (day * 60.0 / speed + 0.5).to_int() + 1;
    let hours_per_frame = 24.0 * speed / (day * 60.0);
    camera(#{ position: [12.5, 6.0, 48.0], look_at: [12.5, 1.5, 25.0] });
    // The sun_speed knob tops out at x64, so the script drives the clock
    // itself: sun frozen, time_of_day stepped once per frame.
    knob("sun", "freeze");
    if pass == "temporal" {
        knob("time_of_day", 5.0);
        wait(30);
        let stride = if speed < 5.0 { 4 } else { 1 };
        gi_metrics(#{ reset: true });
        let i = 0;
        while i < n {
            knob("time_of_day", (5.0 + i * hours_per_frame) % 24.0);
            capture(#{ path: `temporal/${dir}/live-${pad(i)}.png` });
            i += stride;
        }
        gi_metrics(#{ write: dir });
    } else {
        let step = if every == 1 { 1 } else if speed < 5.0 { every * 40 } else if speed < 50.0 { every * 5 } else { every };
        let f = 0;
        while f < n {
            let k = f - hist;
            while k < f {
                let h = (5.0 + k * hours_per_frame + 24.0) % 24.0;
                knob("time_of_day", h);
                wait(1);
                k += 1;
            }
            measure(dir, f);
            f += step;
        }
    }
}

// A box spinning beside a hall lamp at night.
fn spin(pass, hist, every) {
    knob("time_of_day", 22.0);
    knob("sun", "freeze");
    lamps_on(true);
    // The box nearest the lit hall (12.5, 20); the first box in the list is
    // outdoors and black at night.
    let boxes = objects().filter(|o| o.height <= 1.2 && o.size <= 1.5);
    require(boxes.len() > 0, "the scene has a box");
    let b = boxes[0];
    let best = 1e9;
    for o in boxes {
        let dx = o.position[0] - 12.5;
        let dz = o.position[2] - 20.0;
        if dx * dx + dz * dz < best { best = dx * dx + dz * dz; b = o; }
    }
    let p = b.position;
    camera(#{ position: [p[0], p[1] + 1.6, p[2] + 3.0], look_at: [p[0], p[1] + 0.4, p[2]] });
    object(#{ index: b.index, yaw: 0.0, spin: 0.0 });
    wait(120);
    let n = 240;
    if pass == "temporal" {
        object(#{ index: b.index, spin: 2.0 });
        frames("spin", n, 1);
    } else {
        let f = hist;
        while f < n {
            object(#{ index: b.index, yaw: 2.0 * (f - hist) / 60.0, spin: 2.0 });
            wait(hist);
            measure("spin", f);
            f += every * 2;
        }
    }
    object(#{ index: b.index, spin: 0.0 });
}

// Dark room, every lamp switches on in one frame (the worst case: a full re-light).
fn light_switch(pass) {
    knob("time_of_day", 0.0);
    knob("sun", "freeze");
    camera(#{ position: [12.5, 1.7, 20.0], look_at: [12.5, 1.0, 8.0] });
    lamps_on(false);
    // The cache fades slowly after lamps go off (still about 70% of the lit
    // picture 60 frames later), so hold the dark for 10 s before the switch.
    wait(600);
    if pass == "temporal" {
        frames("switch-dark", 8, 1);
        lamps_on(true);
        frames("switch", 120, 1);
    } else {
        // One switch event; measure() pauses, so the traces add no settling.
        lamps_on(true);
        let at = 0;
        for k in [0, 1, 2, 4, 8, 16, 32, 64, 119] {
            if k > at { wait(k - at); at = k; }
            measure("switch", k);
        }
    }
}

// Camera turn: 90 degrees in one second inside the lit hall.
fn turn_pose(f) {
    let a = 1.5707963 * f / 60.0;
    [12.5, 1.7, 20.0, a, -0.05]
}
fn turn(pass, hist, every) {
    knob("time_of_day", 22.0);
    knob("sun", "freeze");
    lamps_on(true);
    let start = turn_pose(0);
    camera(#{ position: [start[0], start[1], start[2]], yaw: start[3], pitch: start[4] });
    wait(120);
    let n = 90; // 60 frames turning, 30 held
    if pass == "temporal" {
        camera(#{ path: [turn_pose(0), turn_pose(60)], frames: 60 });
        frames("turn", n, 1);
    } else {
        let f = 0;
        while f < n {
            let a = if f > 60 { 60 } else { f };
            let s = if a - hist < 0 { 0 } else { a - hist };
            let p = turn_pose(s);
            camera(#{ position: [p[0], p[1], p[2]], yaw: p[3], pitch: p[4] });
            camera(#{ path: [turn_pose(s), turn_pose(a)], frames: if a > s { a - s } else { 1 } });
            wait(hist);
            measure("turn", f);
            f += every / 2;
        }
    }
}

let speeds = if PROFILE == "full" { [100.0, 10.0, 1.0] } else { [100.0, 10.0] };
for speed in speeds { sun(speed, PASS, HIST, REF_EVERY, DAY); }
spin(PASS, HIST, REF_EVERY);
light_switch(PASS);
turn(PASS, HIST, REF_EVERY);
log(`lighting_oracle ${PASS} ${PROFILE} done`);
