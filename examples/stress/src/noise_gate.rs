#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")
)]

//! s7 noise gate. Settles a lamp grid, then grades still flicker, a frozen
//! pattern, pan pops, and the path-traced direct error of the random lamp
//! path against the exact sum inside the same reuse shade.
//!
//! The process must set `GENOS_LIGHTING_LAMP_PICK_DIAG=1` before the first
//! draw. The binary does that. Without it the ray counters stay zero and the
//! settle check fails closed.
//!
//! Pixels with 8 or fewer positive lamps shade the exact sum, so the grid
//! defaults to 16. The live pass turns the exact-sum hold off. A still view
//! would otherwise return before the random path traces.

use std::path::PathBuf;
use std::sync::Mutex;

use genos_debug::noise_gate::{frame_luminance, grade, report_json, GateReport, GateSamples};
use genos_debug::png;
use genos_debug::{Image, RefSetup};
use genos_render::{DebugView, LampPickCounts, LampShadeGate, Renderer, ViewMode, World};
use genos_scene::{
    AmbientMode, Camera, Floor, FogSettings, FpsCounterSettings, HdrSettings, Light, LightingPreset,
    LightingSettings, MenuSettings, Scene, Shape, Solid, Vec3,
};
use genos_window::Window;

const EYE_Y: f32 = 1.7;
const EYE_Z: f32 = 4.0;
/// `LAMP_PICK_EXACT_LIT`. At or under this count the pixel shades the sum.
const EXACT_LIT_LAMPS: u32 = 8;
const HELP: &str = "\
genos-stress noise-gate [options]

Settle, then grade the random lamp path against the exact sum.
Exit 0 for grade A or B. Exit 1 below B.

The baseline is the exact-sum path inside the reuse shade (every point lamp,
scale 1). It is not this binary with lamp pick off. The live pass is the
same shade with the exact-sum hold turned off, so a still view keeps tracing.
`--broken` picks uniformly and shades with weight 1 (no pairwise MIS). That
run must fail.

Lavapipe (default, no --preset): 96x54, 16 lamps, high's reservoir (8
candidates, radius 16, pairwise MIS), one bounce, 4 probe rays, world cache
off, denoiser off, direct view. The picture is direct light only.

Owner, 1080p High, 16 lamps:

  cargo run --release -p genos-stress -- noise-gate --size 1920x1080 --preset high --lamps 16 --out genos-debug/noise

--preset keeps that preset's GI and reservoir, then turns the light cap off,
ambient off, sky off, and compose reuse off, and draws the direct view.
A preset whose lamp pick is off cannot grade the random path.
Lavapipe times are not a result. The report is <out>/metrics.json.

  --size WxH       picture size (default 96x54)
  --settle N       still frames before the capture (default 4200)
  --frames N       still frames, and pan frames (default 64, at least 9)
  --preset NAME    fallback, day-cycle, gi-1bounce, low, medium, high
  --lamps N        lamps on a grid (default 16, at least 9). Intensity is 3/N
  --broken         uniform pick, shade weight 1, no pairwise weight
  --out DIR        report directory (default ./genos-debug/noise)
  --pan-yaw RAD    yaw swept over the pan (default 0.35)
  --help           this text
";

/// What `noise-gate` runs.
#[derive(Clone, Debug)]
pub struct Config {
    pub width: u32,
    pub height: u32,
    pub settle: u32,
    pub frames: u32,
    pub preset: Option<LightingPreset>,
    pub lamps: u32,
    pub broken: bool,
    pub out: PathBuf,
    pub pan_yaw: f32,
}

impl Config {
    /// The lavapipe gate: small picture, 16 lamps, full settle and capture.
    #[must_use]
    pub fn lavapipe(out: PathBuf) -> Self {
        Self {
            width: 96,
            height: 54,
            settle: 4200,
            frames: 64,
            preset: None,
            lamps: 16,
            broken: false,
            out,
            pan_yaw: 0.35,
        }
    }
}

