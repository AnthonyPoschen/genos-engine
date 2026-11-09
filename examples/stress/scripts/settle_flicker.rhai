// Static-scene settle flicker and the camera-step snap.
//
// Anthony, 5 Nov 2026: with every dynamic off, the picture flickers while the
// light is still settling, then holds. One camera step then snaps between that
// settled picture and the moving picture.
//
// This script records that. It does not change a limit. High and Medium
// default the four bits on. The ADR 0039 path is the three settings
// surface_cache.sh_texel_detail, surface_cache.sh_texel_ray_cut, and
// surface_cache.sh_probe_retire. They do nothing unless surface_cache.sh_cells
// is also on, so a run that turns the three on also turns the cells on.
// Launch with the four GENOS_LIGHTING_SURFACE_CACHE_SH_* mirrors set to 1 for
// the on path, and with none of them set for the off path. Those mirrors write
// the same settings keys from the first frame. This script writes those keys
// again so the resolved text matches the path under test.
//
// The scene is examples/stress/scenes/room.rhai: one lamp, nothing moves.
// Start the camera at the pose below (--eye) so the frames before this script
// are already there. The step is +3 degrees of yaw, one pose, then hold.
//
// METRIC lines are the record. mean_delta is the mean per-pixel linear
// luminance change between frames. The snap is the first frame at the new pose
// against the picture settled at that same pose. compare is that pair against
// one CPU path trace of the new pose when the trace returns.

