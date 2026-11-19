#![cfg_attr(
    test,
    expect(clippy::expect_used, reason = "a test aborts when a fixture fails")
)]
//! A flat wall lit only by one moving point lamp.
//!
//! Sun-only pixels stay smooth because the sun is one exact shadow ray. This
//! wall has no sun. The still lamp is the converged reference. While the lamp
//! moves, pixel luma variance (high-pass residual) and the frame-to-frame
//! residual must stay on that reference. A short reservoir mean paints the
//! wall with pixel-to-pixel grain.

use genos_render::{lamp_range, tone, DebugView, Renderer, ViewMode, World};
use genos_scene::{AmbientMode, Camera, LampPickMode, LightingPreset, LightingSettings, Scene};
use genos_window::Window;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
/// Frames with the lamp parked, so the direct mean can settle.
const WARMUP: u32 = 16;
/// Still pairs after warmup. Their residual is the reference.
const STILL_PAIRS: u32 = 4;
/// Frames the lamp travels. Every one must stay near the still residual.
const MOVE_FRAMES: u32 = 8;
const MOVE_STEP: f32 = 0.1;
const WALL_Z: f32 = 0.15;
const LAMP_Y: f32 = 2.0;
const LAMP_Z: f32 = 1.7;
const LAMP_X0: f32 = -0.6;
/// Metres. The wall patch stays inside the ball for the whole travel.
const LAMP_RANGE: f32 = 8.0;
/// Moving error over the still-lamp error. The unfixed short mean is several
/// times the settled wall. The floor is below a lit wall's numerical noise.
const GRAIN_RATIO: f64 = 1.35;
const GRAIN_FLOOR: f64 = 1.0e-9;

struct Rect {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

#[derive(Clone, Copy, Debug)]
struct Grain {
    /// Variance of (pixel − neighbour mean) on the wall. The lamp's smooth
    /// falloff is not this number.
    spatial: f64,
    /// Variance of per-pixel frame deltas after the patch mean is removed.
    /// A uniform brightening is zero. Grain is not.
    temporal: f64,
}

fn open() -> (
    std::sync::MutexGuard<'static, ()>,
    genos_window::Lifecycle<Renderer>,
) {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = GPU
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut window = Window::open_proof(WIDTH, HEIGHT).expect("proof window");
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height)
            .expect("renderer");
    (
        guard,
        genos_window::Lifecycle::new(window, renderer, permit),
    )
}

fn settings() -> LightingSettings {
    let mut settings = LightingSettings::preset(LightingPreset::Potato);
    settings.direct.max_lights = 0;
    settings.direct.refresh_frames = 1;
    settings.direct.resolution_scale = 1;
    settings.sky = false;
    settings.ambient.mode = AmbientMode::Off;
    settings.shadows.enabled = true;
    // The direct mean reprojects through the last camera, and that camera is
    // written only while GI is on (lamp_pick_reproject).
    settings.gi.enabled = true;
    settings.bounce.count = 1;
    settings.probes.enabled = false;
    settings.world_cache.enabled = true;
    settings.denoiser.enabled = false;
    settings.denoiser.screen = false;
    settings.lamp_pick.mode = LampPickMode::Reuse;
    settings.lamp_pick.radius = 0;
    settings.compose_reuse = false;
    settings.restir_di = true;
    settings.restir_spatial_neighbors = 4;
    settings
}

fn as_f32(value: u32) -> f32 {
    f32::from(u16::try_from(value).unwrap_or(0))
}

fn color_for_range(range: f32) -> [f32; 3] {
    let unit = lamp_range([1.0, 0.85, 0.55]).max(1.0e-3);
    let peak = 1.0_f32;
    let want = (range / unit) * (range / unit);
    let scale = want / peak;
    [1.0 * scale, 0.85 * scale, 0.55 * scale]
}

