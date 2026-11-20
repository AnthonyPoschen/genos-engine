#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(test, expect(clippy::doc_markdown, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]

//! The exact shadow's penumbra against the path tracer's disc.
//!
//! One lamp, one wall, GI off, ReSTIR off. The labelled exact sum draws one
//! point on the lamp disc per frame. After the mean settles, the floor's
//! visibility profile matches [`genos_render::direct_light_samples`] (same
//! radius, 64 stratified points). A later frame in the penumbra moves by a
//! small fraction of the lit value.

mod common;

use common::PresentedFrame;
use genos_render::{
    direct_light_samples, luminance, DebugView, Renderer, SceneRead, ViewMode, World, LAMP_DISC,
};
use genos_scene::{
    viewport_uv, AmbientMode, Camera, Floor, LampPickMode, Light, LightingPreset, LightingSettings,
    Scene, Vec3, Wall,
};
use genos_window::Window;

const LAMBERT: f64 = 0.254_647_909;
const WARMUP: u32 = 64;
const AVERAGE: u32 = 8;
const TEMPORAL: u32 = 8;
/// Root mean square of the normalised visibility. A hard ray against this
/// disc is several times larger.
const PROFILE_RMSE: f64 = 0.18;
/// Metres. The 0.1 m disc opens a band wider than one floor sample.
const WIDTH_TOL: f64 = 0.12;
/// A settled sample moves the mean by about 1/n. A raw 0/1 ray is near 1.
const FRAME_STEP: f64 = 0.08;

fn open() -> (std::sync::MutexGuard<'static, ()>, genos_window::Lifecycle<Renderer>) {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = GPU.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut window = Window::open_proof(320, 180).expect("proof window");
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height).expect("renderer");
    (guard, genos_window::Lifecycle::new(window, renderer, permit))
}

fn settings() -> LightingSettings {
    let mut settings = LightingSettings::preset(LightingPreset::Potato);
    settings.direct.max_lights = 0;
    settings.direct.refresh_frames = 1;
    settings.direct.resolution_scale = 1;
    settings.direct.specular = false;
    settings.sky = false;
    settings.ambient.mode = AmbientMode::Off;
    settings.shadows.enabled = true;
    settings.gi.enabled = false;
    settings.bounce.count = 0;
    settings.probes.enabled = false;
    settings.world_cache.enabled = false;
    settings.lamp_pick.mode = LampPickMode::Off;
    settings.lamp_pick.radius = 0;
    settings.compose_reuse = false;
    settings.restir_di = false;
    settings
}

fn scene() -> Scene {
    Scene {
        floor: Floor {
            position: Vec3::ZERO,
            half_x: 8.0,
            half_z: 8.0,
            color: [1.0, 1.0, 1.0],
        },
        walls: vec![Wall {
            position: Vec3::new(0.0, 0.0, 1.0),
            half_x: 2.0,
            half_z: 0.05,
            height: 1.5,
            base: 0.0,
            color: [1.0, 1.0, 1.0],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }],
        solids: Vec::new(),
        lights: vec![Light {
            position: Vec3::new(0.0, 3.0, 0.0),
            color: [1.0, 1.0, 1.0],
            direction: Vec3::ZERO,
        }],
        ceiling: None,
        sky: None,
        lighting: settings(),
        debug: genos_scene::DebugSettings::default(),
        fps_counter: genos_scene::FpsCounterSettings::default(),
        event_loop: genos_scene::EventLoopSettings::default(),
        reflections: genos_scene::ReflectionSettings::default(),
        hdr: genos_scene::HdrSettings::for_preset(LightingPreset::High),
        fog: genos_scene::FogSettings::default(),
        menu: genos_scene::MenuSettings::default(),
        particles: genos_scene::ParticleSettings::default(),
    }
}

fn camera() -> Camera {
    let eye_y = 1.8_f32;
    let eye_z = 5.0_f32;
    let dy = -eye_y;
    let dz = 2.1 - eye_z;
    let pitch = (dy / (dy * dy + dz * dz).sqrt()).asin();
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(0.0, eye_y, eye_z), 0.0, pitch);
    camera
}

fn grab(window: &mut Window, renderer: &mut Renderer, world: &World, camera: &Camera) -> SceneRead {
    let _ = window.pump();
    renderer.draw(world, camera, true).expect("draw").expect("readback");
    renderer.read_scene().expect("scene radiance")
}

fn luma(rgb: [f64; 3]) -> f64 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