time(#{ fixed_dt: 1.0 / 60.0 });
knob("time_of_day", 12.0);
knob("sun", "freeze");
knob("boxes", "still");
knob("dynamic", 0.0);
knob("lamps", "still");

let eye = [0.664, 1.699, 9.629];
let pitch = -0.12;
let yaw0 = -0.679;
camera(#{ position: eye, yaw: yaw0, pitch: pitch });

fn flag_on(text, key) {
    text.contains(`${key}=1`)
}

let before = get_lighting_settings();
let path_on = flag_on(before.text, "surface_cache.sh_texel_detail") || flag_on(before.text, "surface_cache.sh_texel_ray_cut") || flag_on(before.text, "surface_cache.sh_probe_retire") || flag_on(before.text, "surface_cache.sh_cells");
let bit = if path_on { "on" } else { "off" };
set_lighting_settings(#{
    text: `surface_cache.sh_cells=${bit}
surface_cache.sh_texel_detail=${bit}
surface_cache.sh_texel_ray_cut=${bit}
surface_cache.sh_probe_retire=${bit}`
});
let settings = get_lighting_settings();
let cells = flag_on(settings.text, "surface_cache.sh_cells");
let detail = flag_on(settings.text, "surface_cache.sh_texel_detail");
let cut = flag_on(settings.text, "surface_cache.sh_texel_ray_cut");
let retire = flag_on(settings.text, "surface_cache.sh_probe_retire");
let path = if detail && cut && retire && cells { "adr0039" } else { "legacy" };
log(`PATH ${path} sh_cells=${cells} sh_texel_detail=${detail} sh_texel_ray_cut=${cut} sh_probe_retire=${retire}`);
require((detail && cut && retire && cells) || (!detail && !cut && !retire && !cells), "the four settings must be all on or all off");
save_text("resolved.txt", settings.text);

view("full");
let rows = "";

fn remember(rows, line) {
    log(line);
    `${rows}${line}
`
}

let i = 0;
let calm = false;
while i < 18 && !calm {
    let f = flicker(#{ frames: 8, threshold: 0.004 });
    let st = status();
    let n = f.means.len();
    let mean0 = if n > 0 { f.means[0] } else { 0.0 };
    let mean1 = if n > 0 { f.means[n - 1] } else { 0.0 };
    rows = remember(rows, `METRIC path=${path} window=settle${i} mean_delta=${f.mean_delta} max_delta=${f.max_delta} popped_share=${f.popped_share} mean_range=${f.mean_range} mean0=${mean0} mean1=${mean1} stable=${st.stable} picture_settled=${st.picture_settled} still_frames=${st.still_frames} frame=${st.frame}`);
    if i == 0 {
        let pace = timing(#{ frames: 8 });
        rows = remember(rows, `METRIC path=${path} pace frame_ms=${pace.frame_ms} fps=${pace.fps}`);
    }
    calm = st.stable == true && st.picture_settled == true;
    i += 1;
}

let settled_a = wait_settled(#{ timeout_frames: 180, then_frames: 1, path: "settled-a.png", width: 160, height: 90 });
rows = remember(rows, `METRIC path=${path} window=settled_a settled=${settled_a.settled} frames=${settled_a.frames} still_frames=${settled_a.still_frames} frame=${settled_a.frame}`);
let held = flicker(#{ frames: 16, threshold: 0.004 });
let hn = held.means.len();
rows = remember(rows, `METRIC path=${path} window=held mean_delta=${held.mean_delta} max_delta=${held.max_delta} popped_share=${held.popped_share} mean_range=${held.mean_range} mean0=${if hn > 0 { held.means[0] } else { 0.0 }} mean1=${if hn > 0 { held.means[hn - 1] } else { 0.0 }}`);

let step_yaw = yaw0 + 3.0 * 3.141592653589793 / 180.0;
camera(#{ position: eye, yaw: step_yaw, pitch: pitch });
let step0 = capture(#{ name: "step0", path: "step0.png", width: 160, height: 90 });
rows = remember(rows, `METRIC path=${path} window=step0 frame=${step0.frame} luminance=${step0.luminance}`);
let early = flicker(#{ frames: 8, threshold: 0.004, frames_dir: "step-frames" });
let en = early.means.len();
rows = remember(rows, `METRIC path=${path} window=step_early mean_delta=${early.mean_delta} max_delta=${early.max_delta} popped_share=${early.popped_share} mean_range=${early.mean_range} mean0=${if en > 0 { early.means[0] } else { 0.0 }} mean1=${if en > 0 { early.means[en - 1] } else { 0.0 }}`);

let settled_b = wait_settled(#{ timeout_frames: 180, then_frames: 1, path: "settled-b.png", width: 160, height: 90 });
rows = remember(rows, `METRIC path=${path} window=settled_b settled=${settled_b.settled} frames=${settled_b.frames} still_frames=${settled_b.still_frames} frame=${settled_b.frame}`);
let settled_cap = capture(#{ name: "settled_b", path: "settled-b-named.png", width: 160, height: 90 });
rows = remember(rows, `METRIC path=${path} window=settled_b_cap frame=${settled_cap.frame} luminance=${settled_cap.luminance}`);
let snap = diff(#{ a: "step0", b: "settled_b", threshold: 8, out: "step-vs-settled.png" });
rows = remember(rows, `METRIC path=${path} snap mean_abs=${snap.mean_abs} max_abs=${snap.max_abs} over_share=${snap.over_share} lum_step=${snap.luminance_a} lum_settled=${snap.luminance_b} mean_luminance_delta=${snap.mean_luminance_delta}`);

try {
    let settled_ref = compare(#{
        live: "settled_b",
        name: "pose-b",
        prefix: "pose-b-settled",
        spp: 4,
        max_spp: 16,
        noise: 0.08,
        seconds: 60.0,
        width: 160,
        height: 90,
        bounces: 4,
        block: 4
    });
    rows = remember(rows, `METRIC path=${path} ref_settled mean_rel=${settled_ref.mean_rel} mean_abs=${settled_ref.mean_abs} bias=${settled_ref.bias} ref_mean=${settled_ref.ref_mean} live_mean=${settled_ref.live_mean} spp=${settled_ref.spp} noise=${settled_ref.noise}`);
    let step_ref = compare(#{
        live: "step0",
        reference: "pose-b",
        prefix: "pose-b-step",
        width: 160,
        height: 90,
        block: 4
    });
    rows = remember(rows, `METRIC path=${path} ref_step mean_rel=${step_ref.mean_rel} mean_abs=${step_ref.mean_abs} bias=${step_ref.bias} ref_mean=${step_ref.ref_mean} live_mean=${step_ref.live_mean}`);
} catch (err) {
    rows = remember(rows, `METRIC path=${path} ref_error ${err}`);
}

save_text("metrics.txt", rows);
log(`METRIC path=${path} done`);
