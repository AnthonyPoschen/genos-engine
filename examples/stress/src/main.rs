#![expect(clippy::as_conversions, reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md")]
#![expect(clippy::cast_lossless, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_possible_truncation, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::cast_precision_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::disallowed_methods, reason = "TODO(lint-2026-10-14): publish intent with the CLI argument, not set_var. docs/LINT_RULES.md")]
#![expect(clippy::map_unwrap_or, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::redundant_closure, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::semicolon_if_nothing_returned, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::similar_names, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::struct_excessive_bools, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::too_many_lines, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![cfg_attr(test, expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails"))]
#![expect(clippy::wildcard_enum_match_arm, reason = "TODO(lint-2026-10-14): pre-existing wildcard match. docs/LINT_RULES.md")]

//! Lighting stress test: a big, fairly open building with windows, skylights and wide
//! doorways, still and moving lamps, a sun on a day/night cycle, and coloured boxes
//! that slide and spin. `--scale small|big` tiles one 25 m section 1 or 16 times so the
//! cost of size can be read as a delta; `--bench` sweeps lamp count × dynamic share ×
//! scale from a fixed viewpoint on a fixed timeline.
//!
//! Every option also reads an environment variable (`GENOS_STRESS_SCALE`,
//! `GENOS_STRESS_LIGHTS`, `GENOS_STRESS_DYNAMIC`, `GENOS_STRESS_LAYOUT`,
//! `GENOS_STRESS_DAY_SECONDS`, `GENOS_STRESS_STEP`, `GENOS_STRESS_FIXED_DT`); a flag
//! wins over the variable. Lighting launch flags (`--preset`, `--bounces`,
//! `--ambient`, `--time-of-day`, `--gpu-times`) win the same way: each
//! `GENOS_LIGHTING_*` or `GENOS_GPU_TIMES` mirror applies only when that
//! argument was omitted. The panel switches lamps, dynamic share, scale and the sun
//! live.

mod bench;
mod building;
mod gi_check;
mod knobs;
mod stage;

use std::path::PathBuf;
use std::time::Instant;

use genos_debug::{png, Host};
use genos_input::{character_controller, debug_keys, InputCode, InputSystem};
use genos_render::{Renderer, ScreenLine, ScreenRect, World};
use genos_scene::{
    look_direction, update, Actions, Camera, DebugSettings, EventLoopSettings, Floor,
    FpsCounterSettings, LightingPreset, LightingSettings, MenuSettings, Scene, Sky, Vec3,
    PITCH_LIMIT,
};
use genos_ui::{button_panel, Action, PanelButton, PanelRow, PanelSlider, Pointer, State};
use genos_window::{extent_changed, FocusGate, Window};

use building::{Layout, LightMix, View};
use stage::{Scale, Stage};

