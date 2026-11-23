#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! Band-seam continuity: the same surface point resolved at two adjacent
//! footprint levels must agree, and a still far wall must not shimmer.

use genos_render::{DebugView, Level0Voxel, Renderer, ViewMode, World};
use genos_scene::{
    AmbientMode, Camera, Ceiling, Floor, Light, LightingPreset, LightingSettings, Scene, Vec3, Wall,
};
use genos_window::Window;

fn count_f64(n: usize) -> f64 {
    f64::from(u32::try_from(n).expect("count"))
}

type ParentKey = (i32, i32, i32, u32, u32);

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const STILL: u32 = 24;
const SHIMMER_FRAMES: u32 = 8;
/// Mean absolute luma between a seam voxel and the coarser level at the
/// same point.
const SEAM_MEAN: f64 = 0.05;
/// Worst pair at a seam.
const SEAM_MAX: f64 = 0.15;
/// Still-frame pixel step on the far-wall patch, after settle.
const SHIMMER_PIXEL: f64 = 0.08;
const SHIMMER_MEAN: f64 = 0.02;

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

fn hall_camera() -> Camera {
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(0.0, 1.6, 12.0), 0.0, 0.05);
    camera
}

fn luma_plane(rgb: &[f64]) -> Vec<f64> {
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
) -> (Vec<f64>, Vec<Level0Voxel>) {
    let _ = window.pump();
    renderer.draw(world, camera, true).expect("draw").expect("readback");
    let luma = luma_plane(&renderer.read_scene().expect("scene radiance").rgb);
    let voxels = renderer.lighting_level0_voxels().expect("voxels");
    (luma, voxels)
}

fn seam_pairs(voxels: &[Level0Voxel]) -> Vec<(f64, u32, u32)> {
    let mut by_key = std::collections::BTreeMap::new();
    for voxel in voxels {
        if voxel.weight <= 0.5 {
            continue;
        }
        by_key.insert(voxel.key(), voxel);
    }
    let mut pairs = Vec::new();
    for voxel in voxels {
        if voxel.weight <= 0.5 {
            continue;
        }
        let Some(parent_key) = voxel.parent_key() else {
            continue;
        };
        let Some(parent) = by_key.get(&parent_key) else {
            continue;
        };
        let delta = (voxel.luma() - parent.luma()).abs();
        pairs.push((f64::from(delta), voxel.level, parent.level));
    }
    pairs
}

#[test]
fn adjacent_levels_agree_at_a_band_seam() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let world = World::from_scene(scene());
    renderer.set_lighting_settings(settings());
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Full,
        lamp: 0,
    });
    let cam = hall_camera();
    let mut voxels = Vec::new();
    for _ in 0..STILL {
        let (_, now) = grab(window, renderer, &world, &cam);
        voxels = now;
    }
    let pairs = seam_pairs(&voxels);
    let mut by_key = std::collections::BTreeMap::new();
    for voxel in &voxels {
        by_key.insert(voxel.key(), voxel);
    }
    let mut sibling: std::collections::BTreeMap<ParentKey, (f64, u32, f64)> =
        std::collections::BTreeMap::new();
    for voxel in &voxels {
        let Some(parent_key) = voxel.parent_key() else {
            continue;
        };
        let Some(parent) = by_key.get(&parent_key) else {
            continue;
        };
        if voxel.weight <= 0.5 || parent.weight <= 0.5 {
            continue;
        }
        let e = sibling.entry(parent_key).or_insert((0.0, 0, f64::from(parent.luma())));
        e.0 += f64::from(voxel.luma());
        e.1 += 1;
    }
    let mut sib_mean = 0.0_f64;
    let mut sib_worst = 0.0_f64;
    let mut sib_n = 0u32;
    for (sum, n, parent) in sibling.values() {
        let avg = sum / f64::from(*n);
        let d = (avg - parent).abs();
        sib_mean += d;
        sib_worst = sib_worst.max(d);
        sib_n += 1;
        eprintln!("seam siblings {n} avg {avg:.4} parent {parent:.4} d {d:.4}");
    }
    if sib_n > 0 {
        sib_mean /= f64::from(sib_n);
    }
    eprintln!("seam sibling-avg vs parent n {sib_n} mean {sib_mean:.4} worst {sib_worst:.4}");
    assert!(
        pairs.len() >= 8,
        "need seam pairs where both footprint levels are inserted, got {}",
        pairs.len()
    );
    let mean = pairs.iter().map(|p| p.0).sum::<f64>() / count_f64(pairs.len());
    let worst = pairs.iter().map(|p| p.0).fold(0.0, f64::max);
    let mut by_level: std::collections::BTreeMap<u32, (f64, u32)> = std::collections::BTreeMap::new();
    for (delta, level, _) in &pairs {
        let e = by_level.entry(*level).or_insert((0.0, 0));
        e.0 += *delta;
        e.1 += 1;
    }
    eprint!("seam pairs {} mean {mean:.4} worst {worst:.4}", pairs.len());
    for (level, (sum, n)) in &by_level {
        eprint!(" L{level}->{} {:.4}", level + 1, sum / f64::from(*n));
    }
    eprintln!();
    assert!(
        mean <= SEAM_MEAN,
        "band-seam mean luma {mean:.4} exceeds {SEAM_MEAN} over {} pairs (worst {worst:.4})",
        pairs.len()
    );
    assert!(
        worst <= SEAM_MAX,
        "band-seam worst luma {worst:.4} exceeds {SEAM_MAX} (mean {mean:.4}, {} pairs)",
        pairs.len()
    );
}

#[test]
fn a_still_far_wall_does_not_shimmer() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let world = World::from_scene(scene());
    renderer.set_lighting_settings(settings());
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Full,
        lamp: 0,
    });
    let cam = hall_camera();
    let mut prev = None;
    let mut worst_mean = 0.0f64;
    let mut worst_pixel = 0.0f64;
    for step in 0..STILL + SHIMMER_FRAMES {
        let (luma, _) = grab(window, renderer, &world, &cam);
        if step + 1 < STILL {
            continue;
        }
        let now = far_patch(&luma);
        let Some(before) = prev.replace(now.clone()) else {
            continue;
        };
        let mean_step = (now.0 - before.0).abs();
        let mut pixel_step = 0.0f64;
        for (a, b) in before.1.iter().zip(now.1.iter()) {
            pixel_step = pixel_step.max((a - b).abs());
        }
        eprintln!(
            "still far step {}: mean {} step {mean_step:.4} pixel {pixel_step:.4}",
            step + 1 - STILL,
            now.0
        );
        worst_mean = worst_mean.max(mean_step);
        worst_pixel = worst_pixel.max(pixel_step);
    }
    eprintln!("still far-wall shimmer mean {worst_mean:.4} pixel {worst_pixel:.4}");
    assert!(
        worst_mean <= SHIMMER_MEAN,
        "still far-wall mean shimmered {worst_mean} (limit {SHIMMER_MEAN})"
    );
    assert!(
        worst_pixel <= SHIMMER_PIXEL,
        "still far-wall pixel shimmered {worst_pixel} (limit {SHIMMER_PIXEL})"
    );
}