fn rounded_index(v: f64, limit: u32) -> Option<u32> {
    if !v.is_finite() {
        return None;
    }
    let want = v.round();
    let mut i = 0u32;
    while i < limit {
        if (f64::from(i) - want).abs() < 0.5 {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn pixel(read: &SceneRead, uv: [f32; 2]) -> Option<(f64, f64)> {
    let x = rounded_index(f64::from(uv[0]) * f64::from(read.width), read.width)?;
    let y = rounded_index(f64::from(uv[1]) * f64::from(read.height), read.height)?;
    let i = usize::try_from(y)
        .ok()?
        .saturating_mul(usize::try_from(read.width).ok()?)
        .saturating_add(usize::try_from(x).ok()?);
    let base = i.saturating_mul(3);
    let rgb = [
        *read.rgb.get(base)?,
        *read.rgb.get(base.saturating_add(1))?,
        *read.rgb.get(base.saturating_add(2))?,
    ];
    let dist = *read.distance_m.get(i)?;
    if !(0.4..30.0).contains(&dist) {
        return None;
    }
    Some((luma(rgb), dist))
}

fn reference_at(scene: &Scene, z: f32) -> f64 {
    f64::from(luminance(direct_light_samples(
        scene,
        [0.0, 0.02, z],
        [0.0, 1.0, 0.0],
        64,
    )))
}

fn crossing(samples: &[(f64, f64)]) -> Option<f64> {
    let mut prev: Option<(f64, f64)> = None;
    for &(z, vis) in samples {
        if let Some((pz, pv)) = prev {
            if (pv - 0.5) * (vis - 0.5) <= 0.0 && (vis - pv).abs() > 1.0e-6 {
                let t = (0.5 - pv) / (vis - pv);
                return Some(pz + (z - pz) * t);
            }
        }
        prev = Some((z, vis));
    }
    None
}

fn presented_luma(frame: &PresentedFrame, width: u32, height: u32, uv: [f32; 2]) -> Option<f64> {
    let x = rounded_index(f64::from(uv[0]) * f64::from(width), width)?;
    let y = rounded_index(f64::from(uv[1]) * f64::from(height), height)?;
    let i = usize::try_from(y)
        .ok()?
        .saturating_mul(usize::try_from(width).ok()?)
        .saturating_add(usize::try_from(x).ok()?);
    frame.luma().get(i).copied()
}

/// The presented frame shows the disc penumbra, not a hard edge.
///
/// A floor sample whose reference visibility is near one half must sit between
/// the lit floor and the shadowed floor. Knocking out `lamp_disc` aims the
/// ray at the centre, so that sample becomes one or the other.
#[test]
fn the_penumbra_is_in_the_presented_frame() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let world = World::from_scene(scene());
    let camera = camera();
    let scene = scene();
    renderer.set_lighting_settings(settings());
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Direct,
        lamp: 0,
    });
    let mut last = None;
    for _ in 0..WARMUP {
        last = Some(PresentedFrame::from_draw(window, renderer, &world, &camera));
    }
    let frame = last.expect("presented frame");
    let width = renderer.width();
    let height = renderer.height();
    let aspect = f32::from(u16::try_from(width).expect("width"))
        / f32::from(u16::try_from(height).expect("height"));
    let mut lit = 0.0_f64;
    let mut dark = 255.0_f64;
    let mut mid = None::<(f32, f64, f64)>;
    let mut mid_gap = 1.0_f64;
    for step in 0u16..28 {
        let z = 1.50 + f32::from(step) * 0.05;
        let Some(uv) = viewport_uv(&camera, aspect, [0.0, 0.0, z]) else {
            continue;
        };
        let Some(y) = presented_luma(&frame, width, height, uv) else {
            continue;
        };
        let cpu = reference_at(&scene, z);
        let lit_cpu = reference_at(&scene, 2.8);
        let vis = if lit_cpu > 0.0 { cpu / lit_cpu } else { 0.0 };
        if vis > 0.85 {
            lit = lit.max(y);
        }
        if vis < 0.15 {
            dark = dark.min(y);
        }
        let gap = (vis - 0.5).abs();
        if gap < mid_gap {
            mid_gap = gap;
            mid = Some((z, y, vis));
        }
    }
    let (z, y, vis) = mid.expect("no floor sample");
    let span = lit - dark;
    let place = if span > 0.0 { (y - dark) / span } else { 0.0 };
    eprintln!(
        "presented penumbra z {z} vis {vis:.3} luma {y:.1} lit {lit:.1} dark {dark:.1} place {place:.3}"
    );
    assert!(
        span > 8.0 && place > 0.2 && place < 0.8,
        "the presented penumbra is a hard edge (guard {LAMP_DISC}): z {z} vis {vis:.3} luma {y:.1} lit {lit:.1} dark {dark:.1} place {place:.3}"
    );
}