const USAGE: &str = "genos-stress [options]
genos-stress noise-gate [--size WxH] [--settle N] [--frames N] [--preset NAME] [--out DIR]
                          s7 direct-light noise grade. noise-gate --help lists it.
  --scale small|big|N|NxM   building size: small = 1 section (25 m), big = 4x4 (100 m)
  --lights N                lamps in the whole building (panel: 5/25/50/100)
  --dynamic PCT             share of lamps that move, 0-100 (panel: 0/25/50/75/100)
  --layout spread|first     lamps spread over every section, or all in section 0
                            where the benchmark camera stands (default spread;
                            first for --bench, so every scale sees the same lamps)
  --power X                 lamp total multiplier (default 1)
  --day-seconds S           seconds per day/night cycle (default 120)
  --time F                  start time as a fraction of a day: 0 sunrise, 0.25 noon,
                            0.5 sunset (default 0.12)
  --time-of-day H|HH:MM     start the lighting clock in hours: 0 midnight, 6 dawn,
                            12 noon, 18 dusk (06:00, 18:24). Same start as --time.
                            GENOS_LIGHTING_TIME holds that hour only when this
                            argument was omitted
  --sun-speed X             day clock multiplier (default 1)
  --freeze-sun              hold the sun at --time or --time-of-day
  --no-sky                  no sky light: only the sun and the lamps
  --scene FILE              a scene file (genos-camera's .rhai) in place of the building; nothing moves
  --lamp-stop               with --scene, step the point lamp then hold it
                            (examples/stress/scenes/bounce_corner.rhai)
  --still-boxes             hold the coloured boxes
  --seed N                  lamp and box layout seed (default 1)
  --view hall|roomA|sky     start (and benchmark) viewpoint (default hall).
                            sky stands in the south yard and looks up, so the
                            ambient curve can be judged
  --eye X Y Z YAW PITCH     start the camera here instead of --view (free look)
  --size WxH                window size (default 1280x720; 2560x1440 for --bench)
  --proof                   open the proof window (floats, takes no focus; --bench
                            always uses it, so the compositor keeps its size)
  --display NAME            Wayland display name. Omit it to use the session compositor.
  --frames N                quit after N frames; frames step 1/60 s of scene time
  --fixed-dt SECONDS        scene step per frame (wall clock otherwise; 1/60 with --step)
  --step                    pause; Space or the MCP step command advances one fixed step
                            U toggles the unfilled count. Shift-U toggles the tint.
                            F8 and shift-F8 are the same keys. GENOS_DEBUG_KEYS replaces them.
  --shot PATH               settle the light on the last frame and write a PNG
                            without the panel, the menu tab, the frame graph,
                            or tool overlays
  --ui                      keep those overlays in that PNG and in an MCP shot
  --no-panel                hide the panel
  --bench SECONDS           sweep and print tables; per configuration seconds
  --warmup FRAMES           benchmark warm-up frames per configuration (default 30)
  --sweep-lights LIST       benchmark lamp counts (default 5,25,50,100)
  --sweep-dynamic LIST      benchmark dynamic shares (default 0,25,100)
  --sweep-scales LIST       benchmark scales (default small,big)
  --bench-out PATH          also write the tables to PATH
  --gltf FILE               place a glTF 2.0 model (.gltf/.glb) in the building (mean
                            material colour; its SDF is baked on first load for the GI
                            tracer, the current lighting does not see it); repeatable
  --gltf-at X Y Z SCALE     where the following --gltf models go (default 0 0 0 1)
  --script FILE             run a rhai debug script (docs/systems/debugging.md); scene time
                            steps 1/60 s a frame; exit 0 when every check passes
  --report DIR              where the script writes report.md, result.json and pictures
                            (default target/debug-reports/<script name>)
  --out DIR                 unfilled events (default ./genos-debug/unfilled)
  --unfilled[=0|1]          count unfilled pixels (GENOS_LIGHTING_UNFILLED is the mirror)
  --unfilled-tint[=0|1]     magenta tint; turns the count on (GENOS_LIGHTING_UNFILLED_TINT)
  --unfilled-dump PERCENT   save when the share exceeds PERCENT; 0 saves any
                            (GENOS_LIGHTING_UNFILLED_DUMP)
  --unfilled-max-age-secs N drop event folders older than N seconds (default 172800)
                            (GENOS_UNFILLED_MAX_AGE_SECS)
  --unfilled-max-folders N  keep at most N event folders (default 200)
                            (GENOS_UNFILLED_MAX_FOLDERS)
  --unfilled-max-bytes N    keep at most N bytes of events (default 524288000)
                            (GENOS_UNFILLED_MAX_BYTES)
  --unfilled-session-cap N  saves in one process (default 32)
                            (GENOS_UNFILLED_SESSION_CAP)
  --trace                   capture the picture after every script command that changes it
  --preset NAME             potato, day-cycle, gi-1bounce, low, medium, high
                            (GENOS_LIGHTING_PRESET is the mirror)
  --surface-cache-attach-movers on|off
                            store mover cache entries in the object's frame
                            (GENOS_LIGHTING_SURFACE_CACHE_ATTACH_MOVERS;
                            GENOS_LIGHTING_SC_LOCAL is the previous mirror,
                            used only when this is omitted)
  --surface-cache-blend msme|hold
                            published blend (GENOS_LIGHTING_SURFACE_CACHE_BLEND).
                            msme is MultiscaleMeanEstimator.hlsl; hold is the
                            older snap path
  --surface-cache-firefly-sigma F
                            MSME firefly clamp in sigmas, default 8, biased
                            (GENOS_LIGHTING_SURFACE_CACHE_FIREFLY_SIGMA).
                            0 keeps the 0.1 floor
  --surface-cache-ray-budget N
                            cap on cache-round rays; 0 keeps the existing lists
                            (GENOS_LIGHTING_SURFACE_CACHE_RAY_BUDGET)
  --surface-cache-dormant-rays N
                            fresh rays per frame for a dormant cache entry, 0..16.
                            0 keeps one sweep in eight. 1..16 traces one new
                            16-ray entry every ceil(16 / N) frames
                            (GENOS_LIGHTING_SURFACE_CACHE_DORMANT_RAYS)
  --bounces 0|1|2|unlimited bounce count (GENOS_LIGHTING_BOUNCES)
  --ambient MODE            off, constant, hemisphere, day, or curve
                            (GENOS_LIGHTING_AMBIENT)
  --denoise-iters 1|3|5     wavelet iterations (GENOS_LIGHTING_DENOISE_ITERS)
  --denoise-adaptive on|off experimental: drop settled wavelet iterations
                            (GENOS_LIGHTING_DENOISE_ADAPTIVE). Off on every preset
  --denoise-variance F      relative luminance deviation, 0..0.1 (GENOS_LIGHTING_DENOISE_VARIANCE)
  --denoise-launch hash|screen
                            picture wavelet (GENOS_LIGHTING_DENOISE_LAUNCH). Preset high
                            uses screen. The other presets keep hash. This flag and the
                            variable override the preset
  --denoise-screen on|off   alias of --denoise-launch (on is screen, off is hash)
  --gpu-times N             print GPU times every N frames (GENOS_GPU_TIMES)
  --lamp-pick MODE          off, hits, pixels, both, or reuse
                            (GENOS_LIGHTING_LAMP_PICK; reuse is the same mode)
  --lamp-pick-samples N     shadow rays per shaded point, 1..4 (GENOS_LIGHTING_LAMP_PICK_SAMPLES)
  --lamp-pick-rays N        same knob as --lamp-pick-samples
  --lamp-pick-reuse on|off  temporal reservoir (GENOS_LIGHTING_LAMP_PICK_REUSE)
  --lamp-pick-candidates N  unshadowed reservoir candidates, 1..32
                            (GENOS_LIGHTING_LAMP_PICK_CANDIDATES)
  --lamp-pick-radius N      spatial radius in pixels, 0..64; 0 is temporal only
                            (GENOS_LIGHTING_LAMP_PICK_RADIUS)
  --lamp-pick-history N     biased-merge M cap, 1..64 (GENOS_LIGHTING_LAMP_PICK_HISTORY)
  --lamp-pick-unbiased on|off  pairwise MIS when on (GENOS_LIGHTING_LAMP_PICK_UNBIASED)
  --lighting-full-effort    full refresh every frame (GENOS_LIGHTING_FULL_EFFORT=1)
  --lighting-sun on|off     small sun step (GENOS_LIGHTING_SUN, default on)
  --lighting-sun-bands N    indirect bands, 1..64 (GENOS_LIGHTING_SUN_BANDS, default 8)
  --lighting-catch-up N     kept-hit frames after a point lamp stops, 0..16
                            (GENOS_LIGHTING_CATCH_UP, default 4)
  --lighting-debug          read lighting counters (GENOS_LIGHTING_DEBUG).
                            Not an fps run: it reads the GPU back every frame";

const LIGHT_STEPS: [usize; 4] = [5, 25, 50, 100];
/// Panel ids. Each row's buttons count up from its base.
const ID_SCALE: u32 = 300;
const ID_SUN: u32 = 400;
const ID_BOXES: u32 = 500;
const ID_LAYOUT: u32 = 600;
const ID_SKY: u32 = 700;
const ID_OUT: u32 = 800;
/// Sliders, one per knob.
const ID_KNOB: u32 = 1000;
const PANEL_KNOBS: [&str; 6] = [
    "time_of_day",
    "sun_speed",
    "lights",
    "dynamic",
    "unfilled",
    "unfilled_tint",
];
/// A hitch longer than this does not replay the missed time.
const MAX_FRAME_SECONDS: f32 = 0.25;

struct Options {
    scale: Scale,
    lights: usize,
    dynamic: u32,
    layout: Layout,
    power: f32,
    day_seconds: f32,
    time: f32,
    sun_speed: f32,
    freeze_sun: bool,
    no_sky: bool,
    scene: Option<PathBuf>,
    lamp_stop: bool,
    still_boxes: bool,
    seed: u64,
    view: View,
    eye: Option<[f32; 5]>,
    size: (u32, u32),
    proof: bool,
    /// Wayland display name from `--display`. Empty uses the session compositor.
    display: Option<String>,
    frames: Option<u32>,
    /// Pause and advance one fixed step on Space or MCP `step`.
    step: bool,
    /// Scene seconds per frame. Unset keeps wall-clock time, except `--frames` and `--shot`.
    fixed_dt: Option<f32>,
    shot: Option<PathBuf>,
    /// Keep the panel, menu tab, frame graph, and tool overlays in a shot.
    ui: bool,
    panel: bool,
    bench: Option<bench::Plan>,
    bench_out: Option<PathBuf>,
    script: Option<genos_debug::script::ScriptRun>,
    /// glTF files to place in the building: path, position and uniform scale.
    gltf: Vec<(PathBuf, [f32; 4])>,
    /// Set only when the flag was passed. Each wins over the matching GENOS_* variable.
    preset: Option<String>,
    denoise_iters: Option<String>,
    denoise_adaptive: Option<String>,
    denoise_variance: Option<String>,
    denoise_launch: Option<String>,
    denoise_screen: Option<String>,
    gpu_times: Option<String>,
    lamp_pick: Option<String>,
    lamp_pick_samples: Option<String>,
    lamp_pick_rays: Option<String>,
    lamp_pick_reuse: Option<String>,
    lamp_pick_candidates: Option<String>,
    lamp_pick_radius: Option<String>,
    lamp_pick_history: Option<String>,
    lamp_pick_unbiased: Option<String>,
}

fn parse<T: std::str::FromStr>(name: &str, text: &str) -> Result<T, String> {
    text.parse()
        .map_err(|_| format!("bad value {text} for {name}"))
}

fn list<T: std::str::FromStr>(name: &str, text: &str) -> Result<Vec<T>, String> {
    text.split(',')
        .map(|item| parse(name, item.trim()))
        .collect()
}

fn off_word(text: &str) -> bool {
    matches!(
        text.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "off" | "no"
    )
}

/// `--step` and `--fixed-dt` win over `GENOS_STRESS_STEP` and `GENOS_STRESS_FIXED_DT`.
fn time_control(
    step_arg: bool,
    fixed_arg: Option<f32>,
    step_env: Option<&str>,
    fixed_env: Option<&str>,
) -> Result<(bool, Option<f32>), String> {
    let step = if step_arg {
        true
    } else {
        match step_env.map(str::trim).filter(|text| !text.is_empty()) {
            Some(text) if off_word(text) => false,
            Some(_) => true,
            None => false,
        }
    };
    let fixed_dt = if let Some(dt) = fixed_arg {
        Some(dt)
    } else if let Some(text) = fixed_env.map(str::trim).filter(|text| !text.is_empty()) {
        Some(parse("--fixed-dt", text)?)
    } else {
        None
    };
    if let Some(dt) = fixed_dt {
        if !dt.is_finite() || dt <= 0.0 {
            return Err(format!("--fixed-dt must be greater than 0, got {dt}"));
        }
    }
    Ok((step, fixed_dt))
}

fn env<T: std::str::FromStr>(name: &str) -> Result<Option<T>, String> {
    match std::env::var(name) {
        Ok(text) => parse(name, &text).map(Some),
        Err(_) => Ok(None),
    }
}

fn options() -> Result<Options, String> {
    let mut opts = Options {
        scale: match std::env::var("GENOS_STRESS_SCALE") {
            Ok(text) => Scale::parse(&text)?,
            Err(_) => Scale::SMALL,
        },
        lights: env("GENOS_STRESS_LIGHTS")?.unwrap_or(5),
        dynamic: env("GENOS_STRESS_DYNAMIC")?.unwrap_or(50),
        layout: match std::env::var("GENOS_STRESS_LAYOUT") {
            Ok(text) => Layout::parse(&text).ok_or(format!("unknown layout {text}"))?,
            Err(_) => Layout::Spread,
        },
        power: 1.0,
        day_seconds: env("GENOS_STRESS_DAY_SECONDS")?.unwrap_or(120.0),
        time: 0.12,
        sun_speed: 1.0,
        freeze_sun: false,
        no_sky: false,
        scene: None,
        lamp_stop: false,
        still_boxes: false,
        seed: 1,
        view: View::Hall,
        eye: None,
        size: (1280, 720),
        proof: false,
        display: None,
        frames: None,
        step: false,
        fixed_dt: None,
        shot: None,
        ui: false,
        panel: true,
        bench: None,
        bench_out: None,
        script: None,
        gltf: Vec::new(),
        preset: None,
        denoise_iters: None,
        denoise_adaptive: None,
        denoise_variance: None,
        denoise_launch: None,
        denoise_screen: None,
        gpu_times: None,
        lamp_pick: None,
        lamp_pick_samples: None,
        lamp_pick_rays: None,
        lamp_pick_reuse: None,
        lamp_pick_candidates: None,
        lamp_pick_radius: None,
        lamp_pick_history: None,
        lamp_pick_unbiased: None,
    };
    let mut report: Option<PathBuf> = None;
    let mut trace = false;
    let mut seconds = None;
    let mut size_set = false;
    let mut layout_set = std::env::var("GENOS_STRESS_LAYOUT").is_ok();
    let mut warmup = 30;
    let mut sweep_lights = LIGHT_STEPS.to_vec();
    let mut sweep_dynamic = vec![0, 25, 100];
    let mut sweep_scales = vec![Scale::SMALL, Scale::BIG];
    let mut gltf_at = [0.0, 0.0, 0.0, 1.0];
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--scale" => opts.scale = Scale::parse(&value()?)?,
            "--lights" => opts.lights = parse(&arg, &value()?)?,
            "--dynamic" => opts.dynamic = parse(&arg, &value()?)?,
            "--layout" => {
                let text = value()?;
                opts.layout = Layout::parse(&text).ok_or(format!("unknown layout {text}"))?;
                layout_set = true;
            }
            "--power" => opts.power = parse(&arg, &value()?)?,
            "--day-seconds" => opts.day_seconds = parse(&arg, &value()?)?,
            "--time" => opts.time = parse(&arg, &value()?)?,
            "--sun-speed" => opts.sun_speed = parse(&arg, &value()?)?,
            "--freeze-sun" => opts.freeze_sun = true,
            "--no-sky" => opts.no_sky = true,
            "--scene" => opts.scene = Some(PathBuf::from(value()?)),
            "--lamp-stop" => opts.lamp_stop = true,
            "--still-boxes" => opts.still_boxes = true,
            "--seed" => opts.seed = parse(&arg, &value()?)?,
            "--view" => {
                let text = value()?;
                opts.view = View::parse(&text).ok_or(format!("unknown view {text}"))?;
            }
            "--eye" => {
                let mut eye = [0.0; 5];
                for slot in &mut eye {
                    *slot = parse(&arg, &value()?)?;
                }
                opts.eye = Some(eye);
            }
            "--size" => {
                let text = value()?;
                let (w, h) = text.split_once('x').ok_or(format!("bad size {text}"))?;
                opts.size = (parse(&arg, w)?, parse(&arg, h)?);
                size_set = true;
            }
            "--proof" => opts.proof = true,
            "--display" => opts.display = Some(value()?),
            "--frames" => opts.frames = Some(parse(&arg, &value()?)?),
            "--fixed-dt" => opts.fixed_dt = Some(parse(&arg, &value()?)?),
            "--step" => opts.step = true,
            "--shot" => opts.shot = Some(PathBuf::from(value()?)),
            "--ui" => opts.ui = true,
            "--no-panel" => opts.panel = false,
            "--bench" => seconds = Some(parse::<f32>(&arg, &value()?)?),
            "--warmup" => warmup = parse(&arg, &value()?)?,
            "--sweep-lights" => sweep_lights = list(&arg, &value()?)?,
            "--sweep-dynamic" => sweep_dynamic = list(&arg, &value()?)?,
            "--sweep-scales" => {
                sweep_scales = value()?
                    .split(',')
                    .map(|s| Scale::parse(s.trim()))
                    .collect::<Result<_, _>>()?
            }
            "--bench-out" => opts.bench_out = Some(PathBuf::from(value()?)),
            "--script" => {
                opts.script = Some(genos_debug::script::ScriptRun {
                    script: PathBuf::from(value()?),
                    report_dir: PathBuf::new(),
                    trace: false,
                })
            }
            "--gltf-at" => {
                for slot in &mut gltf_at {
                    *slot = parse(&arg, &value()?)?;
                }
            }
            "--gltf" => opts.gltf.push((PathBuf::from(value()?), gltf_at)),
            "--report" => report = Some(PathBuf::from(value()?)),
            "--out" => {
                // The renderer reads this from the process arguments.
                let _dir = value()?;
            }
            arg if arg.starts_with("--out=") => {
                if arg.trim_start_matches("--out=").trim().is_empty() {
                    return Err("--out needs a directory".into());
                }
            }
            "--trace" => trace = true,
            "--preset" => opts.preset = Some(value()?),
            "--denoise-iters" => opts.denoise_iters = Some(value()?),
            "--denoise-adaptive" => opts.denoise_adaptive = Some(value()?),
            "--denoise-variance" => opts.denoise_variance = Some(value()?),
            "--denoise-launch" => opts.denoise_launch = Some(value()?),
            "--denoise-screen" => opts.denoise_screen = Some(value()?),
            "--gpu-times" => opts.gpu_times = Some(value()?),
            "--lamp-pick" => opts.lamp_pick = Some(value()?),
            "--lamp-pick-samples" => opts.lamp_pick_samples = Some(value()?),
            "--lamp-pick-rays" => opts.lamp_pick_rays = Some(value()?),
            "--lamp-pick-reuse" => opts.lamp_pick_reuse = Some(value()?),
            "--lamp-pick-candidates" => opts.lamp_pick_candidates = Some(value()?),
            "--lamp-pick-radius" => opts.lamp_pick_radius = Some(value()?),
            "--lamp-pick-history" => opts.lamp_pick_history = Some(value()?),
            "--lamp-pick-unbiased" => opts.lamp_pick_unbiased = Some(value()?),
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => match genos_render::unfilled_flag_needs_value(other) {
                Some(true) => {
                    let next = value()?;
                    genos_render::check_unfilled_arg(other, Some(&next))?;
                }
                Some(false) => genos_render::check_unfilled_arg(other, None)?,
                None => match genos_scene::lighting_flag_needs_value(other) {
                    Some(needs) => {
                        let next = needs.then(|| value()).transpose()?;
                        genos_scene::check_lighting_arg(other, next.as_deref())?;
                        if let Some(hours) =
                            genos_scene::lighting_arg_time_of_day(other, next.as_deref())?
                        {
                            opts.time = knobs::day_from_hours(f64::from(hours));
                        }
                    }
                    None => match genos_render::gpu_times_flag_needs_value(other) {
                        Some(true) => {
                            let next = value()?;
                            genos_render::check_gpu_times_arg(other, Some(&next))?;
                        }
                        Some(false) => genos_render::check_gpu_times_arg(other, None)?,
                        None => match genos_render::lighting_sun_flag_needs_value(other) {
                            Some(true) => {
                                let next = value()?;
                                genos_render::check_lighting_sun_arg(other, Some(&next))?;
                            }
                            Some(false) => genos_render::check_lighting_sun_arg(other, None)?,
                            None => return Err(format!("unknown argument {other}\n{USAGE}")),
                        },
                    },
                },
            },
        }
    }
    opts.dynamic = opts.dynamic.min(100);
    let (step, fixed_dt) = time_control(
        opts.step,
        opts.fixed_dt,
        std::env::var("GENOS_STRESS_STEP").ok().as_deref(),
        std::env::var("GENOS_STRESS_FIXED_DT").ok().as_deref(),
    )?;
    opts.step = step;
    opts.fixed_dt = fixed_dt;
    if opts.lamp_stop && opts.scene.is_none() {
        return Err("--lamp-stop needs --scene examples/stress/scenes/bounce_corner.rhai".into());
    }
    if let Some(run) = &mut opts.script {
        let stem = run
            .script
            .file_stem()
            .map_or("script".into(), |s| s.to_string_lossy().to_string());
        run.report_dir = report.unwrap_or_else(|| PathBuf::from("target/debug-reports").join(stem));
        run.trace = trace;
    }
    if let Some(seconds) = seconds {
        // A benchmark draws at a fixed size in a window the compositor does not tile,
        // and with the same lamps near the camera at every scale.
        opts.proof = true;
        if !size_set {
            opts.size = (2560, 1440);
        }
        if !layout_set {
            opts.layout = Layout::First;
        }
        opts.bench = Some(bench::Plan {
            seconds,
            warmup_frames: warmup,
            lights: sweep_lights,
            dynamic: sweep_dynamic.into_iter().map(|d| d.min(100)).collect(),
            scales: sweep_scales,
            day: opts.time,
            layout: opts.layout,
            size: opts.size,
        });
    }
    Ok(opts)
}