fn world() -> World {
    World::from_scene(Scene {
        floor: genos_scene::Floor {
            position: genos_scene::Vec3::new(0.0, 0.0, 3.0),
            half_x: 6.0,
            half_z: 6.0,
            color: [0.4, 0.4, 0.4],
        },
        walls: vec![genos_scene::Wall {
            position: genos_scene::Vec3::new(0.0, 0.0, 0.0),
            half_x: 4.0,
            half_z: 0.15,
            height: 4.0,
            base: 0.0,
            color: [0.72, 0.72, 0.72],
            absorption: 0.0,
            reflectance: 0.8,
            color_mix: -1.0,
        }],
        solids: Vec::new(),
        lights: vec![genos_scene::Light {
            position: genos_scene::Vec3::new(LAMP_X0, LAMP_Y, LAMP_Z),
            color: color_for_range(LAMP_RANGE),
            direction: genos_scene::Vec3::ZERO,
        }],
        ceiling: Some(genos_scene::Ceiling {
            height: 4.0,
            color: [0.5, 0.5, 0.5],
        }),
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
    })
}

fn camera() -> Camera {
    let mut camera = Camera::opening();
    camera.set_pose(genos_scene::Vec3::new(0.0, 1.6, 5.5), 0.0, 0.0);
    camera
}

fn screen_u(t: f32, span: u32) -> u32 {
    let scaled = (t * as_f32(span)).floor();
    if !scaled.is_finite() || scaled <= 0.0 {
        return 0;
    }
    let last = span.saturating_sub(1);
    let mut px = 0u32;
    while px < last {
        let next = f32::from(u16::try_from(px + 1).unwrap_or(u16::MAX));
        if next > scaled {
            break;
        }
        px += 1;
    }
    px
}

fn patch(camera: &Camera) -> Rect {
    let aspect = as_f32(WIDTH) / as_f32(HEIGHT);
    let mut min_x = WIDTH;
    let mut max_x = 0u32;
    let mut min_y = HEIGHT;
    let mut max_y = 0u32;
    for x in [-0.7_f32, 0.9] {
        for y in [1.0_f32, 2.6] {
            let uv =
                genos_scene::viewport_uv(camera, aspect, [x, y, WALL_Z]).expect("wall on screen");
            let px = screen_u(uv[0], WIDTH);
            let py = screen_u(uv[1], HEIGHT);
            min_x = min_x.min(px);
            max_x = max_x.max(px);
            min_y = min_y.min(py);
            max_y = max_y.max(py);
        }
    }
    assert!(
        max_x > min_x + 8 && max_y > min_y + 8,
        "patch is too small {min_x}..{max_x} {min_y}..{max_y}"
    );
    Rect {
        x0: min_x.saturating_add(2),
        y0: min_y.saturating_add(2),
        x1: max_x.saturating_sub(2),
        y1: max_y.saturating_sub(2),
    }
}

fn luma_image(rgb: &[f64]) -> Vec<f64> {
    let pixels = rgb.len() / 3;
    let mut out = Vec::with_capacity(pixels);
    for pixel in 0..pixels {
        let i = pixel.saturating_mul(3);
        let r = rgb.get(i).copied().unwrap_or(0.0);
        let g = rgb.get(i.saturating_add(1)).copied().unwrap_or(0.0);
        let b = rgb.get(i.saturating_add(2)).copied().unwrap_or(0.0);
        out.push(0.2126 * r + 0.7152 * g + 0.0722 * b);
    }
    out
}

fn index_of(width: u32, x: u32, y: u32) -> usize {
    usize::try_from(u64::from(y) * u64::from(width) + u64::from(x)).unwrap_or(0)
}

fn at(luma: &[f64], width: u32, x: u32, y: u32) -> f64 {
    luma.get(index_of(width, x, y)).copied().unwrap_or(0.0)
}

