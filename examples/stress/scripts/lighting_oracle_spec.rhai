// Specular oracle scenarios (stage S0, docs/systems/lighting/specular-v3-design.md).
// Run through scripts/lighting/oracle_run.sh with SPEC=1; scored by
// scripts/lighting/oracle_metrics.py (spec_err_near, spec_pop, frames_to_ref, spec_ms).
// Same PASS/PROFILE/HIST conventions as lighting_oracle.rhai.
//
// Engine hooks this script needs (stage S0b, not yet in the script API):
//   object(#{ index, roughness, metallic })  per-solid material override
//   reference(#{ ..., aov: "specular" })      writes <prefix>-reference-spec.png
//   gi_metrics spec_ms column                 GPU time of specular steps 1-5
// Until they land the scenes run but every surface keeps its scene material.
let PASS = "temporal";
let PROFILE = "quick";
let HIST = 30;
let REF_EVERY = if PROFILE == "full" { 1 } else { 12 };

time(#{ fixed_dt: 1.0 / 60.0 });
knob("boxes", "still");
knob("lamps", "still");
knob("sun", "freeze");

fn pad(i) { if i < 10 { `000${i}` } else if i < 100 { `00${i}` } else if i < 1000 { `0${i}` } else { `${i}` } }

fn measure(dir, f) {
    time(#{ pause: true });
    capture(#{ path: `oracle/${dir}/f${pad(f)}-live.png` });
    capture(#{ path: `oracle/${dir}/f${pad(f)}-depth.png`, mode: "depth" });
    let r = reference(#{ prefix: `oracle/${dir}/f${pad(f)}`, spp: 16, max_spp: 256,
                         noise: 0.05, seconds: 30, bounces: 3, aov: "specular" });
    log(`ORACLE ${dir} f=${f} spp=${r.spp} noise=${r.noise} s=${r.trace_seconds}`);
    time(#{ pause: false });
}

fn frames(dir, n) {
    gi_metrics(#{ reset: true });
    for i in 0..n { capture(#{ path: `temporal/${dir}/live-${pad(i)}.png` }); }
    gi_metrics(#{ write: dir });
}

fn lamps_on(on) {
    for l in lights() { if !l.directional { light(#{ index: l.index, on: on }); } }
}

// Small solids nearest a point, closest first.
fn nearest(x, z, k) {
    let boxes = objects().filter(|o| o.height <= 1.2 && o.size <= 1.5);
    boxes.sort(|a, b| {
        let da = (a.position[0] - x) ** 2 + (a.position[2] - z) ** 2;
        let db = (b.position[0] - x) ** 2 + (b.position[2] - z) ** 2;
        if da < db { -1 } else if da > db { 1 } else { 0 }
    });
    boxes.extract(0, if k < boxes.len() { k } else { boxes.len() })
}

fn night_hall() {
    knob("time_of_day", 22.0);
    lamps_on(true);
}

// 1. gloss-sweep: five solids at roughness 0.05..0.9, metal and dielectric rows.
fn gloss_sweep(pass, every) {
    night_hall();
    let rough = [0.05, 0.2, 0.4, 0.6, 0.9];
    let s = nearest(12.5, 20.0, 10);
    require(s.len() >= 5, "the scene has five small solids near the hall");
    for i in 0..s.len() {
        object(#{ index: s[i].index, roughness: rough[i % 5], metallic: if i < 5 { 1.0 } else { 0.0 } });
    }
    camera(#{ position: [12.5, 1.7, 24.0], look_at: [12.5, 0.6, 20.0] });
    wait(120);
    if pass == "temporal" { frames("gloss-sweep", 60); }
    else { let f = 0; while f < 60 { measure("gloss-sweep", f); wait(every); f += every; } }
    for o in s { object(#{ index: o.index, reset: true }); }
}

// 2. gloss-edge: camera pans so a bright reflected lamp crosses the screen edge.
fn edge_pose(f) { [12.5, 1.7, 22.0, -0.6 + 1.2 * f / 120.0, -0.35] }
fn gloss_edge(pass, hist, every) {
    night_hall();
    let s = nearest(12.5, 20.0, 1);
    for o in s { object(#{ index: o.index, roughness: 0.05, metallic: 1.0 }); }
    let n = 120;
    if pass == "temporal" {
        let p = edge_pose(0);
        camera(#{ position: [p[0], p[1], p[2]], yaw: p[3], pitch: p[4] });
        wait(60);
        camera(#{ path: [edge_pose(0), edge_pose(n)], frames: n });
        frames("gloss-edge", n);
    } else {
        let f = hist;
        while f < n {
            let a = f - hist;
            let p = edge_pose(a);
            camera(#{ position: [p[0], p[1], p[2]], yaw: p[3], pitch: p[4] });
            camera(#{ path: [edge_pose(a), edge_pose(f)], frames: hist });
            wait(hist);
            measure("gloss-edge", f);
            f += every / 2;
        }
    }
    for o in s { object(#{ index: o.index, reset: true }); }
}

// 3. gloss-offscreen: glossy surface reflecting a lit wall behind the camera.
fn gloss_offscreen(pass, every) {
    night_hall();
    let s = nearest(12.5, 14.0, 3);
    for o in s { object(#{ index: o.index, roughness: 0.05, metallic: 1.0 }); }
    camera(#{ position: [12.5, 1.2, 17.0], look_at: [12.5, 0.0, 13.0] });
    wait(120);
    if pass == "temporal" { frames("gloss-offscreen", 60); }
    else { let f = 0; while f < 60 { measure("gloss-offscreen", f); wait(every); f += every; } }
    for o in s { object(#{ index: o.index, reset: true }); }
}

// 4. gloss-switch: lamp seen only in reflection turns on (30 dark, 120 on, 120 off).
fn gloss_switch(pass) {
    knob("time_of_day", 0.0);
    lamps_on(false);
    let s = nearest(12.5, 20.0, 1);
    for o in s { object(#{ index: o.index, roughness: 0.05, metallic: 1.0 }); }
    camera(#{ position: [12.5, 1.0, 22.0], look_at: [12.5, 0.2, 20.0] });
    wait(600);
    if pass == "temporal" {
        frames("gloss-switch-dark", 30);
        lamps_on(true);
        frames("gloss-switch", 120);
        lamps_on(false);
        frames("gloss-switch-off", 120);
    } else {
        lamps_on(true);
        let at = 0;
        for k in [0, 1, 2, 4, 8, 16, 32, 64, 119] {
            if k > at { wait(k - at); at = k; }
            measure("gloss-switch", k);
        }
    }
    for o in s { object(#{ index: o.index, reset: true }); }
}

// 5. gloss-band: roughness animated 0.0 -> 1.0 over 180 frames on one panel.
fn gloss_band(pass, every) {
    night_hall();
    let s = nearest(12.5, 20.0, 1);
    require(s.len() == 1, "the scene has a panel");
    let ix = s[0].index;
    camera(#{ position: [12.5, 1.5, 22.5], look_at: [12.5, 0.5, 20.0] });
    object(#{ index: ix, roughness: 0.0, metallic: 1.0 });
    wait(120);
    let n = 180;
    if pass == "temporal" {
        gi_metrics(#{ reset: true });
        for i in 0..n {
            object(#{ index: ix, roughness: i / (n - 1.0) });
            capture(#{ path: `temporal/gloss-band/live-${pad(i)}.png` });
        }
        gi_metrics(#{ write: "gloss-band" });
    } else {
        let f = 0;
        while f < n {
            object(#{ index: ix, roughness: f / (n - 1.0) });
            wait(1);
            measure("gloss-band", f);
            f += every;
        }
    }
    object(#{ index: ix, reset: true });
}

gloss_sweep(PASS, REF_EVERY);
gloss_edge(PASS, HIST, REF_EVERY);
gloss_offscreen(PASS, REF_EVERY);
gloss_switch(PASS);
gloss_band(PASS, REF_EVERY);
log(`lighting_oracle_spec ${PASS} ${PROFILE} done`);