fn main() {
    if let Err(err) = run() {
        eprintln!("genos-stress: {err}");
        std::process::exit(1);
    }
}

/// A fly camera at `view`. It has no colliders: the stress test flies through walls,
/// so a view or a bench pose can sit anywhere in the building.
fn camera_at(view: View) -> Camera {
    let (x, z, yaw, pitch) = view.pose();
    let mut camera = Camera::new(x, z, yaw);
    camera.pitch = pitch;
    camera
}

/// Fly speed in metres per second; Shift is four times faster.
const FLY_SPEED: f32 = 5.0;
/// Stick look in radians per second.
const STICK_LOOK: f32 = 2.0;

/// Move the fly camera: move keys along the view on the ground plane. Space goes up.
/// C, left Ctrl, and right Ctrl go down. Either Shift is four times faster. Mouse
/// look and capture go through [`update`] with no time step, which also skips its physics.
fn fly(camera: &mut Camera, scene: &mut Scene, input: &InputSystem, actions: &Actions, dt: f32) {
    update(camera, scene, actions, 0.0);
    camera.yaw += actions.look_x * STICK_LOOK * dt;
    camera.pitch =
        (camera.pitch + actions.look_y * STICK_LOOK * dt).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    let forward = look_direction(camera.yaw, 0.0);
    let right = look_direction(camera.yaw + std::f32::consts::FRAC_PI_2, 0.0);
    let mut lift = 0.0;
    if input.keyboard.down(InputCode::key_space) {
        lift += 1.0;
    }
    if input.keyboard.down(InputCode::key_c)
        || input.keyboard.down(InputCode::key_control_left)
        || input.keyboard.down(InputCode::key_control_right)
    {
        lift -= 1.0;
    }
    let boost = if input.keyboard.down(InputCode::key_shift_left)
        || input.keyboard.down(InputCode::key_shift_right)
    {
        4.0
    } else {
        1.0
    };
    let step = (forward * actions.forward + right * actions.strafe + Vec3::Y * lift)
        * (FLY_SPEED * boost * dt);
    let position = camera.position + step;
    camera.set_pose(position, camera.yaw, camera.pitch);
}

