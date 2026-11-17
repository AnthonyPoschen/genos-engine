// A lamp dim must land on the scaled bounce. The 2% field hold left a 1.5%
// dim parked above its target. Both dims are graded through the new lamp ring
// (about 120 frames). The absolute error is the held window's mean range,
// and never looser than the readout floor below. That floor is under the
// parked error this script exists to catch (about 0.002 on the hall bounce).

time(#{ fixed_dt: 1.0 / 60.0 });
knob("time_of_day", 12.0);
knob("sun", "freeze");
knob("boxes", "still");
knob("dynamic", 0.0);
knob("lamps", "still");
knob("power", 1.0);

let eye = [14.5, 1.7, 23.5];
let pitch = 0.08;
let yaw0 = -0.3554;
camera(#{ position: eye, yaw: yaw0, pitch: pitch });

set_lighting_settings(#{
    text: `surface_cache.sh_cells=on
surface_cache.sh_texel_detail=on
surface_cache.sh_texel_ray_cut=on
surface_cache.sh_probe_retire=on`
});
view("bounce");

let rows = "";
fn remember(rows, line) {
    log(line);
    `${rows}${line}
`
}

let readout_floor = 0.0005;

fn walk_ring(rows, label) {
    let ring = 0;
    let start = status().frame;
    while status().frame < start + 160 && ring < 24 {
        let f = flicker(#{ frames: 8, threshold: 0.004 });
        let st = status();
        rows = remember(rows, `METRIC dim window=${label}${ring} mean_delta=${f.mean_delta} max_delta=${f.max_delta} popped_share=${f.popped_share} mean_range=${f.mean_range} frame=${st.frame}`);
        ring += 1;
    }
    rows
}

// The hall bounce is still filling at frame 160. The base is the settled
// picture, so a dim is not hidden under that fill. sc_settle on High is 300.
// wait_settled itself gives up after about 120 s of wall time, so the fill is
// walked in short windows first.
let guard = 0;
while status().frame < 340 && guard < 45 {
    let f = flicker(#{ frames: 8, threshold: 0.004 });
    let st = status();
    rows = remember(rows, `METRIC dim window=open${guard} mean_delta=${f.mean_delta} max_delta=${f.max_delta} popped_share=${f.popped_share} mean_range=${f.mean_range} frame=${st.frame}`);
    guard += 1;
}
let opened = wait_settled(#{ timeout_frames: 80, then_frames: 1, path: "opened.png", width: 160, height: 90 });
rows = remember(rows, `METRIC dim window=opened settled=${opened.settled} frames=${opened.frames} still_frames=${opened.still_frames}`);
require(opened.settled == true, "the dim base did not settle");
let base = capture(#{ name: "base", path: "base.png", width: 160, height: 90 });
let held = flicker(#{ frames: 8, threshold: 0.004 });
rows = remember(rows, `METRIC dim window=base luminance=${base.luminance} mean_range=${held.mean_range} frame=${base.frame}`);
let noise = held.mean_range;
if noise < readout_floor {
    noise = readout_floor;
}

fn land(rows, label, scale, base_l, noise) {
    knob("power", scale);
    let applied = knob("power");
    rows = remember(rows, `METRIC dim window=${label} power=${applied}`);
    require(applied == scale, `${label} power snapped to ${applied}, wanted ${scale}`);
    rows = walk_ring(rows, label);
    let cap = capture(#{ name: label, path: `${label}.png`, width: 160, height: 90 });
    let target = base_l * scale;
    let err = if cap.luminance > target { cap.luminance - target } else { target - cap.luminance };
    rows = remember(rows, `METRIC dim window=${label} luminance=${cap.luminance} target=${target} abs_err=${err} noise=${noise} frame=${cap.frame}`);
    require(err <= noise, `${label} landed ${cap.luminance}, target ${target}, error ${err} above noise ${noise}`);
    rows
}

rows = land(rows, "dim985", 0.985, base.luminance, noise);
knob("power", 1.0);
rows = walk_ring(rows, "restore");
let full = capture(#{ name: "full", path: "full.png", width: 160, height: 90 });
rows = remember(rows, `METRIC dim window=full luminance=${full.luminance} frame=${full.frame}`);
rows = land(rows, "dim90", 0.90, full.luminance, noise);
save_text("metrics.txt", rows);
