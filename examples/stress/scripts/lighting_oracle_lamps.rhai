// Many-lamp oracle scenarios: scripts/lighting/oracle_run.sh lamps.
// lighting_oracle.rhai's scene has 5 point lamps, so every pixel has at most 5 lit
// lamps and shades the exact sum (LAMP_PICK_EXACT_LIT = 8). These scenarios put
// LAMPS lamps in the camera's section, a quarter of them moving, so near pixels
// see far more than 8 lit lamps and the random lamp path is what gets graded.
// Same two passes and the same reference grading as lighting_oracle.rhai.
let PASS = "temporal";
let PROFILE = "lamps";
let HIST = 30;
let EVERY = 12;
let LAMPS = 32;

time(#{ fixed_dt: 1.0 / 60.0 });
knob("boxes", "still");
knob("layout", "first");
knob("lights", LAMPS);
knob("dynamic", 25);
knob("lamps", "move");
knob("lamp_clock", 0.0);

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

fn frames(dir, n) {
    gi_metrics(#{ reset: true });
    let i = 0;
    while i < n { capture(#{ path: `temporal/${dir}/live-${pad(i)}.png` }); i += 1; }
    gi_metrics(#{ write: dir });
}

fn points() { lights().filter(|l| !l.directional) }

// Point lamps whose reach sphere holds p: an upper bound on point_lit there
// (the shader also drops lamps behind the surface).
fn reach(p) {
    let n = 0;
    for l in points() {
        if !l.on { continue; }
        let dx = l.position[0] - p[0]; let dy = l.position[1] - p[1]; let dz = l.position[2] - p[2];
        if dx * dx + dy * dy + dz * dz <= l.range * l.range { n += 1; }
    }
    n
}

fn cover(dir, p) {
    let n = reach(p);
    log(`LAMP_COVER ${dir} lamps=${points().len()} reach=${n}`);
    require(n > 8, `${dir}: only ${n} lamps reach the subject, so it would shade the exact sum`);
}

// A box spinning among the lamps at night, lamps moving.
fn spin(pass, hist, every) {
    knob("time_of_day", 22.0);
    knob("sun", "freeze");
    for l in points() { light(#{ index: l.index, on: true }); }
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
    knob("lamp_clock", 0.0);
    wait(120);
    cover("lamps-spin", [p[0], p[1] + 0.4, p[2]]);
    let n = 240;
    if pass == "temporal" {
        knob("lamp_clock", 2.0);
        object(#{ index: b.index, spin: 2.0 });
        frames("lamps-spin", n);
    } else {
        let f = hist;
        while f < n {
            knob("lamp_clock", 2.0 + (f - hist) / 60.0);
            object(#{ index: b.index, yaw: 2.0 * (f - hist) / 60.0, spin: 2.0 });
            wait(hist);
            measure("lamps-spin", f);
            f += every * 2;
        }
    }
    object(#{ index: b.index, spin: 0.0 });
}

// Half the lamps lit, then the other half switches on in one frame, lamps moving.
fn light_switch(pass) {
    knob("time_of_day", 0.0);
    knob("sun", "freeze");
    camera(#{ position: [12.5, 1.7, 20.0], look_at: [12.5, 1.0, 8.0] });
    let i = 0;
    for l in points() { light(#{ index: l.index, on: i % 2 == 0 }); i += 1; }
    knob("lamp_clock", 10.0);
    wait(600);
    if pass == "temporal" {
        frames("lamps-switch-half", 8);
        for l in points() { light(#{ index: l.index, on: true }); }
        cover("lamps-switch", [12.5, 1.0, 14.0]);
        frames("lamps-switch", 120);
    } else {
        for l in points() { light(#{ index: l.index, on: true }); }
        cover("lamps-switch", [12.5, 1.0, 14.0]);
        let at = 0;
        for k in [0, 1, 2, 4, 8, 16, 32, 64, 119] {
            if k > at { wait(k - at); at = k; }
            measure("lamps-switch", k);
        }
    }
}

spin(PASS, HIST, EVERY);
light_switch(PASS);
log(`lighting_oracle_lamps ${PASS} done`);
