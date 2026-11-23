#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! Level-0 footprint sizing: still counts, inset uniqueness, a pure yaw
//! must not churn keys, and a still camera must not change levels.

use genos_render::{DebugView, Level0Voxel, Renderer, ViewMode, World};
use genos_scene::{
    look_direction, AmbientMode, Camera, Ceiling, Floor, Light, LightingPreset, LightingSettings,
    Scene, Vec3, Wall,
};
use genos_window::Window;
use std::collections::BTreeSet;

const STILL: u32 = 32;
const YAW_FRAMES: u32 = 8;
const YAW: f32 = 0.35;
const COUNT_FRAC: f64 = 0.20;
const CHURN_FRAC: f64 = 0.08;
const STILL_FRAC: f64 = 0.01;

fn count_f64(n: usize) -> f64 {
    f64::from(u32::try_from(n).expect("count"))
}

fn open(
    width: u32,
    height: u32,
) -> (
    std::sync::MutexGuard<'static, ()>,
    genos_window::Lifecycle<Renderer>,
) {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = GPU.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut window = Window::open_proof(width, height).expect("proof window");
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height).expect("renderer");
    (guard, genos_window::Lifecycle::new(window, renderer, permit))
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

fn scene() -> Scene {
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
            vec![Light {
                position: Vec3::new(0.0, 3.4, 0.0),
                color: [5.0, 4.6, 4.0],
                direction: Vec3::ZERO,
            }],
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

fn hall_camera(yaw: f32) -> Camera {
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(0.0, 1.6, 12.0), yaw, 0.05);
    camera
}

fn floor_camera() -> Camera {
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(0.0, 1.6, 4.0), 0.0, -0.55);
    camera
}

fn grab(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
) -> Vec<Level0Voxel> {
    let _ = window.pump();
    renderer.draw(world, camera, true).expect("draw").expect("readback");
    renderer.lighting_level0_voxels().expect("level-0 readback")
}

fn settle(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
    frames: u32,
) -> Vec<Level0Voxel> {
    let mut voxels = Vec::new();
    for _ in 0..frames {
        voxels = grab(window, renderer, world, camera);
    }
    voxels
}

fn keys(voxels: &[Level0Voxel]) -> BTreeSet<(i32, i32, i32, u32, u32)> {
    voxels.iter().map(|voxel| voxel.key()).collect()
}

fn tangent_key(voxel: &Level0Voxel) -> (u32, u32, u32, (i32, i32)) {
    let axis = voxel.axis >> 1;
    let tangent = match axis {
        0 => (voxel.cell[1], voxel.cell[2]),
        1 => (voxel.cell[0], voxel.cell[2]),
        _ => (voxel.cell[0], voxel.cell[1]),
    };
    (axis, voxel.axis, voxel.level, tangent)
}

fn assert_no_normal_duplicates(voxels: &[Level0Voxel]) {
    let mut first = std::collections::BTreeMap::new();
    for voxel in voxels {
        let key = tangent_key(voxel);
        let prev = first.insert(key, *voxel);
        assert!(
            prev.is_none(),
            "surface cell appears twice along its normal: {prev:?} and {voxel:?}"
        );
    }
}

/// True when `pos` projects inside the frame with a 5% margin.
fn in_view(camera: &Camera, pos: [f32; 3], aspect: f32) -> bool {
    let forward = look_direction(camera.yaw, camera.pitch);
    let mut right = forward.cross(Vec3::Y);
    let scale = right.length().max(1.0e-6);
    right = right / scale;
    let up = right.cross(forward);
    let rel = Vec3::new(
        pos[0] - camera.position.x,
        pos[1] - camera.position.y,
        pos[2] - camera.position.z,
    );
    let depth = rel.dot(forward);
    if depth <= 0.05 {
        return false;
    }
    let t = 30.0_f32.to_radians().tan();
    let ndc_x = rel.dot(right) / (depth * t * aspect);
    let ndc_y = rel.dot(up) / (depth * t);
    ndc_x.abs() <= 0.9 && ndc_y.abs() <= 0.9
}

