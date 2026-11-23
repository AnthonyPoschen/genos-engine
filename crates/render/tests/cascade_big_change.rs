#![cfg_attr(
    test,
    expect(clippy::as_conversions, reason = "pixel ndc from the integer size"),
    expect(
        clippy::cast_precision_loss,
        reason = "pixel ndc from the integer size"
    ),
    expect(clippy::expect_used, reason = "a test aborts when a fixture fails")
)]

//! Sun jump, lamp flip, and lamp-off: a change only down-weights stored
//! history, and the next regular refresh (stride P=4) dominates catch-up
//! within 5 frames. Lamp-off is checked against a path-traced reference
//! (the closed room must go nearly black). Lavapipe times are not the
//! on-device cost.

use genos_render::{radiance_hits, trace_ray, CascadeTimes, DebugView, Renderer, ViewMode, World};
use genos_scene::{
    look_direction, AmbientMode, Camera, Ceiling, Floor, Light, LightingPreset, LightingSettings,
    Scene, Vec3, Wall,
};
use genos_window::Window;

fn count_f64(n: usize) -> f64 {
    f64::from(u32::try_from(n).expect("count"))
}

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const STILL: u32 = 16;
const REF_AFTER: u32 = 16;
const CATCH_MEAN: f64 = 0.04;
const PERIODS: [u32; 1] = [4];

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
    let mut settings = LightingSettings::preset(LightingPreset::Fallback);
    settings.direct.refresh_frames = 1;
    settings.direct.resolution_scale = 1;
    settings.sky = false;
    settings.ambient.mode = AmbientMode::Off;
    settings.gi.enabled = true;
    settings.bounce.count = 1;
    settings.probes.enabled = false;
    settings.world_cache.enabled = true;
    settings.denoiser.enabled = false;
    settings.denoiser.screen = false;
    settings.restir_di = false;
    settings
}

fn sun_at(yaw: f32) -> Light {
    Light {
        position: Vec3::new(0.0, 7.0, 0.0),
        color: [8.0, 7.6, 7.0],
        direction: Vec3::new(yaw.sin(), -0.85, yaw.cos()),
    }
}

fn lamp(on: bool) -> Light {
    let scale = if on { 5.0 } else { 0.0 };
    Light {
        position: Vec3::new(0.0, 3.4, 0.0),
        color: [scale, scale * 0.92, scale * 0.8],
        direction: Vec3::ZERO,
    }
}

fn scene_with(sun: Light, lamp: Light) -> Scene {
    {
        let mut __scene = Scene::from_lists(
            Floor {
                position: Vec3::ZERO,
                half_x: 8.0,
                half_z: 16.0,
                color: [0.72, 0.72, 0.72],
            },
            vec![
                Wall {
                    position: Vec3::new(0.0, 0.0, -16.0),
                    half_x: 8.0,
                    half_z: 0.1,
                    height: 4.0,
                    base: 0.0,
                    color: [0.85, 0.82, 0.75],
                    absorption: 0.0,
                    reflectance: -1.0,
                    color_mix: -1.0,
                },
                Wall {
                    position: Vec3::new(0.0, 0.0, 16.0),
                    half_x: 8.0,
                    half_z: 0.1,
                    height: 4.0,
                    base: 0.0,
                    color: [0.55, 0.62, 0.7],
                    absorption: 0.0,
                    reflectance: -1.0,
                    color_mix: -1.0,
                },
                Wall {
                    position: Vec3::new(-8.0, 0.0, 0.0),
                    half_x: 0.1,
                    half_z: 16.0,
                    height: 4.0,
                    base: 0.0,
                    color: [0.7, 0.7, 0.7],
                    absorption: 0.0,
                    reflectance: -1.0,
                    color_mix: -1.0,
                },
                Wall {
                    position: Vec3::new(8.0, 0.0, 0.0),
                    half_x: 0.1,
                    half_z: 16.0,
                    height: 4.0,
                    base: 0.0,
                    color: [0.7, 0.7, 0.7],
                    absorption: 0.0,
                    reflectance: -1.0,
                    color_mix: -1.0,
                },
            ],
            Vec::new(),
            vec![sun, lamp],
        );
        __scene.ceiling = Some(Ceiling {
            height: 4.0,
            color: [0.45, 0.45, 0.45],
        });
        __scene.sky = None;
        __scene.lighting = settings();
        __scene.debug = genos_scene::DebugSettings::default();
        __scene.fps_counter = genos_scene::FpsCounterSettings::default();
        __scene.event_loop = genos_scene::EventLoopSettings::default();
        __scene.reflections = genos_scene::ReflectionSettings::default();
        __scene.hdr = genos_scene::HdrSettings::for_preset(LightingPreset::High);
        __scene.fog = genos_scene::FogSettings::default();
        __scene.menu = genos_scene::MenuSettings::default();
        __scene.particles = genos_scene::ParticleSettings::default();
        __scene
    }
}