fn apply_lighting(stage: &mut Stage, patch: &genos_mcp::LightingSettings) {
    if let Some(value) = patch.sun_frozen {
        stage.sun_frozen = value;
    }
    if let Some(value) = patch.sky_on {
        stage.sky_on = value;
    }
    if let Some(value) = patch.boxes_still {
        stage.boxes_still = value;
    }
    if let Some(value) = patch.dynamic {
        stage.mix.dynamic_pct = value;
    }
    if let Some(value) = patch.day {
        stage.day = value;
    }
    stage.apply();
}

fn settings_text(stage: &Stage) -> String {
    format!(
        "sun: {}\nsky: {}\nboxes: {}\ndynamic: {}\nday: {}\n",
        if stage.sun_frozen { "freeze" } else { "run" },
        if stage.sky_on { "on" } else { "off" },
        if stage.boxes_still { "still" } else { "move" },
        stage.mix.dynamic_pct,
        stage.day,
    )
}

/// A passed flag wins over the matching environment variable. An omitted flag
/// leaves that variable alone, so it still applies.
fn apply_lighting_cli(opts: &Options) -> Result<(), String> {
    let mut check = LightingSettings::preset(LightingPreset::High);
    if let Some(name) = &opts.preset {
        if LightingPreset::parse(name).is_none() {
            return Err(format!("unknown preset {name}"));
        }
        std::env::set_var("GENOS_LIGHTING_PRESET", name);
    }
    let keys = [
        (
            opts.denoise_iters.as_deref(),
            "denoise_iters",
            "GENOS_LIGHTING_DENOISE_ITERS",
        ),
        (
            opts.denoise_adaptive.as_deref(),
            "denoise_adaptive",
            "GENOS_LIGHTING_DENOISE_ADAPTIVE",
        ),
        (
            opts.denoise_variance.as_deref(),
            "denoise_variance",
            "GENOS_LIGHTING_DENOISE_VARIANCE",
        ),
        (
            opts.denoise_screen.as_deref(),
            "denoise_screen",
            "GENOS_LIGHTING_DENOISE_SCREEN",
        ),
        (
            opts.denoise_launch.as_deref(),
            "denoise_launch",
            "GENOS_LIGHTING_DENOISE_LAUNCH",
        ),
        (
            opts.lamp_pick.as_deref(),
            "lamp_pick",
            "GENOS_LIGHTING_LAMP_PICK",
        ),
        (
            opts.lamp_pick_samples.as_deref(),
            "lamp_pick_samples",
            "GENOS_LIGHTING_LAMP_PICK_SAMPLES",
        ),
        (
            opts.lamp_pick_rays.as_deref(),
            "lamp_pick_rays",
            "GENOS_LIGHTING_LAMP_PICK_RAYS",
        ),
        (
            opts.lamp_pick_reuse.as_deref(),
            "lamp_pick_reuse",
            "GENOS_LIGHTING_LAMP_PICK_REUSE",
        ),
        (
            opts.lamp_pick_candidates.as_deref(),
            "lamp_pick_candidates",
            "GENOS_LIGHTING_LAMP_PICK_CANDIDATES",
        ),
        (
            opts.lamp_pick_radius.as_deref(),
            "lamp_pick_radius",
            "GENOS_LIGHTING_LAMP_PICK_RADIUS",
        ),
        (
            opts.lamp_pick_history.as_deref(),
            "lamp_pick_history",
            "GENOS_LIGHTING_LAMP_PICK_HISTORY",
        ),
        (
            opts.lamp_pick_unbiased.as_deref(),
            "lamp_pick_unbiased",
            "GENOS_LIGHTING_LAMP_PICK_UNBIASED",
        ),
    ];
    for (value, key, var) in keys {
        if let Some(value) = value {
            check.set_key(key, value)?;
            std::env::set_var(var, value);
        }
    }
    if let Some(every) = &opts.gpu_times {
        parse::<u32>("--gpu-times", every)?;
        std::env::set_var("GENOS_GPU_TIMES", every);
    }
    Ok(())
}