/// Parse `noise-gate`'s own arguments. `--help` prints the text and returns `Ok(None)`.
///
/// # Errors
///
/// An argument is missing its value, or a value is not the kind that flag takes.
pub fn parse<I, S>(args: I) -> Result<Option<Config>, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut width = 96u32;
    let mut height = 54u32;
    let mut settle = 4200u32;
    let mut frames = 64u32;
    let mut preset = None;
    let mut lamps = 16u32;
    let mut broken = false;
    let mut out = PathBuf::from("genos-debug/noise");
    let mut pan_yaw = 0.35f32;
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        let arg = arg.as_ref();
        match arg {
            "--help" | "-h" => {
                println!("{HELP}");
                return Ok(None);
            }
            "--size" => {
                let text = it.next().ok_or("--size needs WxH")?;
                let text = text.as_ref();
                let (w, h) = text
                    .split_once('x')
                    .ok_or_else(|| format!("--size {text} is not WxH"))?;
                width = w.parse().map_err(|_| format!("--size {text} is not WxH"))?;
                height = h.parse().map_err(|_| format!("--size {text} is not WxH"))?;
            }
            "--settle" => {
                let text = it.next().ok_or("--settle needs a count")?;
                settle = text
                    .as_ref()
                    .parse()
                    .map_err(|_| format!("--settle {} is not a count", text.as_ref()))?;
            }
            "--frames" => {
                let text = it.next().ok_or("--frames needs a count")?;
                frames = text
                    .as_ref()
                    .parse()
                    .map_err(|_| format!("--frames {} is not a count", text.as_ref()))?;
            }
            "--preset" => {
                let text = it.next().ok_or("--preset needs a name")?;
                preset = Some(
                    LightingPreset::parse(text.as_ref())
                        .ok_or_else(|| format!("unknown preset {}", text.as_ref()))?,
                );
            }
            "--lamps" => {
                let text = it.next().ok_or("--lamps needs a count")?;
                lamps = text
                    .as_ref()
                    .parse()
                    .map_err(|_| format!("--lamps {} is not a count", text.as_ref()))?;
            }
            "--broken" => broken = true,
            "--out" => {
                let text = it.next().ok_or("--out needs a directory")?;
                out = PathBuf::from(text.as_ref());
            }
            "--pan-yaw" => {
                let text = it.next().ok_or("--pan-yaw needs radians")?;
                pan_yaw = text
                    .as_ref()
                    .parse()
                    .map_err(|_| format!("--pan-yaw {} is not a number", text.as_ref()))?;
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if width == 0 || height == 0 {
        return Err("--size needs a positive WxH".to_string());
    }
    if settle == 0 {
        return Err("--settle needs at least 1 frame".to_string());
    }
    if frames < 9 {
        return Err("--frames needs at least 9 (the pop window)".to_string());
    }
    if lamps <= EXACT_LIT_LAMPS {
        return Err(format!(
            "--lamps {lamps} shades the exact sum (the threshold is {EXACT_LIT_LAMPS}); the random path needs more"
        ));
    }
    if !pan_yaw.is_finite() || pan_yaw < 0.0 {
        return Err("--pan-yaw needs a non-negative number".to_string());
    }
    Ok(Some(Config {
        width,
        height,
        settle,
        frames,
        preset,
        lamps,
        broken,
        out,
        pan_yaw,
    }))
}

/// Parse the arguments and run. `--help` is exit 0.
///
/// # Errors
///
/// [`parse`] failed, or [`run`] failed to draw or to write the report.
pub fn run_args<I, S>(args: I) -> Result<i32, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    match parse(args)? {
        None => Ok(0),
        Some(config) => run(&config),
    }
}