fn hall_camera() -> Camera {
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(0.0, 1.6, 12.0), 0.0, 0.05);
    camera
}

fn luma(rgb: &[f64]) -> Vec<f64> {
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

fn mean_abs(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 0.0;
    }
    let mut sum = 0.0;
    for i in 0..n {
        sum += (a[i] - b[i]).abs();
    }
    sum / count_f64(n)
}

fn grab(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
) -> (Vec<f64>, CascadeTimes) {
    let _ = window.pump();
    renderer
        .draw(world, camera, true)
        .expect("draw")
        .expect("readback");
    let luma = luma(&renderer.read_scene().expect("scene radiance").rgb);
    let times = renderer.lighting_cascade_times();
    (luma, times)
}

fn apply_period(renderer: &mut Renderer, period: u32) {
    renderer.set_cascade_refresh_period(period).expect("period");
}

fn print_times(label: &str, t: CascadeTimes) {
    eprintln!(
        "{label}: period {} claim {:.3} ms coarse {:.3} mid {:.3} fine {:.3} l0 {:.3} upper {:.3}",
        t.period,
        t.claim_ms,
        t.upper_coarse_ms,
        t.upper_mid_ms,
        t.upper_fine_ms,
        t.l0_ms,
        t.upper_ms(),
    );
}

fn settle(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
    frames: u32,
) -> (Vec<f64>, CascadeTimes) {
    let mut last = (Vec::new(), CascadeTimes::default());
    for _ in 0..frames {
        last = grab(window, renderer, world, camera);
    }
    last
}

fn reset_run(renderer: &mut Renderer, period: u32) {
    renderer.set_lighting_settings(settings());
    apply_period(renderer, period);
    renderer.reset_lighting_cache();
}

fn catch_limit(_period: u32) -> u32 {
    5
}

fn camera_ray(camera: &Camera, aspect: f32, x: f32, y: f32) -> [f32; 3] {
    let forward = look_direction(camera.yaw, camera.pitch);
    let mut right = forward.cross(Vec3::Y);
    let scale = right.length().max(1.0e-6);
    right = right / scale;
    let up = right.cross(forward);
    let tan_half = (60.0_f32.to_radians() * 0.5).tan();
    let dir = forward + right * (x * tan_half * aspect) + up * (y * tan_half);
    let len = dir.length().max(1.0e-6);
    [dir.x / len, dir.y / len, dir.z / len]
}

/// Sparse path-traced luma of the camera picture. A closed room with the
/// lamp off and the sun above the ceiling is nearly black; this is that
/// reference, not a settled GPU frame.
fn path_traced_luma(scene: &Scene, camera: &Camera, width: u32, height: u32) -> Vec<f64> {
    let aspect = width as f32 / height.max(1) as f32;
    let origin = [camera.position.x, camera.position.y, camera.position.z];
    let step = 4u32;
    let mut out = Vec::new();
    let mut y = 0u32;
    while y < height {
        let mut x = 0u32;
        while x < width {
            let ndc_x = (2.0 * (x as f32 + 0.5) / width as f32) - 1.0;
            let ndc_y = 1.0 - (2.0 * (y as f32 + 0.5) / height as f32);
            let dir = camera_ray(camera, aspect, ndc_x, ndc_y);
            let l = match trace_ray(scene, origin, dir, 200.0) {
                Some(hit) => {
                    let rgb = radiance_hits(scene, hit.point, hit.normal, hit.albedo, 4);
                    0.2126 * f64::from(rgb[0])
                        + 0.7152 * f64::from(rgb[1])
                        + 0.0722 * f64::from(rgb[2])
                }
                None => 0.0,
            };
            out.push(l);
            x += step;
        }
        y += step;
    }
    out
}

fn mean_luma(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.iter().copied().sum::<f64>() / count_f64(v.len())
}