fn lighting_text(renderer: &Renderer) -> String {
    let stats = renderer.settle_stats();
    format!(
        "stable: {}\nstill_frames: {}\npicture_settled: {}\n",
        stats.settled,
        stats.still_frames,
        renderer.picture_settled(),
    )
}

fn run() -> Result<(), String> {
    if std::env::args().nth(1).as_deref() == Some("gi-check") {
        return gi_check::run(std::env::args().skip(2));
    }
    if std::env::args().nth(1).as_deref() == Some("lighting-ref") {
        return match genos_stress::lighting_ref::run(std::env::args().skip(2)) {
            Ok(code) => {
                std::process::exit(code);
            }
            Err(err) => Err(err),
        };
    }
    if std::env::args().nth(1).as_deref() == Some("noise-gate") {
        // The settle check reads the lamp-pick diag counters. The argument is
        // this subcommand; the variable is how the renderer already publishes them.
        std::env::set_var("GENOS_LIGHTING_LAMP_PICK_DIAG", "1");
        return match genos_stress::noise_gate::run_args(std::env::args().skip(2)) {
            Ok(code) => {
                std::process::exit(code);
            }
            Err(err) => Err(err),
        };
    }
    let opts = options()?;
    apply_lighting_cli(&opts)?;
    let mix = LightMix {
        count: opts.lights,
        dynamic_pct: opts.dynamic,
        layout: opts.layout,
    };
    let mut stage = Stage::new(
        opts.scale,
        opts.seed,
        mix,
        opts.power,
        opts.time,
        opts.day_seconds,
    );
    stage.sun_speed = opts.sun_speed;
    stage.sun_frozen = opts.freeze_sun;
    stage.sky_on = !opts.no_sky;
    stage.boxes_still = opts.still_boxes;
    if let Some(path) = &opts.scene {
        stage.set_room(genos_scene::load_path(path)?);
        stage.lamp_stop = opts.lamp_stop;
    }
    let mut camera = camera_at(opts.view);
    if let Some([x, y, z, yaw, pitch]) = opts.eye {
        camera.set_pose(genos_scene::Vec3::new(x, y, z), yaw, pitch);
    }
    let (width, height) = opts.size;
    let display = opts.display.as_deref();
    let mut window = if opts.proof {
        Window::open_proof_on(width, height, display)?
    } else {
        Window::open_named_on(
            width,
            height,
            "genos-stress",
            "Genos lighting stress",
            display,
        )?
    };
    window.install_seat_pump();
    Renderer::set_blocker(genos_window::seat_enter, genos_window::seat_leave);
    // Window title: what this run is (GENOS_TITLE_TAG from gi-check: binary, GI,
    // world mode, flags, test), and REF while a reference renders. The app id the
    // compositor rules match does not change.
    let title_base = match std::env::var("GENOS_TITLE_TAG") {
        Ok(tag) if opts.proof => format!("Genos Engine | {tag}"),
        _ => String::new(),
    };
    let mut title_ref = false;
    if !title_base.is_empty() {
        window.set_title(&title_base);
    }
    let first = window.pump();
    let mut renderer = Renderer::open(window.display, window.surface, first.width, first.height)?;
    if !opts.gltf.is_empty() {
        match load_gltf_pumped(
            &mut window,
            &mut renderer,
            &mut stage,
            &camera,
            opts.gltf.clone(),
        ) {
            Ok(()) => {}
            Err(err) if err == "window closed during load" => finish(window, renderer, 0),
            Err(err) => return Err(err),
        }
    }
    stage.unfilled_count = renderer.unfilled().count;
    stage.unfilled_tint = renderer.unfilled().tint;
    let host = genos_mcp::Host::new(stage.world.scene.clone(), camera.clone());
    let _server = genos_mcp::Server::start_on(host, genos_mcp::listen_port())?;
    eprintln!(
        "genos-stress lighting http://127.0.0.1:{}/lighting",
        genos_mcp::listen_port()
    );
    eprintln!(
        "genos-stress {} ({} sections), {} occluders, {} moving boxes, {} lamps ({} moving)",
        stage.scale.label(),
        stage.scale.sections(),
        stage.occluders(),
        stage.building.movers(),
        stage.lamp_count(),
        stage.mix.dynamic().min(stage.lamp_count()),
    );

    if let Some(plan) = &opts.bench {
        let text = bench::run(&mut renderer, &mut window, &mut stage, &camera, plan)?;
        println!("{text}");
        if let Some(path) = &opts.bench_out {
            std::fs::write(path, &text).map_err(|err| err.to_string())?;
        }
        finish(window, renderer, 0);
    }

    let mut tools = genos_debug::Tools::new();
    let mut script = opts.script.clone().map(|run| {
        tools.set_fixed_step(Some(bench::STEP));
        tools.set_out_dir(run.report_dir.clone());
        genos_debug::script::spawn(run)
    });
    if opts.step || opts.fixed_dt.is_some() {
        tools.set_fixed_step(Some(opts.fixed_dt.unwrap_or(bench::STEP)));
    }
    if opts.step {
        tools.pause();
        eprintln!(
            "genos-stress: stepped at {} s; Space or MCP step advances one frame",
            opts.fixed_dt.unwrap_or(bench::STEP)
        );
    }
    let fixed_step = opts.frames.is_some() || opts.shot.is_some();
    let mut size = (renderer.width(), renderer.height());
    let mut focus = FocusGate::default();
    let mut ui = State::default();
    let mut input = InputSystem::new();
    let keys = debug_keys()?;
    let controls = character_controller();
    let mut drawn = 0u32;
    let mut clock: Option<Instant> = None;
    let mut sim_time = 0.0f32;
    let run_started = Instant::now();
    let mut fps = FrameRate::default();
    let mut graph = FrameGraph::default();
    loop {
        let now = Instant::now();
        let real_dt = clock
            .map(|then| now.saturating_duration_since(then).as_secs_f32())
            .unwrap_or(bench::STEP);
        let mut dt = tools.scene_dt(if fixed_step {
            bench::STEP
        } else {
            real_dt.min(MAX_FRAME_SECONDS)
        });
        clock = Some(now);
        window
            .stall()
            .pull_published(stage.world.scene.event_loop.longest_stall_ms);
        let frame = window.pump();
        let stall_ms = window.stall().longest_ms();
        stage.world.scene.event_loop.note(stall_ms);
        if !title_base.is_empty() {
            let rendering =
                genos_debug::reference::RENDERING.load(std::sync::atomic::Ordering::SeqCst) > 0;
            if rendering != title_ref {
                title_ref = rendering;
                window.set_title(&if rendering {
                    format!("{title_base} | REF")
                } else {
                    title_base.clone()
                });
            }
        }
        if frame.closing {
            finish(window, renderer, tools.quit_code().unwrap_or(0));
        }
        if extent_changed(size.0, size.1, frame.width, frame.height) || frame.resized {
            renderer.resize(frame.width, frame.height)?;
            size = (renderer.width(), renderer.height());
        }
        let gated = focus.gate(&frame);
        input.begin_frame();
        input.apply_evdev_keys(&gated.keys_down);
        if opts.step && input.keyboard.pressed(InputCode::key_space) {
            tools.request_steps(1);
            dt = tools.scene_dt(bench::STEP);
        }
        input.set_mouse_button(InputCode::mouse_left, gated.mouse_left);
        input.add_mouse_delta(gated.mouse_dx, gated.mouse_dy);
        input.poll_gamepads();
        if keys.pressed(&input, "unfilled_tint") {
            stage.unfilled_tint = !stage.unfilled_tint;
            if stage.unfilled_tint {
                stage.unfilled_count = true;
            }
        } else if keys.pressed(&input, "unfilled") {
            stage.unfilled_count = !stage.unfilled_count;
            if !stage.unfilled_count {
                stage.unfilled_tint = false;
            }
        }
        let move_axis = controls.axis_2d(&input, "move");
        let look_axis = controls.axis_2d(&input, "look");

        fps.note(real_dt);
        let (paints, look_capture) = if opts.panel {
            let ui_frame = button_panel(
                &mut ui,
                &panel_title(&stage, &fps),
                &[
                    panel_rows(&stage, &renderer.debug_settings(&stage.world.scene).out),
                    knob_rows(&stage),
                ]
                .concat(),
                [size.0 as f32, size.1 as f32],
                &camera,
                Pointer {
                    x: frame.pointer_x,
                    y: frame.pointer_y,
                    down: !camera.captured && (gated.mouse_left || gated.capture_click),
                    ..Pointer::default()
                },
            );
            for action in &ui_frame.actions {
                match *action {
                    Action::Press(id) => press(&mut stage, id),
                    Action::Slide { id, value } => slide(&mut stage, id, value),
                    _ => {}
                }
            }
            (ui_frame.paints, ui_frame.look_capture)
        } else {
            (Vec::new(), true)
        };
        let mut overlay: Vec<ScreenRect> = paints
            .iter()
            .map(|p| ScreenRect {
                x: p.x,
                y: p.y,
                w: p.w,
                h: p.h,
                color: p.color,
            })
            .collect();

        if let Some(patch) = genos_mcp::take_settings() {
            apply_lighting(&mut stage, &patch);
        }
        stage.advance(dt);
        fly(
            &mut camera,
            &mut stage.world.scene,
            &input,
            &Actions {
                forward: move_axis.y,
                strafe: move_axis.x,
                mouse_dx: input.mouse.dx,
                mouse_dy: input.mouse.dy,
                look_x: look_axis.x,
                look_y: look_axis.y,
                capture_click: look_capture
                    && (gated.capture_click || controls.down(&input, "capture")),
                escape: controls.down(&input, "release"),
            },
            dt,
        );
        if camera.captured != frame.pointer_locked {
            if let Err(err) = window.set_pointer_capture(camera.captured) {
                eprintln!("genos-stress: {err}");
                camera.captured = false;
            }
        }

        if opts.ui {
            stage.world.scene.debug.ui = true;
        }
        let capture_ui = stage.world.scene.debug.ui;
        let tools_read = tools.before_draw(&mut genos_debug::Ctx {
            renderer: &mut renderer,
            camera: &mut camera,
            host: &mut stage,
        });
        overlay.extend_from_slice(tools.overlay());
        graph.push(real_dt * 1000.0);
        if opts.panel {
            let (rects, lines) = graph.draw([size.0 as f32, size.1 as f32]);
            overlay.extend(rects);
            renderer.set_overlay_lines(&lines);
        }

        let mut unfilled = renderer.unfilled();
        unfilled.count = stage.unfilled_count;
        unfilled.tint = stage.unfilled_tint;
        renderer.set_unfilled(unfilled);
        let applied = renderer.unfilled();
        if keys.pressed(&input, "unfilled") || keys.pressed(&input, "unfilled_tint") {
            eprintln!("unfilled count {} tint {}", applied.count, applied.tint);
        }
        stage.unfilled_count = applied.count;
        stage.unfilled_tint = applied.tint;

        drawn += 1;
        if opts.step && dt > 0.0 {
            sim_time += dt;
        }
        let last = opts.frames.is_some_and(|limit| drawn >= limit);
        let shot = genos_mcp::take_shot_request();
        let want_read = shot || tools_read || (last && opts.shot.is_some());
        if want_read && !tools.show_capture_overlay(capture_ui) {
            overlay.clear();
            renderer.set_overlay_lines(&[]);
        }
        let draw_start = Instant::now();
        let pixels =
            renderer.draw_with_overlay(&stage.world, &camera, &overlay, want_read, false)?;
        if opts.step && dt > 0.0 {
            eprintln!(
                "genos-stress: frame {drawn} sim {sim_time:.4} wall {:.3}s settled {}",
                run_started.elapsed().as_secs_f64(),
                renderer.picture_settled()
            );
        }
        tools.after_draw(
            &mut renderer,
            pixels.as_deref(),
            genos_debug::FrameTiming {
                frame_ms: real_dt as f64 * 1000.0,
                draw_ms: draw_start.elapsed().as_secs_f64() * 1000.0,
                gpu_ms: None,
            },
        );
        if shot {
            let png = pixels
                .as_ref()
                .map(|pixels| png::encode_png(renderer.width(), renderer.height(), pixels))
                .unwrap_or_default();
            genos_mcp::finish_shot(&png);
        }
        genos_mcp::publish_lighting(&lighting_text(&renderer));
        genos_mcp::publish_settings(&settings_text(&stage));
        if let (Some(path), Some(pixels)) = (&opts.shot, pixels) {
            png::write_png(path, renderer.width(), renderer.height(), &pixels)?;
            let (hh, mm) = building::clock(stage.day);
            let eye = camera.position;
            eprintln!(
                "genos-stress wrote {} at {hh:02}:{mm:02}, eye {:.2} {:.2} {:.2} yaw {:.3} pitch {:.3}",
                path.display(),
                eye.x,
                eye.y,
                eye.z,
                camera.yaw,
                camera.pitch
            );
        }
        if last || tools.quit_code().is_some() {
            let mut code = tools.quit_code().unwrap_or(0);
            if let Some(handle) = script.as_ref() {
                if handle.is_finished() {
                    let passed = script
                        .take()
                        .and_then(|handle| handle.join().ok())
                        .unwrap_or(false);
                    if !passed {
                        code = 1;
                    }
                }
            }
            println!("genos-stress frames={drawn}");
            finish(window, renderer, code);
        }
    }
}

