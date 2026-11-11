// Age of the shown mean through lamp 3's move (stage F1).
// Same pose and move as fill_curve.rhai. Stops at the short hold so a
// lavapipe run stays bounded. GENOS_LIGHTING_SC_TRACE=1 prints lightingage.

time(#{ fixed_dt: 1.0 / 60.0 });
knob("sun", "freeze");
knob("boxes", "still");
knob("lamps", "still");
knob("dynamic", 0.0);
knob("time_of_day", 8.88);
camera(#{ position: [14.5, 1.7, 23.5], yaw: -0.3554, pitch: 0.08 });

fn pad(i) {
    let n = i.to_int();
    if n < 10 { `000${n}` } else if n < 100 { `00${n}` } else if n < 1000 { `0${n}` } else { `${n}` }
}

fn shot(tag, modes) {
    let f = status().frame;
    let name = `${tag}-f${pad(f)}`;
    log(`MARK ${tag} frame=${f}`);
    captures(#{ modes: modes, prefix: name });
    view("full");
}

fn reach(n) {
    while status().frame < n {
        wait(1);
    }
}

reach(5);
shot("albedo", ["albedo"]);
reach(40);
shot("still", ["full", "depth"]);
reach(80);
shot("still", ["full", "depth"]);
reach(160);
shot("still", ["full", "depth"]);

let origin = [12.5, 2.6, 21.5];
let dest = [14.0, 2.6, 22.5];
let best = -1;
let bestd = 1000000000000.0;
for l in lights() {
    if l.directional { continue; }
    let p = l.position;
    let dx = p[0] - origin[0];
    let dy = p[1] - origin[1];
    let dz = p[2] - origin[2];
    let d = dx * dx + dy * dy + dz * dz;
    if d < bestd {
        bestd = d;
        best = l.index;
    }
}
require(best >= 0, "lamp 3 is in the hall");
log(`LAMP3 index=${best} dist2=${bestd} frame=${status().frame}`);
let move_at = status().frame;
light(#{ index: best, path: [origin, dest], frames: 40 });
log(`MARK move_start frame=${move_at}`);

reach(move_at + 20);
shot("mid", ["full", "depth"]);
reach(move_at + 40);
shot("stop", ["full", "depth"]);
reach(move_at + 80);
shot("after", ["full", "depth"]);
log(`MARK done frame=${status().frame}`);