fn spatial_var(luma: &[f64], width: u32, rect: &Rect) -> f64 {
    let mut sum = 0.0;
    let mut sum_sq = 0.0;
    let mut count = 0u32;
    if rect.x1 <= rect.x0.saturating_add(2) || rect.y1 <= rect.y0.saturating_add(2) {
        return 0.0;
    }
    for y in rect.y0.saturating_add(1)..rect.y1.saturating_sub(1) {
        for x in rect.x0.saturating_add(1)..rect.x1.saturating_sub(1) {
            let here = at(luma, width, x, y);
            let neighbour = 0.25
                * (at(luma, width, x - 1, y)
                    + at(luma, width, x + 1, y)
                    + at(luma, width, x, y - 1)
                    + at(luma, width, x, y + 1));
            let residual = here - neighbour;
            sum += residual;
            sum_sq += residual * residual;
            count = count.saturating_add(1);
        }
    }
    if count == 0 {
        return 0.0;
    }
    let n = f64::from(count);
    let mean = sum / n;
    (sum_sq / n - mean * mean).max(0.0)
}

fn grab(window: &mut Window, renderer: &mut Renderer, world: &World, camera: &Camera) -> Vec<f64> {
    let _ = window.pump();
    renderer
        .draw(world, camera, true)
        .expect("draw")
        .expect("readback");
    let read = renderer.read_scene().expect("scene radiance");
    luma_image(&read.rgb)
}

fn place(world: &mut World, frame: u32) {
    if let Some(lamp) = world.scene.lights.first_mut() {
        lamp.position.x = LAMP_X0 + MOVE_STEP * as_f32(frame);
    }
}

fn mean_of(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / f64::from(u16::try_from(values.len()).unwrap_or(0))
}

fn patch_mean(luma: &[f64], width: u32, rect: &Rect) -> f64 {
    let mut sum = 0.0;
    let mut count = 0u32;
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            sum += at(luma, width, x, y);
            count = count.saturating_add(1);
        }
    }
    if count == 0 {
        0.0
    } else {
        sum / f64::from(count)
    }
}

fn luma8(y: f64) -> i32 {
    let shown = tone(genos_math::f32_from_f64(y.clamp(0.0, 4.0)));
    let code = (f64::from(shown).clamp(0.0, 1.0) * 255.0).round();
    let mut step = 0i32;
    let mut cursor = 0.0_f64;
    while step < 255 && cursor + 0.5 < code {
        step += 1;
        cursor += 1.0;
    }
    step
}

/// Mean absolute neighbour difference of the tonemapped luma, in 8-bit steps.
fn grain8(luma: &[f64], width: u32, rect: &Rect) -> f64 {
    let mut sum = 0.0;
    let mut count = 0u32;
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            let a = luma8(at(luma, width, x, y));
            if x + 1 < rect.x1 {
                let b = luma8(at(luma, width, x + 1, y));
                sum += f64::from((a - b).abs());
                count = count.saturating_add(1);
            }
            if y + 1 < rect.y1 {
                let b = luma8(at(luma, width, x, y + 1));
                sum += f64::from((a - b).abs());
                count = count.saturating_add(1);
            }
        }
    }
    if count == 0 {
        0.0
    } else {
        sum / f64::from(count)
    }
}

fn abs_luma(luma: &[f64]) -> Vec<f64> {
    luma.iter().map(|value| value.abs()).collect()
}

fn sub(live: &[f64], exact: &[f64]) -> Vec<f64> {
    let n = live.len().min(exact.len());
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        out.push(live.get(i).copied().unwrap_or(0.0) - exact.get(i).copied().unwrap_or(0.0));
    }
    out
}

#[test]
fn a_moving_point_lamp_does_not_grain_a_flat_wall() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let mut world = world();
    let camera = camera();
    let wall = patch(&camera);
    renderer.set_lighting_settings(settings());
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Direct,
        lamp: 0,
    });
    place(&mut world, 0);
    for _ in 0..WARMUP {
        let _ = grab(window, renderer, &world, &camera);
    }
    let mut still_frames = Vec::new();
    for _ in 0..STILL_PAIRS {
        let now = grab(window, renderer, &world, &camera);
        still_frames.push(now);
    }
    let mut move_frames = Vec::new();
    for frame in 1..=MOVE_FRAMES {
        place(&mut world, frame);
        let now = grab(window, renderer, &world, &camera);
        move_frames.push(now);
    }
    let mut exact_off = settings();
    exact_off.restir_di = false;
    renderer.set_lighting_settings(exact_off);
    place(&mut world, 0);
    let still_exact = grab(window, renderer, &world, &camera);
    let mut move_exact = Vec::new();
    for frame in 1..=MOVE_FRAMES {
        place(&mut world, frame);
        move_exact.push(grab(window, renderer, &world, &camera));
    }
    grade_wall(&wall, &still_frames, &move_frames, &still_exact, &move_exact);
}