fn finish(window: Window, renderer: Renderer, code: i32) -> ! {
    Renderer::clear_blocker();
    window.clear_seat_pump();
    drop(window);
    renderer.abandon();
    std::process::exit(code);
}

fn loading_world() -> World {
    World::from_scene(Scene {
        floor: Floor {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 2.0,
            half_z: 2.0,
            color: [0.15, 0.15, 0.16],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights: Vec::new(),
        ceiling: None,
        sky: Some(Sky {
            color: [0.02, 0.02, 0.025],
        }),
        lighting: LightingSettings::preset(LightingPreset::Potato),
        debug: DebugSettings::default(),
        fps_counter: FpsCounterSettings::default(),
        event_loop: EventLoopSettings::default(),
        menu: MenuSettings::default(),
    })
}

fn bake_gltf(mut world: World, models: &[(PathBuf, [f32; 4])]) -> Result<World, String> {
    for (path, [x, y, z, scale]) in models {
        let model =
            genos_load::load_gltf_file(path).map_err(|err| format!("{}: {err}", path.display()))?;
        let mut place = [0.0; 16];
        place[0] = *scale;
        place[5] = *scale;
        place[10] = *scale;
        place[12] = *x;
        place[13] = *y;
        place[14] = *z;
        place[15] = 1.0;
        let mut bake = genos_render::bake::BakeSettings::default();
        if std::env::var_os("GENOS_BAKE").is_none() {
            bake.mode = genos_render::bake::BakeMode::OnLoad;
        }
        let added = world
            .add_gltf_traced(&model, &place, &bake)
            .map_err(|err| format!("{}: bake: {err:?}", path.display()))?;
        eprintln!(
            "genos-stress: {} placed as {} mesh draws",
            path.display(),
            added.len()
        );
    }
    Ok(world)
}

fn load_gltf_pumped(
    window: &mut Window,
    renderer: &mut Renderer,
    stage: &mut Stage,
    camera: &Camera,
    models: Vec<(PathBuf, [f32; 4])>,
) -> Result<(), String> {
    let heavy = std::mem::replace(&mut stage.world, loading_world());
    let meter = window.stall_meter();
    let loaded = genos_window::run_pumped(
        &meter,
        genos_window::PUMP_BUDGET,
        || {
            let frame = window.pump();
            if frame.closing {
                return genos_window::PumpStep::Close;
            }
            let _ = renderer.draw_with_overlay(&stage.world, camera, &[], false, false);
            genos_window::PumpStep::Continue
        },
        move || bake_gltf(heavy, &models),
    )?;
    stage.world = loaded?;
    Ok(())
}

/// Frame time over the last 240 frames as a line, bottom left, with 16.7 ms and
/// 33.3 ms guides. Drawn in the overlay pass with the panel.
#[derive(Default)]
struct FrameGraph {
    ms: std::collections::VecDeque<f32>,
}

impl FrameGraph {
    const FRAMES: usize = 240;
    const W: f32 = 240.0;
    const H: f32 = 72.0;
    /// Milliseconds at the top of the graph.
    const TOP_MS: f32 = 50.0;

    fn push(&mut self, ms: f32) {
        self.ms.push_back(ms);
        while self.ms.len() > Self::FRAMES {
            self.ms.pop_front();
        }
    }

    fn draw(&self, viewport: [f32; 2]) -> (Vec<ScreenRect>, Vec<ScreenLine>) {
        let (x0, y0) = (16.0, viewport[1] - 16.0 - Self::H);
        // 50 ms tall unless frames run longer; then the slowest frame fits.
        let top = self.ms.iter().fold(Self::TOP_MS, |m, v| m.max(*v * 1.1));
        let y_of = |ms: f32| y0 + Self::H * (1.0 - (ms / top).clamp(0.0, 1.0));
        let back = ScreenRect {
            x: x0,
            y: y0,
            w: Self::W,
            h: Self::H,
            color: [0.08, 0.09, 0.11],
        };
        let guide = |ms: f32| ScreenLine {
            a: [x0, y_of(ms)],
            b: [x0 + Self::W, y_of(ms)],
            width: 1.0,
            color: [0.35, 0.38, 0.42],
        };
        let mut lines = vec![guide(1000.0 / 60.0), guide(1000.0 / 30.0)];
        let step = Self::W / (Self::FRAMES - 1) as f32;
        let start = Self::FRAMES - self.ms.len();
        let points: Vec<[f32; 2]> = self
            .ms
            .iter()
            .enumerate()
            .map(|(i, ms)| [x0 + (start + i) as f32 * step, y_of(*ms)])
            .collect();
        lines.extend(points.windows(2).map(|p| ScreenLine {
            a: p[0],
            b: p[1],
            width: 2.0,
            color: [0.45, 0.85, 0.40],
        }));
        (vec![back], lines)
    }
}

/// Frame rate over the last half second, for the panel title.
#[derive(Default)]
struct FrameRate {
    seconds: f32,
    frames: u32,
    fps: f32,
}

impl FrameRate {
    fn note(&mut self, dt: f32) {
        self.seconds += dt;
        self.frames += 1;
        if self.seconds >= 0.5 {
            *self = Self {
                fps: self.frames as f32 / self.seconds,
                ..Self::default()
            };
        }
    }
}

fn panel_title(stage: &Stage, rate: &FrameRate) -> String {
    let (hh, mm) = building::clock(stage.day);
    format!(
        "Stress {} {:.0} fps {hh:02}:{mm:02}",
        stage.scale.label(),
        rate.fps,
    )
}

fn row(label: &str, buttons: Vec<PanelButton>) -> PanelRow {
    PanelRow {
        label: label.into(),
        buttons,
        slider: None,
        dropdown: None,
        checks: Vec::new(),
    }
}

/// A slider row for each panel knob, with its value as the readout.
fn knob_rows(stage: &Stage) -> Vec<PanelRow> {
    let all = stage.knobs();
    PANEL_KNOBS
        .iter()
        .enumerate()
        .filter_map(|(i, name)| {
            let k = all.iter().find(|k| k.name == *name)?;
            Some(PanelRow {
                label: k.label.clone(),
                buttons: Vec::new(),
                slider: Some(PanelSlider {
                    id: ID_KNOB + i as u32,
                    value: k.value,
                    min: k.min,
                    max: k.max,
                    step: k.step,
                    text: k.text.clone(),
                    steps: Vec::new(),
                }),
                dropdown: None,
                checks: Vec::new(),
            })
        })
        .collect()
}

/// A panel slider moved: set its knob.
fn slide(stage: &mut Stage, id: u32, value: f64) {
    let Some(name) = id
        .checked_sub(ID_KNOB)
        .and_then(|i| PANEL_KNOBS.get(i as usize))
    else {
        return;
    };
    let _ = stage.set_knob(name, value);
}

fn button(id: u32, text: String, selected: bool) -> PanelButton {
    PanelButton { id, text, selected }
}

fn panel_rows(stage: &Stage, out_dir: &str) -> Vec<PanelRow> {
    vec![
        row(
            "Out",
            vec![
                button(ID_OUT, out_dir.into(), false),
                button(
                    ID_OUT + 1,
                    "default".into(),
                    out_dir == DebugSettings::DEFAULT_OUT,
                ),
            ],
        ),
        row(
            "Lamps in",
            vec![
                button(ID_LAYOUT, "all".into(), stage.mix.layout == Layout::Spread),
                button(
                    ID_LAYOUT + 1,
                    "section 0".into(),
                    stage.mix.layout == Layout::First,
                ),
            ],
        ),
        row(
            "Scale",
            vec![
                button(ID_SCALE, "small".into(), stage.scale == Scale::SMALL),
                button(ID_SCALE + 1, "big".into(), stage.scale == Scale::BIG),
            ],
        ),
        row(
            "Sun",
            vec![
                button(ID_SUN, "run".into(), !stage.sun_frozen),
                button(ID_SUN + 1, "freeze".into(), stage.sun_frozen),
            ],
        ),
        row(
            "Sky",
            vec![
                button(ID_SKY, "on".into(), stage.sky_on),
                button(ID_SKY + 1, "off".into(), !stage.sky_on),
            ],
        ),
        row(
            "Boxes",
            vec![
                button(ID_BOXES, "move".into(), !stage.boxes_still),
                button(ID_BOXES + 1, "still".into(), stage.boxes_still),
            ],
        ),
    ]
}

fn press(stage: &mut Stage, id: u32) {
    let pick = |base: u32, len: usize| {
        (base..base + len as u32)
            .contains(&id)
            .then(|| (id - base) as usize)
    };
    if let Some(i) = pick(ID_SCALE, 2) {
        stage.set_scale([Scale::SMALL, Scale::BIG][i]);
    } else if let Some(i) = pick(ID_SUN, 2) {
        stage.sun_frozen = i == 1;
    } else if let Some(i) = pick(ID_SKY, 2) {
        stage.sky_on = i == 0;
    } else if let Some(i) = pick(ID_BOXES, 2) {
        stage.boxes_still = i == 1;
    } else if let Some(i) = pick(ID_LAYOUT, 2) {
        stage.mix.layout = [Layout::Spread, Layout::First][i];
    } else if id == ID_OUT + 1 {
        let _ = stage
            .world
            .scene
            .debug
            .set_key("out", DebugSettings::DEFAULT_OUT);
    }
    stage.apply();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_and_fixed_dt_prefer_the_argument() {
        let (step, dt) = time_control(true, Some(0.05), Some("off"), Some("0.2")).unwrap();
        assert!(step);
        assert!((dt.unwrap() - 0.05).abs() < 1.0e-6);
        let (step, dt) = time_control(false, None, Some("1"), Some("0.2")).unwrap();
        assert!(step);
        assert!((dt.unwrap() - 0.2).abs() < 1.0e-6);
        let (step, dt) = time_control(false, None, Some("off"), None).unwrap();
        assert!(!step);
        assert!(dt.is_none());
        assert!(time_control(false, Some(0.0), None, None).is_err());
    }
}