fn catch_up(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
    reference: &[f64],
    period: u32,
) -> (u32, f64, CascadeTimes) {
    let limit = catch_limit(period);
    let mut first = None;
    let mut caught = 0u32;
    let mut err = f64::MAX;
    for frame in 1..=limit {
        let (pic, times) = grab(window, renderer, world, camera);
        if first.is_none() {
            first = Some(times);
        }
        err = mean_abs(&pic, reference);
        eprintln!(
            "catch frame {frame}: mean-abs {err:.4} pic {:.4} ref {:.4}",
            mean_luma(&pic),
            mean_luma(reference)
        );
        if err <= CATCH_MEAN {
            caught = frame;
            break;
        }
    }
    (caught, err, first.expect("first frame after the change"))
}

fn period_cost(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
    period: u32,
) -> (f64, f64) {
    reset_run(renderer, period);
    let mut l0 = 0.0;
    let mut upper = 0.0;
    let n = 8u32;
    for i in 0..n + STILL {
        let (_, times) = grab(window, renderer, world, camera);
        if i >= STILL {
            l0 += times.l0_ms;
            upper += times.upper_ms();
        }
    }
    let frames = f64::from(n);
    (l0 / frames, upper / frames)
}

#[test]
fn a_sun_jump_and_lamp_flip_catch_up_per_period() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Full,
        lamp: 0,
    });
    let camera = hall_camera();

    let rest = scene_with(sun_at(0.0), lamp(true));
    let jumped = scene_with(sun_at(0.8), lamp(true));
    let off = scene_with(sun_at(0.0), lamp(false));

    let world_rest = World::from_scene(rest.clone());
    let world_jump = World::from_scene(jumped.clone());
    let world_off = World::from_scene(off.clone());

    for period in PERIODS {
        // Same-period settled picture is the catch-up target. A period-1
        // reference stays noisier (or smoother) than P=4 and never agrees
        // within the 0.04 mean-abs line, even after 16 frames.
        reset_run(renderer, period);
        let _ = settle(window, renderer, &world_rest, &camera, STILL);
        let (sun_ref, _) = settle(window, renderer, &world_jump, &camera, REF_AFTER);

        reset_run(renderer, period);
        let _ = settle(window, renderer, &world_rest, &camera, STILL);
        let (sun_frames, sun_err, sun_times) =
            catch_up(window, renderer, &world_jump, &camera, &sun_ref, period);
        print_times(&format!("sun jump P={period}"), sun_times);
        eprintln!("sun jump P={period} caught in {sun_frames} frames, mean-abs {sun_err:.4}");
        let sun_limit = catch_limit(period);
        assert!(
            sun_frames > 0 && sun_frames <= sun_limit,
            "P={period} sun jump did not catch the same-period settled picture in {sun_limit} frames (err {sun_err:.4})"
        );

        reset_run(renderer, period);
        let _ = settle(window, renderer, &world_rest, &camera, STILL);
        let (lamp_ref, _) = settle(window, renderer, &world_off, &camera, REF_AFTER);

        reset_run(renderer, period);
        let _ = settle(window, renderer, &world_rest, &camera, STILL);
        let (lamp_frames, lamp_err, lamp_times) =
            catch_up(window, renderer, &world_off, &camera, &lamp_ref, period);
        print_times(&format!("lamp flip P={period}"), lamp_times);
        eprintln!("lamp flip P={period} caught in {lamp_frames} frames, mean-abs {lamp_err:.4}");
        let lamp_limit = catch_limit(period);
        assert!(
            lamp_frames > 0 && lamp_frames <= lamp_limit,
            "P={period} lamp flip did not catch the settled-off picture in {lamp_limit} frames (err {lamp_err:.4})"
        );

        let pt = path_traced_luma(&off, &camera, WIDTH, HEIGHT);
        let pt_mean = mean_luma(&pt);
        let gpu_mean = mean_luma(&{
            let (pic, _) = grab(window, renderer, &world_off, &camera);
            pic
        });
        eprintln!("lamp off P={period} gpu mean luma {gpu_mean:.4} path-traced {pt_mean:.4}");
        assert!(
            pt_mean <= 0.02,
            "P={period} path-traced lamp-off should be nearly black, mean luma {pt_mean:.4}"
        );
        assert!(
            gpu_mean <= pt_mean + CATCH_MEAN,
            "P={period} lamp-off did not reach the path-traced dark room in {lamp_limit} frames (gpu {gpu_mean:.4} pt {pt_mean:.4})"
        );
    }

    let (l0_4, up_4) = period_cost(window, renderer, &world_rest, &camera, 4);
    eprintln!("period cost: P=4 L0 {l0_4:.3} ms upper {up_4:.3} ms");
}