/// Run the gate. `0` is grade A or B.
///
/// # Errors
///
/// The window or the renderer failed to open, a frame failed to draw, or the
/// report directory could not be written.
pub fn run(config: &Config) -> Result<i32, String> {
    let _gpu = gpu_lock();
    let started = std::time::Instant::now();
    let mut window = Window::open_proof(config.width, config.height)?;
    let first = window.pump();
    let mut renderer = Renderer::open(window.display, window.surface, first.width, first.height)?;
    let width = renderer.width();
    let height = renderer.height();
    let world = overlap_world(config.lamps);
    let pose = still_camera();
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Direct,
        ..DebugView::default()
    });
    let live_gate = if config.broken {
        LampShadeGate::Broken
    } else {
        LampShadeGate::Random
    };
    eprintln!(
        "noise-gate {width}x{height} lamps {} settle {} frames {} preset {} {}",
        config.lamps,
        config.settle,
        config.frames,
        config.preset.map_or("direct-quiet", LightingPreset::name),
        if config.broken { "broken" } else { "random" }
    );
    let mut live = capture_pass(
        &mut window,
        &mut renderer,
        &world,
        &pose,
        config,
        live_gate,
        "live",
    )?;
    let mut exact = capture_pass(
        &mut window,
        &mut renderer,
        &world,
        &pose,
        config,
        LampShadeGate::Exact,
        "exact",
    )?;
    renderer.set_lamp_shade_gate(LampShadeGate::Product);
    renderer.set_lighting_settings(gate_lighting(config.preset));
    let solos = solo_frames(&mut window, &mut renderer, &world, &pose)?;
    let reference = reference_of(&world.scene, &pose, width, height);
    let report = grade_captures(
        width,
        height,
        &mut live,
        &mut exact,
        frame_luminance(&reference.image),
        reference.hit,
        solos,
    )?;
    write_gate_files(&config.out, width, height, &report, &live, &exact)?;
    println!("{report}");
    eprintln!(
        "noise-gate wrote {} in {:.1}s",
        config.out.display(),
        started.elapsed().as_secs_f64()
    );
    Ok(i32::from(!report.exit_ok))
}

fn grade_captures(
    width: u32,
    height: u32,
    live: &mut Pass,
    exact: &mut Pass,
    reference_y: Vec<f32>,
    hit: Vec<bool>,
    solos: Vec<Vec<f32>>,
) -> Result<GateReport, String> {
    grade(
        &capture_samples(
            width,
            height,
            live,
            reference_y.clone(),
            hit.clone(),
            solos.clone(),
        ),
        &capture_samples(width, height, exact, reference_y, hit, solos),
    )
}

fn capture_samples(
    width: u32,
    height: u32,
    pass: &mut Pass,
    reference: Vec<f32>,
    hit: Vec<bool>,
    solos: Vec<Vec<f32>>,
) -> GateSamples {
    GateSamples {
        width,
        height,
        still: std::mem::take(&mut pass.still),
        pan: std::mem::take(&mut pass.pan),
        reference,
        hit,
        solos,
        still_rays: pass.still_rays,
        still_restir: pass.still_restir,
        pan_restir: pass.pan_restir,
        pan_exact: pass.pan_exact,
    }
}

fn write_gate_files(
    out: &std::path::Path,
    width: u32,
    height: u32,
    report: &GateReport,
    live: &Pass,
    exact: &Pass,
) -> Result<(), String> {
    std::fs::create_dir_all(out).map_err(|err| err.to_string())?;
    std::fs::write(out.join("metrics.json"), report_json(report)).map_err(|err| err.to_string())?;
    png::write_png(&out.join("live-still.png"), width, height, &live.last_still)?;
    png::write_png(&out.join("live-pan.png"), width, height, &live.last_pan)?;
    png::write_png(&out.join("exact-still.png"), width, height, &exact.last_still)?;
    png::write_png(&out.join("exact-pan.png"), width, height, &exact.last_pan)?;
    Ok(())
}

struct Pass {
    still: Vec<Vec<f32>>,
    pan: Vec<Vec<f32>>,
    still_rays: u32,
    still_restir: u32,
    pan_restir: u32,
    pan_exact: u32,
    last_still: Vec<u8>,
    last_pan: Vec<u8>,
}

