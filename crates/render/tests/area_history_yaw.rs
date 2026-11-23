#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! A yaw across a shadow edge must not leave a ghost penumbra mean.
//!
//! `area_history` used to read the same screen pixel. On a flat floor the
//! depth and normal still matched, so a newly lit pixel kept the shadow
//! mean and faded it over many frames. After a reprojection to the previous
//! frame, those pixels reach the settled lit value in two frames.

use genos_render::{DebugView, Renderer, SceneRead, ViewMode, World};
use genos_scene::{
    look_direction, AmbientMode, Camera, Floor, LampPickMode, Light, LightingPreset,
    LightingSettings, Scene, Vec3, Wall,
};
use genos_window::Window;

const WARMUP: u32 = 32;
const AFTER: u32 = 2;
/// Yaw, radians. Enough to slide the wall shadow across a block of floor.
const YAW: f32 = 0.4;
/// Newly-lit pixels may sit this far from the settled luma, as a fraction of
/// the lit-minus-dark gap. A kept shadow mean after two frames is near 1.
const GAP_FRAC: f64 = 0.25;
const MIN_PIXELS: usize = 20;

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
    let mut settings = LightingSettings::preset(LightingPreset::Fallback);
    settings.direct.max_lights = 0;
    settings.direct.refresh_frames = 1;
    settings.direct.resolution_scale = 1;
    settings.direct.specular = false;
    settings.sky = false;
    settings.ambient.mode = AmbientMode::Off;
    settings.shadows.enabled = true;
    // Last camera is uploaded only when GI, a bounce, and the world cache
    // are on. The wall is black so the bounce does not colour the floor.
    settings.gi.enabled = true;
    settings.bounce.count = 1;
    settings.probes.enabled = false;
    settings.world_cache.enabled = true;
    settings.denoiser.enabled = false;
    settings.denoiser.screen = false;
    settings.lamp_pick.mode = LampPickMode::Off;
    settings.lamp_pick.radius = 0;
    settings.compose_reuse = false;
    settings.restir_di = false;
    settings
}

fn scene() -> Scene {
    {
        let mut __scene = Scene::from_lists(
            Floor {
                position: Vec3::ZERO,
                half_x: 8.0,
                half_z: 8.0,
                color: [1.0, 1.0, 1.0],
            },
            vec![Wall {
                position: Vec3::new(0.0, 0.0, 1.0),
                half_x: 2.0,
                half_z: 0.05,
                height: 1.5,
                base: 0.0,
                color: [0.0, 0.0, 0.0],
                absorption: 1.0,
                reflectance: -1.0,
                color_mix: -1.0,
            }],
            Vec::new(),
            vec![Light {
                position: Vec3::new(0.0, 3.0, 0.0),
                color: [1.0, 1.0, 1.0],
                direction: Vec3::ZERO,
            }],
        );
        __scene.ceiling = None;
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

fn camera(yaw: f32) -> Camera {
    let eye_y = 1.8_f32;
    let eye_z = 5.0_f32;
    let dy = -eye_y;
    let dz = 2.1 - eye_z;
    let pitch = (dy / (dy * dy + dz * dz).sqrt()).asin();
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(0.0, eye_y, eye_z), yaw, pitch);
    camera
}

fn grab(window: &mut Window, renderer: &mut Renderer, world: &World, camera: &Camera) -> SceneRead {
    let _ = window.pump();
    renderer.draw(world, camera, true).expect("draw").expect("readback");
    renderer.read_scene().expect("scene radiance")
}

fn settle(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
    frames: u32,
) -> SceneRead {
    let mut read = None;
    for _ in 0..frames {
        read = Some(grab(window, renderer, world, camera));
    }
    read.expect("settle drew no frame")
}

fn luma_at(read: &SceneRead, i: usize) -> Option<(f64, f64)> {
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
    Some((0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2], dist))
}

fn dim_f32(v: u32) -> f32 {
    f32::from(u16::try_from(v).expect("viewport dim"))
}