#[test]
fn the_penumbra_matches_the_disc_and_holds_still() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let world = World::from_scene(scene());
    let camera = camera();
    let scene = scene();
    renderer.set_lighting_settings(settings());
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Direct,
        lamp: 0,
    });
    for _ in 0..WARMUP {
        let _ = grab(window, renderer, &world, &camera);
    }
    let mut frames = Vec::new();
    for _ in 0..AVERAGE {
        frames.push(grab(window, renderer, &world, &camera));
    }
    let width = u16::try_from(frames[0].width).expect("width");
    let height = u16::try_from(frames[0].height).expect("height");
    let aspect = f32::from(width) / f32::from(height);
    let mut rows = Vec::new();
    let mut on_screen = 0u32;
    for step in 0u16..28 {
        let z = 1.50 + f32::from(step) * 0.05;
        let uv = viewport_uv(&camera, aspect, [0.0, 0.0, z]);
        let Some(uv) = uv else {
            continue;
        };
        on_screen += 1;
        let mut gpu = 0.0;
        let mut hits = 0u32;
        for frame in &frames {
            if let Some((y, _)) = pixel(frame, uv) {
                gpu += y;
                hits += 1;
            }
        }
        if hits == 0 {
            continue;
        }
        gpu /= f64::from(hits);
        let cpu = reference_at(&scene, z);
        rows.push((z, gpu, cpu));
    }
    assert!(
        on_screen >= 10,
        "the penumbra left the view ({on_screen} samples)"
    );
    let (mid_z, lit_gpu) = grade_profile(&rows);
    let uv = viewport_uv(&camera, aspect, [0.0, 0.0, mid_z]).expect("mid penumbra off screen");
    let mut previous: Option<f64> = None;
    let mut worst = 0.0_f64;
    for _ in 0..TEMPORAL {
        let read = grab(window, renderer, &world, &camera);
        let (y, _) = pixel(&read, uv).expect("mid penumbra missed the floor");
        if let Some(before) = previous {
            worst = worst.max((y - before).abs() / lit_gpu);
        }
        previous = Some(y);
    }
    assert!(
        worst < FRAME_STEP,
        "penumbra flicker {worst} of the lit value at z {mid_z} (limit {FRAME_STEP})"
    );
}

fn grade_profile(rows: &[(f32, f64, f64)]) -> (f32, f64) {
    assert!(rows.len() >= 8, "too few shaded samples: {}", rows.len());
    let lit_gpu = rows.iter().map(|(_, g, _)| *g).fold(0.0_f64, f64::max);
    let lit_cpu = rows.iter().map(|(_, _, c)| *c).fold(0.0_f64, f64::max);
    assert!(lit_gpu > 1.0e-3 && lit_cpu > 1.0e-3, "no lit sample gpu {lit_gpu} cpu {lit_cpu}");
    let scale = lit_gpu / (lit_cpu * LAMBERT);
    assert!(
        (scale - 1.0).abs() < 0.20,
        "a fully lit pixel left the centre irradiance: gpu {lit_gpu} cpu {lit_cpu} scale {scale}"
    );
    let mut err = 0.0;
    let mut n = 0.0;
    let mut vis = Vec::new();
    for &(z, gpu, cpu) in rows {
        let gv = gpu / lit_gpu;
        let cv = if lit_cpu > 0.0 { cpu / lit_cpu } else { 0.0 };
        err += (gv - cv) * (gv - cv);
        n += 1.0;
        vis.push((f64::from(z), gv));
    }
    let rmse = (err / n).sqrt();
    assert!(
        rmse < PROFILE_RMSE,
        "penumbra profile rmse {rmse} (limit {PROFILE_RMSE}) rows {rows:?}"
    );
    let gpu_cross = crossing(&vis).expect("gpu profile has no 0.5 crossing");
    let cpu_vis: Vec<(f64, f64)> = rows
        .iter()
        .map(|(z, _, c)| (f64::from(*z), c / lit_cpu))
        .collect();
    let cpu_cross = crossing(&cpu_vis).expect("reference profile has no 0.5 crossing");
    assert!(
        (gpu_cross - cpu_cross).abs() < WIDTH_TOL,
        "penumbra centre gpu {gpu_cross} reference {cpu_cross}"
    );
    let mut mid_z = rows[0].0;
    let mut mid_gap = 1.0;
    for &(z, _, cpu) in rows {
        let gap = (cpu / lit_cpu - 0.5).abs();
        if gap < mid_gap {
            mid_gap = gap;
            mid_z = z;
        }
    }
    assert!(
        mid_gap < 0.25,
        "no partial sample in view (closest gap {mid_gap} at z {mid_z})"
    );
    (mid_z, lit_gpu)
}