fn capture_pass(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    pose: &Camera,
    config: &Config,
    gate: LampShadeGate,
    label: &str,
) -> Result<Pass, String> {
    renderer.set_lighting_settings(gate_lighting(config.preset));
    renderer.set_lamp_shade_gate(gate);
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Direct,
        ..DebugView::default()
    });
    for i in 0..config.settle {
        let _ = window.pump();
        renderer.draw(world, pose, false)?;
        if i + 1 == config.settle || (i + 1) % 50 == 0 {
            let c = renderer.lighting_lamp_pick_counts();
            eprintln!(
                "noise-gate {label} settle {}/{} rays {} exact {} restir {} copied {}",
                i + 1,
                config.settle,
                c.pixels,
                c.exact,
                c.restir,
                c.copied
            );
        }
    }
    let mut still = Vec::with_capacity(usize::try_from(config.frames).unwrap_or(0));
    let mut last_still = Vec::new();
    let mut still_counts = LampPickCounts::default();
    for _ in 0..config.frames {
        let (y, bgra, counts) = draw_frame(window, renderer, world, pose)?;
        still_counts = counts;
        last_still = bgra;
        still.push(y);
    }
    eprintln!(
        "noise-gate {label} still rays {} exact {} restir {} copied {}",
        still_counts.pixels, still_counts.exact, still_counts.restir, still_counts.copied
    );
    let mut pan = Vec::with_capacity(usize::try_from(config.frames).unwrap_or(0));
    let mut last_pan = Vec::new();
    let mut pan_counts = still_counts;
    for i in 0..config.frames {
        let mut camera = pose.clone();
        let yaw = if config.frames <= 1 {
            0.0
        } else {
            config.pan_yaw * u32_as_f32(i) / u32_as_f32(config.frames - 1)
        };
        camera.set_pose(Vec3::new(0.0, EYE_Y, EYE_Z), yaw, 0.0);
        let (y, bgra, counts) = draw_frame(window, renderer, world, &camera)?;
        pan_counts = counts;
        last_pan = bgra;
        pan.push(y);
    }
    eprintln!(
        "noise-gate {label} pan rays {} exact {} restir {} copied {}",
        pan_counts.pixels, pan_counts.exact, pan_counts.restir, pan_counts.copied
    );
    Ok(Pass {
        still,
        pan,
        still_rays: still_counts.pixels,
        still_restir: still_counts.restir,
        pan_restir: pan_counts.restir,
        pan_exact: pan_counts.exact,
        last_still,
        last_pan,
    })
}

fn draw_frame(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
) -> Result<(Vec<f32>, Vec<u8>, LampPickCounts), String> {
    let _ = window.pump();
    let bgra = renderer
        .draw(world, camera, true)?
        .ok_or_else(|| "readback returned no picture".to_string())?;
    let y = frame_luminance(&Image::from_bgra(
        renderer.width(),
        renderer.height(),
        &bgra,
    ));
    Ok((y, bgra, renderer.lighting_lamp_pick_counts()))
}

fn solo_frames(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    pose: &Camera,
) -> Result<Vec<Vec<f32>>, String> {
    let mut solos = Vec::new();
    for lamp in 0..world.scene.lights.len() {
        renderer.set_debug_view(DebugView {
            mode: ViewMode::Light,
            lamp: u32::try_from(lamp).unwrap_or(0),
        });
        let (y, _, _) = draw_frame(window, renderer, world, pose)?;
        solos.push(y);
    }
    Ok(solos)
}

fn reference_of(scene: &Scene, camera: &Camera, width: u32, height: u32) -> genos_debug::Reference {
    let mut setup = RefSetup::from_camera(scene, camera, width, height, 1);
    setup.max_bounces = 0;
    setup.max_spp = 1;
    setup.noise_target = 0.0;
    genos_debug::render_reference(&setup)
}

fn u32_as_f32(n: u32) -> f32 {
    let hi = f32::from(u16::try_from(n >> 16).unwrap_or(0));
    let lo = f32::from(u16::try_from(n & 0xffff).unwrap_or(0));
    hi.mul_add(65536.0, lo)
}

fn still_camera() -> Camera {
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(0.0, EYE_Y, EYE_Z), 0.0, 0.0);
    camera
}

/// Columns of a near-square lamp grid. `n` is at least 1.
fn grid_cols(n: u32) -> u32 {
    let mut cols = 1u32;
    while cols.saturating_mul(cols) < n {
        cols += 1;
    }
    cols
}

