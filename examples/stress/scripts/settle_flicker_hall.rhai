// Hall bounce-view companion to settle_flicker.rhai.
// High and Medium default the four ADR 0039 bits on.
// Same static rules (dynamics off, boxes still, sun frozen) at the stress hall
// pose. The picture is the bounce view, so a direct lamp cannot hide a GI snap.
// No path trace: the room trace at 16 spp was noise 0.48 and did not separate
// the step from the settled picture.

time(#{ fixed_dt: 1.0 / 60.0 });
knob("time_of_day", 12.0);
knob("sun", "freeze");
knob("boxes", "still");
knob("dynamic", 0.0);
knob("lamps", "still");

let eye = [14.5, 1.7, 23.5];
let pitch = 0.08;
let yaw0 = -0.3554;
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
log(`PATH ${path} view=bounce sh_cells=${cells} sh_texel_detail=${detail} sh_texel_ray_cut=${cut} sh_probe_retire=${retire}`);
require((detail && cut && retire && cells) || (!detail && !cut && !retire && !cells), "the four settings must be all on or all off");

view("bounce");
let rows = "";
fn remember(rows, line) {
    log(line);
    `${rows}${line}
`
}

// Legacy hall settle0 column. A window through the lamp ring (frame ~120) uses
// the same ceiling. wait_settled does not skip that span.
let legacy_settle_max = 0.172;
let legacy_settle_popped = 0.860;
let legacy_step_max = 0.028;
let legacy_step_popped = 0.0078;
let legacy_snap_max = 29.0;

let i = 0;
let calm = false;
while i < 10 && !calm {
    let f = flicker(#{ frames: 8, threshold: 0.004 });
    let st = status();
    rows = remember(rows, `METRIC path=${path} view=bounce window=settle${i} mean_delta=${f.mean_delta} max_delta=${f.max_delta} popped_share=${f.popped_share} mean_range=${f.mean_range} stable=${st.stable} picture_settled=${st.picture_settled} still_frames=${st.still_frames} frame=${st.frame}`);
    if st.frame >= 100 {
        require(f.max_delta <= legacy_settle_max, `settle${i} max ${f.max_delta} above legacy ${legacy_settle_max}`);
        require(f.popped_share <= legacy_settle_popped, `settle${i} popped ${f.popped_share} above legacy ${legacy_settle_popped}`);
    }
    calm = st.stable == true && st.picture_settled == true;
    i += 1;
}
let ring = 0;
while status().frame < 160 && ring < 24 {
    let f = flicker(#{ frames: 8, threshold: 0.004 });
    let st = status();
    rows = remember(rows, `METRIC path=${path} view=bounce window=ring${ring} mean_delta=${f.mean_delta} max_delta=${f.max_delta} popped_share=${f.popped_share} mean_range=${f.mean_range} frame=${st.frame}`);
    if st.frame >= 100 {
        require(f.max_delta <= legacy_settle_max, `ring${ring} max ${f.max_delta} above legacy ${legacy_settle_max}`);
        require(f.popped_share <= legacy_settle_popped, `ring${ring} popped ${f.popped_share} above legacy ${legacy_settle_popped}`);
    }
    ring += 1;
}
let settled_a = wait_settled(#{ timeout_frames: 150, then_frames: 1, path: "settled-a.png", width: 160, height: 90 });
rows = remember(rows, `METRIC path=${path} view=bounce window=settled_a settled=${settled_a.settled} frames=${settled_a.frames} still_frames=${settled_a.still_frames}`);
let held = flicker(#{ frames: 8, threshold: 0.004 });
rows = remember(rows, `METRIC path=${path} view=bounce window=held mean_delta=${held.mean_delta} max_delta=${held.max_delta} popped_share=${held.popped_share} mean_range=${held.mean_range}`);

let step_yaw = yaw0 + 3.0 * 3.141592653589793 / 180.0;
camera(#{ position: eye, yaw: step_yaw, pitch: pitch });
let step0 = capture(#{ name: "step0", path: "step0.png", width: 160, height: 90 });
rows = remember(rows, `METRIC path=${path} view=bounce window=step0 frame=${step0.frame} luminance=${step0.luminance}`);
let early = flicker(#{ frames: 6, threshold: 0.004 });
rows = remember(rows, `METRIC path=${path} view=bounce window=step_early mean_delta=${early.mean_delta} max_delta=${early.max_delta} popped_share=${early.popped_share} mean_range=${early.mean_range}`);
require(early.max_delta <= legacy_step_max, `step max ${early.max_delta} above legacy ${legacy_step_max}`);
require(early.popped_share <= legacy_step_popped, `step popped ${early.popped_share} above legacy ${legacy_step_popped}`);
// sc_settle on High is 300 still frames. A 120-frame wait ends before that
// flag can become true. The picture gates are unchanged.
let settled_b = wait_settled(#{ timeout_frames: 360, then_frames: 1, path: "settled-b.png", width: 160, height: 90 });
rows = remember(rows, `METRIC path=${path} view=bounce window=settled_b settled=${settled_b.settled} frames=${settled_b.frames} still_frames=${settled_b.still_frames}`);
capture(#{ name: "settled_b", path: "settled-b-named.png", width: 160, height: 90 });
let snap = diff(#{ a: "step0", b: "settled_b", threshold: 8, out: "step-vs-settled.png" });
rows = remember(rows, `METRIC path=${path} view=bounce snap mean_abs=${snap.mean_abs} max_abs=${snap.max_abs} over_share=${snap.over_share} lum_step=${snap.luminance_a} lum_settled=${snap.luminance_b} mean_luminance_delta=${snap.mean_luminance_delta}`);
require(snap.max_abs <= legacy_snap_max, `snap max ${snap.max_abs} above legacy ${legacy_snap_max}`);
require(settled_a.settled == true, "pose A did not settle");
require(settled_b.settled == true, "pose B did not settle");
save_text("metrics.txt", rows);