fn grade_wall(
    wall: &Rect,
    still_frames: &[Vec<f64>],
    move_frames: &[Vec<f64>],
    still_exact: &[f64],
    move_exact: &[Vec<f64>],
) {
    let still_live = still_frames.last().expect("still frame");
    let still_err = sub(still_live, still_exact);
    let still_mean = patch_mean(still_live, WIDTH, wall);
    let exact_mean = patch_mean(still_exact, WIDTH, wall);
    assert!(
        still_mean > 0.02 && exact_mean > 0.02,
        "the wall is dark: live {still_mean:.4} exact {exact_mean:.4}"
    );
    let mut still_step = Vec::new();
    let mut earlier = still_frames.first().expect("still pair");
    for frame in still_frames.iter().skip(1) {
        still_step.push(spatial_var(&sub(frame, earlier), WIDTH, wall));
        earlier = frame;
    }
    let still = Grain {
        spatial: spatial_var(&still_err, WIDTH, wall),
        temporal: mean_of(&still_step),
    };
    let mut move_spatial = Vec::new();
    let mut move_temporal = Vec::new();
    let mut move_grain = Vec::new();
    let still_grain = grain8(&abs_luma(&still_err), WIDTH, wall);
    let mut prev_err = still_err;
    for (index, (live, exact)) in move_frames.iter().zip(move_exact).enumerate() {
        let err = sub(live, exact);
        let spatial = spatial_var(&err, WIDTH, wall);
        let temporal = spatial_var(&sub(&err, &prev_err), WIDTH, wall);
        let grain = grain8(&abs_luma(&err), WIDTH, wall);
        let frame = index + 1;
        let live_mean = patch_mean(live, WIDTH, wall);
        let exact_frame = patch_mean(exact, WIDTH, wall);
        eprintln!(
            "moving-lamp frame {frame}: error spatial {spatial:.6e} temporal {temporal:.6e} grain8 {grain:.3} live {live_mean:.4} exact {exact_frame:.4}"
        );
        move_spatial.push(spatial);
        move_temporal.push(temporal);
        move_grain.push(grain);
        prev_err = err;
    }
    let moving = Grain {
        spatial: mean_of(&move_spatial),
        temporal: mean_of(&move_temporal),
    };
    let moving_grain = mean_of(&move_grain);
    let still_spatial = still.spatial;
    let still_temporal = still.temporal;
    let moving_spatial = moving.spatial;
    let moving_temporal = moving.temporal;
    eprintln!(
        "moving-lamp grain: still spatial {still_spatial:.6e} temporal {still_temporal:.6e} grain8 {still_grain:.3}; moving spatial {moving_spatial:.6e} temporal {moving_temporal:.6e} grain8 {moving_grain:.3}; wall {still_mean:.4}"
    );
    let spatial_limit = still_spatial * GRAIN_RATIO + GRAIN_FLOOR;
    let temporal_limit = still_temporal * GRAIN_RATIO + GRAIN_FLOOR;
    let grain_limit = still_grain * GRAIN_RATIO + 0.05;
    assert!(
        moving_spatial <= spatial_limit,
        "wall luma variance {moving_spatial:.6e} above the still lamp {still_spatial:.6e} (limit {spatial_limit:.6e})"
    );
    assert!(
        moving_temporal <= temporal_limit,
        "frame-to-frame residual {moving_temporal:.6e} above the still lamp {still_temporal:.6e} (limit {temporal_limit:.6e})"
    );
    assert!(
        moving_grain <= grain_limit,
        "8-bit error grain {moving_grain:.3} above the still lamp {still_grain:.3} (limit {grain_limit:.3})"
    );
}