/// `n` lamps on a grid over a floor, and a box that shadows the centre from
/// part of the ground. Each lamp's colour is `3/n`, so the open floor stays
/// about as bright as the old three-lamp scene. The open floor is an overlap:
/// no lamp is most of the light.
fn overlap_world(n: u32) -> World {
    let n = n.max(1);
    let cols = grid_cols(n);
    let rows = n.div_ceil(cols);
    let scale = 3.0 / u32_as_f32(n);
    let mut lights = Vec::with_capacity(usize::try_from(n).unwrap_or(0));
    for i in 0..n {
        let col = i % cols;
        let row = i / cols;
        let x = if cols <= 1 {
            0.0
        } else {
            -1.6 + 3.2 * u32_as_f32(col) / u32_as_f32(cols - 1)
        };
        let z = if rows <= 1 {
            -1.5
        } else {
            -1.5 - 2.4 * u32_as_f32(row) / u32_as_f32(rows - 1)
        };
        lights.push(Light {
            position: Vec3::new(x, 2.2, z),
            color: [scale, scale, scale],
            direction: Vec3::ZERO,
        });
    }
    let scene = Scene {
        floor: Floor {
            position: Vec3::ZERO,
            half_x: 20.0,
            half_z: 20.0,
            color: [0.8, 0.8, 0.8],
        },
        walls: Vec::new(),
        solids: vec![Solid {
            shape: Shape::Square,
            position: Vec3::new(0.0, 0.0, -1.0),
            size: 0.8,
            height: 3.0,
            yaw: 0.0,
            color: [0.35, 0.35, 0.35],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }],
        lights,
        ceiling: None,
        sky: None,
        lighting: LightingSettings::preset(LightingPreset::Fallback),
        debug: genos_scene::DebugSettings::default(),
        fps_counter: FpsCounterSettings::default(),
        event_loop: genos_scene::EventLoopSettings::default(),
        menu: MenuSettings::default(),
        reflections: genos_scene::ReflectionSettings::default(),
        hdr: HdrSettings::legacy_bytes(),
        fog: FogSettings::default(),
    };
    World::from_scene(scene)
}

fn gate_lighting(preset: Option<LightingPreset>) -> LightingSettings {
    let mut settings = if let Some(preset) = preset {
        LightingSettings::preset(preset)
    } else {
        let mut settings = LightingSettings::preset(LightingPreset::High);
        settings.bounce.count = 1;
        settings.probes.enabled = true;
        settings.probes.rays = 4;
        settings.probes.tile = 64;
        settings.world_cache.enabled = false;
        settings.denoiser.enabled = false;
        settings.prefill = false;
        settings.gi.enabled = true;
        settings
    };
    settings.direct.max_lights = 0;
    settings.sky = false;
    settings.ambient.mode = AmbientMode::Off;
    settings.shadows.enabled = true;
    settings.compose_reuse = false;
    settings
}

fn gpu_lock() -> std::sync::MutexGuard<'static, ()> {
    static GPU: Mutex<()> = Mutex::new(());
    GPU.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_the_lavapipe_gate() {
        let config = parse(["--out", "genos-debug/noise"]).unwrap().unwrap();
        assert_eq!(config.width, 96);
        assert_eq!(config.height, 54);
        assert_eq!(config.settle, 4200);
        assert_eq!(config.frames, 64);
        assert_eq!(config.lamps, 16);
        assert!(!config.broken);
        assert!(config.preset.is_none());
        assert!((config.pan_yaw - 0.35).abs() < 1.0e-6);
    }

    #[test]
    fn a_short_pan_is_refused() {
        let err = parse(["--frames", "4"]).unwrap_err();
        assert!(err.contains("at least 9"), "{err}");
    }

    #[test]
    fn eight_lamps_never_reach_the_random_path() {
        let err = parse(["--lamps", "8"]).unwrap_err();
        assert!(err.contains("exact sum"), "{err}");
    }

    #[test]
    fn broken_is_a_flag_and_sixteen_lamps_keep_the_three_lamp_budget() {
        let config = parse(["--lamps", "16", "--broken"]).unwrap().unwrap();
        assert!(config.broken);
        assert_eq!(config.lamps, 16);
        let world = overlap_world(16);
        assert_eq!(world.scene.lights.len(), 16);
        let sum: f32 = world.scene.lights.iter().map(|lamp| lamp.color[0]).sum();
        assert!((sum - 3.0).abs() < 1.0e-4, "{sum}");
    }
}