fn keys_in_view(voxels: &[Level0Voxel], camera: &Camera, aspect: f32) -> BTreeSet<(i32, i32, i32, u32, u32)> {
    voxels
        .iter()
        .filter(|voxel| in_view(camera, voxel.pos, aspect))
        .map(|voxel| voxel.key())
        .collect()
}

/// Still hall and floor poses keep a stable, unique level-0 set.
#[test]
fn level0_footprint_counts_are_stable_and_unique() {
    let (_gpu, mut owner) = open(320, 180);
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let world = World::from_scene(scene());
    renderer.set_lighting_settings(settings());
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Full,
        lamp: 0,
    });

    let hall_cam = hall_camera(0.0);
    let aspect = 320.0_f32 / 180.0_f32;
    let hall = settle(window, renderer, &world, &hall_cam, STILL);
    let hall_n = hall.len();
    assert!(hall_n >= 64, "too few hall voxels: {hall_n}");
    assert_no_normal_duplicates(&hall);
    let hall_again = settle(window, renderer, &world, &hall_cam, STILL);
    let hall_keys = keys_in_view(&hall, &hall_cam, aspect);
    let hall_keys_again = keys_in_view(&hall_again, &hall_cam, aspect);
    let changed = hall_keys
        .symmetric_difference(&hall_keys_again)
        .copied()
        .collect::<Vec<_>>();
    let still = count_f64(changed.len()) / count_f64(hall_keys.len().max(1));
    eprintln!(
        "still hall in-view {} then {} changed {} ({still:.4})",
        hall_keys.len(),
        hall_keys_again.len(),
        changed.len()
    );
    assert!(
        still <= STILL_FRAC,
        "a still camera changed {} in-view level-0 keys ({still}) over {STILL} frames: {changed:?}",
        changed.len()
    );

    let floor = settle(window, renderer, &world, &floor_camera(), STILL);
    let floor_n = floor.len();
    assert!(floor_n >= 64, "too few floor voxels: {floor_n}");
    assert_no_normal_duplicates(&floor);
    let mid = f64::midpoint(count_f64(hall_n), count_f64(floor_n));
    let span = (count_f64(hall_n) - count_f64(floor_n)).abs() / mid.max(1.0);
    eprintln!("footprint counts hall {hall_n} floor {floor_n} span {span:.3}");
    assert!(
        span <= COUNT_FRAC,
        "hall {hall_n} and floor {floor_n} counts differ by {span} (limit {COUNT_FRAC})"
    );
}

/// Pure yaw must not regenerate level-0 keys on surfaces that stay in view.
#[test]
fn a_pure_yaw_does_not_churn_level0_keys() {
    let (_gpu, mut owner) = open(320, 180);
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let world = World::from_scene(scene());
    renderer.set_lighting_settings(settings());
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Full,
        lamp: 0,
    });

    let start_cam = hall_camera(0.0);
    let start = settle(window, renderer, &world, &start_cam, STILL);
    let aspect = 320.0_f32 / 180.0_f32;
    let start_keys = keys_in_view(&start, &start_cam, aspect);
    assert!(start_keys.len() >= 64, "too few start voxels: {}", start_keys.len());

    let mut worst = 0.0f64;
    for step in 1..=YAW_FRAMES {
        let t = f32::from(u16::try_from(step).expect("step")) / f32::from(u16::try_from(YAW_FRAMES).expect("frames"));
        let cam = hall_camera(YAW * t);
        let now = grab(window, renderer, &world, &cam);
        let stay = start_keys
            .intersection(&keys_in_view(&start, &cam, aspect))
            .copied()
            .collect::<BTreeSet<_>>();
        let now_keys = keys(&now);
        let shared = stay.intersection(&now_keys).count();
        let denom = stay.len().max(1);
        let lost = 1.0
            - f64::from(u32::try_from(shared).expect("shared"))
                / f64::from(u32::try_from(denom).expect("stay"));
        eprintln!(
            "yaw step {step}: stay {} shared {shared} lost {lost:.3} total now {}",
            stay.len(),
            now.len()
        );
        worst = worst.max(lost);
    }
    assert!(
        worst <= CHURN_FRAC,
        "pure yaw churned level-0 keys: lost {worst} of the union (limit {CHURN_FRAC})"
    );
}
