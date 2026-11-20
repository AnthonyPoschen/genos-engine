#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! Far-wall stability: every inserted level-0 voxel is traced, publish
//! leaves no empty sample, and a slow yaw does not make distant wall
//! blocks jump from frame to frame.

use genos_render::{DebugView, Renderer, ViewMode, World};
use genos_scene::{
    AmbientMode, Camera, Ceiling, Floor, Light, LightingPreset, LightingSettings, Scene, Vec3, Wall,
};
use genos_window::Window;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const STILL: u32 = 24;
const YAW_FRAMES: u32 = 8;
const YAW: f32 = 0.2;
const MEAN_STEP: f64 = 0.08;
const PIXEL_STEP: f64 = 0.22;

fn open() -> (
    std::sync::MutexGuard<'static, ()>,
    genos_window::Lifecycle<Renderer>,
) {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = GPU.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut window = Window::open_proof(WIDTH, HEIGHT).expect("proof window");
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height).expect("renderer");
    (guard, genos_window::Lifecycle::new(window, renderer, permit))
}

fn settings() -> LightingSettings {
    let mut settings = LightingSettings::preset(LightingPreset::Potato);
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

fn scene() -> Scene {
    Scene {
        floor: Floor {
            position: Vec3::ZERO,
            half_x: 8.0,
            half_z: 16.0,
            color: [0.72, 0.72, 0.72],
        },
        walls: vec![
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
        solids: Vec::new(),
        lights: vec![Light {
            position: Vec3::new(0.0, 3.4, 0.0),
            color: [5.0, 4.6, 4.0],
            direction: Vec3::ZERO,
        }],
        ceiling: Some(Ceiling {
            height: 4.0,
            color: [0.45, 0.45, 0.45],
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
    }
}

fn hall_camera(yaw: f32) -> Camera {
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(0.0, 1.6, 12.0), yaw, 0.05);
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

/// Centre patch of the far wall. Under a 60° vFOV at ~28 m the 4 m wall
/// only fills a short mid-frame band; the middle third also covers the
/// lamp-lit ceiling and floor, whose screen motion is not the block
/// shimmer this test is for.
fn far_patch(luma: &[f64]) -> (f64, Vec<f64>) {
    let mut sum = 0.0;
    let mut vals = Vec::new();
    let x0 = WIDTH / 3;
    let x1 = (WIDTH * 2) / 3;
    let y0 = (HEIGHT * 82) / 180;
    let y1 = (HEIGHT * 105) / 180;
    for y in y0..y1 {
        for x in x0..x1 {
            let i = usize::try_from(y * WIDTH + x).expect("pixel");
            let v = luma.get(i).copied().unwrap_or(0.0);
            sum += v;
            vals.push(v);
        }
    }
    let n = f64::from(u32::try_from(vals.len()).expect("patch"));
    (sum / n.max(1.0), vals)
}

fn grab(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
) -> (Vec<f64>, u32, u32, u32) {
    let _ = window.pump();
    renderer.draw(world, camera, true).expect("draw").expect("readback");
    let luma = luma(&renderer.read_scene().expect("scene radiance").rgb);
    let (inserted, traced) = renderer.lighting_level0_counts().expect("counts");
    let gaps = renderer.lighting_cascade_gaps();
    (luma, inserted, traced, gaps)
}

/// Every inserted voxel is traced, and no visible surface is left empty.
#[test]
fn every_inserted_level0_voxel_is_traced() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let world = World::from_scene(scene());
    renderer.set_lighting_settings(settings());
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Full,
        lamp: 0,
    });
    let cam = hall_camera(0.0);
    let mut last = (0, 0, 0);
    for _ in 0..STILL {
        let (_, inserted, traced, gaps) = grab(window, renderer, &world, &cam);
        last = (inserted, traced, gaps);
    }
    let (inserted, traced, gaps) = last;
    let voxels = renderer.lighting_level0_voxels().expect("voxels");
    eprintln!(
        "still counts inserted {inserted} traced {traced} gaps {gaps} unique {}",
        voxels.len()
    );
    assert!(inserted >= 64, "too few inserted voxels: {inserted}");
    assert_eq!(
        voxels.len(),
        usize::try_from(traced).expect("traced"),
        "unique table keys {} != traced {traced}",
        voxels.len()
    );
    assert_eq!(
        traced, inserted,
        "traced {traced} != inserted {inserted}"
    );
    assert_eq!(gaps, 0, "diag 11 (empty published samples) is {gaps}");
}

/// A slow yaw across a distant wall must not make the patch jump as hard blocks.
#[test]
fn a_far_wall_stays_stable_during_a_yaw() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let world = World::from_scene(scene());
    renderer.set_lighting_settings(settings());
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Full,
        lamp: 0,
    });
    let mut prev = None;
    let mut worst_mean = 0.0f64;
    let mut worst_pixel = 0.0f64;
    for step in 0..=YAW_FRAMES {
        let t = f32::from(u16::try_from(step).expect("step"))
            / f32::from(u16::try_from(YAW_FRAMES).expect("frames"));
        let cam = hall_camera(YAW * t);
        let (luma, inserted, traced, gaps) = grab(window, renderer, &world, &cam);
        if step == 0 {
            for _ in 0..STILL {
                let _ = grab(window, renderer, &world, &cam);
            }
            let (luma, inserted, traced, gaps) = grab(window, renderer, &world, &cam);
            assert_eq!(traced, inserted, "settle traced {traced} != inserted {inserted}");
            assert_eq!(gaps, 0, "settle gaps {gaps}");
            let patch = far_patch(&luma);
            eprintln!("settle patch mean {}", patch.0);
            prev = Some(patch);
            continue;
        }
        assert_eq!(traced, inserted, "yaw traced {traced} != inserted {inserted}");
        assert_eq!(gaps, 0, "yaw step {step} gaps {gaps}");
        let now = far_patch(&luma);
        let before = prev.replace(now.clone()).expect("previous patch");
        let mean_step = (now.0 - before.0).abs();
        let mut pixel_step = 0.0f64;
        for (a, b) in before.1.iter().zip(now.1.iter()) {
            pixel_step = pixel_step.max((a - b).abs());
        }
        eprintln!(
            "yaw step {step}: mean {} step {mean_step:.4} pixel {pixel_step:.4} inserted {inserted}",
            now.0
        );
        worst_mean = worst_mean.max(mean_step);
        worst_pixel = worst_pixel.max(pixel_step);
    }
    assert!(
        worst_mean <= MEAN_STEP,
        "far-wall mean jumped {worst_mean} (limit {MEAN_STEP})"
    );
    assert!(
        worst_pixel <= PIXEL_STEP,
        "far-wall pixel jumped {worst_pixel} (limit {PIXEL_STEP})"
    );
}