/// True when the eye ray at this pixel hits the floor. Distance is metres
/// from the eye. The ray matches the last-camera projection in
/// `lamp_pick_history_pixel`.
fn hits_floor(
    camera: &Camera,
    aspect: f32,
    col: u32,
    row: u32,
    width: u32,
    height: u32,
    dist: f64,
) -> bool {
    let ndc_x = (dim_f32(col) + 0.5) / dim_f32(width) * 2.0 - 1.0;
    let ndc_y = 1.0 - (dim_f32(row) + 0.5) / dim_f32(height) * 2.0;
    let forward = look_direction(camera.yaw, camera.pitch);
    let mut right = forward.cross(Vec3::Y);
    let scale = right.length().max(1.0e-6);
    right = right / scale;
    let up = right.cross(forward);
    let t = 30.0_f32.to_radians().tan();
    let dir = (forward + right * (ndc_x * t * aspect) + up * (ndc_y * t)).normalize();
    let hit_x = f64::from(camera.position.x) + f64::from(dir.x) * dist;
    let hit_y = f64::from(camera.position.y) + f64::from(dir.y) * dist;
    let hit_z = f64::from(camera.position.z) + f64::from(dir.z) * dist;
    hit_y.abs() <= 0.15 && hit_x.abs() <= 8.0 && hit_z.abs() <= 8.0
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let n = f64::from(u32::try_from(values.len()).expect("values"));
    values.iter().sum::<f64>() / n
}

/// Still lamp, camera yaw across a shadow edge: pixels newly in light reach
/// the settled value within two frames. A same-pixel mean would still be
/// close to the old shadow.
#[test]
fn a_yaw_across_a_shadow_leaves_no_ghost_mean() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let world = World::from_scene(scene());
    let before = camera(0.0);
    let after = camera(YAW);
    renderer.set_lighting_settings(settings());
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Direct,
        lamp: 0,
    });

    let settled = settle(window, renderer, &world, &after, WARMUP);
    let previous = settle(window, renderer, &world, &before, WARMUP);
    let mut live_read = None;
    for _ in 0..AFTER {
        live_read = Some(grab(window, renderer, &world, &after));
    }
    let live_read = live_read.expect("yaw drew no frame");

    let pixels = settled.rgb.len() / 3;
    assert_eq!(pixels, previous.rgb.len() / 3);
    assert_eq!(pixels, live_read.rgb.len() / 3);

    let width = settled.width;
    let height = settled.height;
    let aspect = dim_f32(width) / dim_f32(height);
    let mut floor = Vec::new();
    for i in 0..pixels {
        let Some((settled_luma, settled_dist)) = luma_at(&settled, i) else {
            continue;
        };
        let Some((previous_luma, previous_dist)) = luma_at(&previous, i) else {
            continue;
        };
        let Some((live_luma, live_dist)) = luma_at(&live_read, i) else {
            continue;
        };
        let i_u = u32::try_from(i).expect("pixel");
        let col = i_u % width;
        let row = i_u / width;
        if !hits_floor(&after, aspect, col, row, width, height, settled_dist)
            || !hits_floor(&before, aspect, col, row, width, height, previous_dist)
            || !hits_floor(&after, aspect, col, row, width, height, live_dist)
        {
            continue;
        }
        floor.push((settled_luma, previous_luma, live_luma));
    }
    assert!(
        floor.len() >= MIN_PIXELS * 4,
        "too few floor pixels to grade a yaw: {}",
        floor.len()
    );

    let mut sorted: Vec<f64> = floor.iter().map(|row| row.0).collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("luma"));
    let dark_cut = sorted[sorted.len() / 5];
    let lit_cut = sorted[(sorted.len() * 4) / 5];
    assert!(
        lit_cut > dark_cut * 1.5 + 1.0e-4,
        "no shadow contrast after the yaw: dark {dark_cut} lit {lit_cut}"
    );

    let mut newly = Vec::new();
    for (settled_luma, previous_luma, live_luma) in &floor {
        if *settled_luma >= lit_cut && *previous_luma <= dark_cut {
            newly.push((*settled_luma, *previous_luma, *live_luma));
        }
    }
    assert!(
        newly.len() >= MIN_PIXELS,
        "too few newly-lit pixels after a {YAW} rad yaw: {} (dark {dark_cut} lit {lit_cut}, floor {})",
        newly.len(),
        floor.len()
    );

    let settled_mean = mean(&newly.iter().map(|row| row.0).collect::<Vec<_>>());
    let previous_mean = mean(&newly.iter().map(|row| row.1).collect::<Vec<_>>());
    let live_mean = mean(&newly.iter().map(|row| row.2).collect::<Vec<_>>());
    let gap = settled_mean - previous_mean;
    assert!(
        gap > 0.0,
        "newly-lit pixels are not brighter when settled: settled {settled_mean} previous {previous_mean}"
    );
    let err = (live_mean - settled_mean).abs() / gap;
    eprintln!(
        "yaw ghost check: live {live_mean:.5} settled {settled_mean:.5} previous {previous_mean:.5} err {err:.4} pixels {}",
        newly.len()
    );
    assert!(
        err <= GAP_FRAC,
        "ghost shadow after {AFTER} frames: live {live_mean} settled {settled_mean} previous {previous_mean} err {err} (limit {GAP_FRAC}) pixels {}",
        newly.len()
    );
}
