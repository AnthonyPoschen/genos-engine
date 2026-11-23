#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]
#![expect(clippy::cast_precision_loss, reason = "probe means")]
#![expect(clippy::as_conversions, reason = "probe means")]
#![expect(
    clippy::disallowed_methods,
    reason = "a hand-run probe (#[ignore]): cargo test passes no arguments to one test, so its knobs are variables"
)]
#![expect(
    clippy::many_single_char_names,
    clippy::similar_names,
    reason = "the probe keeps the letters of the estimator it checks"
)]
//! Energy-closure probe: level-0 stored irradiance against the CPU path
//! reference's bounce irradiance at the same point and normal, per level,
//! for one-bounce and many-bounce references. Run with --ignored --nocapture.

use genos_debug::reference::bounce_irradiance;
use genos_render::{direct_light_in, CacheQuery, DebugView, Renderer, Surfaces, ViewMode, World};
use genos_scene::{
    AmbientMode, Camera, Ceiling, Floor, Light, LightingPreset, LightingSettings, Scene, Vec3, Wall,
};
use genos_window::Window;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;

fn open() -> (std::sync::MutexGuard<'static, ()>, genos_window::Lifecycle<Renderer>) {
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


fn axis_vec(a: u32) -> [f32; 3] {
    let mut v = [0.0; 3];
    v[(a >> 1) as usize] = if a & 1 == 0 { 1.0 } else { -1.0 };
    v
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        ((self.0 >> 40) as f32) / 16_777_216.0
    }
}

fn cosine(n: [f32; 3], u: f32, v: f32) -> [f32; 3] {
    let up = if n[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
    let t = norm(cross(up, n));
    let b = cross(n, t);
    let r = u.sqrt();
    let phi = std::f32::consts::TAU * v;
    let z = (1.0 - u).max(0.0).sqrt();
    norm([
        t[0] * r * phi.cos() + b[0] * r * phi.sin() + n[0] * z,
        t[1] * r * phi.cos() + b[1] * r * phi.sin() + n[1] * z,
        t[2] * r * phi.cos() + b[2] * r * phi.sin() + n[2] * z,
    ])
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt().max(1e-9);
    [a[0] / l, a[1] / l, a[2] / l]
}

/// One-bounce irradiance split at distance `split`: (near, far).
fn split_one_bounce(scene: &Scene, p: [f32; 3], n: [f32; 3], split: f32, samples: u32) -> (f32, f32) {
    let from = [p[0] + n[0] * 0.02, p[1] + n[1] * 0.02, p[2] + n[2] * 0.02];
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let (mut near, mut far) = (0.0f32, 0.0f32);
    for _ in 0..samples {
        let d = cosine(n, rng.next(), rng.next());
        let Some(hit) = scene.first_hit(from, d, 200.0) else { continue };
        let dist = ((hit.point[0] - from[0]).powi(2) + (hit.point[1] - from[1]).powi(2) + (hit.point[2] - from[2]).powi(2)).sqrt();
        let lift = [hit.point[0] + hit.normal[0] * 0.02, hit.point[1] + hit.normal[1] * 0.02, hit.point[2] + hit.normal[2] * 0.02];
        let e = direct_light_in(scene, scene, lift, hit.normal);
        let l = luma([e[0] * hit.albedo[0], e[1] * hit.albedo[1], e[2] * hit.albedo[2]]) / std::f32::consts::PI;
        if dist <= split { near += l; } else { far += l; }
    }
    let k = std::f32::consts::PI / samples as f32;
    (near * k, far * k)
}

/// One-bounce irradiance from hits at t >= t0: (clear path from 0, pass-through from t0).
fn far_one_bounce(scene: &Scene, p: [f32; 3], n: [f32; 3], t0: f32, samples: u32) -> (f32, f32) {
    let from = [p[0] + n[0] * 0.02, p[1] + n[1] * 0.02, p[2] + n[2] * 0.02];
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    let (mut clear, mut pass) = (0.0f32, 0.0f32);
    let shade = |hit: genos_render::TraceHit| {
        let lift = [hit.point[0] + hit.normal[0] * 0.02, hit.point[1] + hit.normal[1] * 0.02, hit.point[2] + hit.normal[2] * 0.02];
        let e = direct_light_in(scene, scene, lift, hit.normal);
        0.8 * luma([e[0] * hit.albedo[0], e[1] * hit.albedo[1], e[2] * hit.albedo[2]]) / std::f32::consts::PI
    };
    for _ in 0..samples {
        let d = cosine(n, rng.next(), rng.next());
        if let Some(hit) = scene.first_hit(from, d, 200.0) {
            let dist = ((hit.point[0] - from[0]).powi(2) + (hit.point[1] - from[1]).powi(2) + (hit.point[2] - from[2]).powi(2)).sqrt();
            if dist >= t0 { clear += shade(hit); }
        }
        let start = [from[0] + d[0] * t0, from[1] + d[1] * t0, from[2] + d[2] * t0];
        if let Some(hit) = scene.first_hit(start, d, 200.0 - t0) { pass += shade(hit); }
    }
    let k = std::f32::consts::PI / samples as f32;
    (clear * k, pass * k)
}

fn uniform(n: [f32; 3], u: f32, v: f32) -> [f32; 3] {
    let up = if n[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
    let t = norm(cross(up, n));
    let b = cross(n, t);
    let r = (1.0 - u * u).max(0.0).sqrt();
    let phi = std::f32::consts::TAU * v;
    norm([
        t[0] * r * phi.cos() + b[0] * r * phi.sin() + n[0] * u,
        t[1] * r * phi.cos() + b[1] * r * phi.sin() + n[1] * u,
        t[2] * r * phi.cos() + b[2] * r * phi.sin() + n[2] * u,
    ])
}

fn shade_hit(scene: &Scene, hit: genos_render::TraceHit) -> f32 {
    let lift = [hit.point[0] + hit.normal[0] * 0.02, hit.point[1] + hit.normal[1] * 0.02, hit.point[2] + hit.normal[2] * 0.02];
    let e = direct_light_in(scene, scene, lift, hit.normal);
    0.8 * luma([e[0] * hit.albedo[0], e[1] * hit.albedo[1], e[2] * hit.albedo[2]]) / std::f32::consts::PI
}

/// Luma SH-L1 (c0, c) of an upper cell at p of size `size`, the shader's chain.
fn cpu_cell_sh(scene: &Scene, p: [f32; 3], n: [f32; 3], size: f32, rays: u32) -> Option<(f32, [f32; 3])> {
    if size > 32.0 + 1e-6 {
        return None;
    }
    let parent = cpu_cell_sh(scene, p, n, size * 2.0, rays);
    let from = [p[0] + n[0] * 0.02, p[1] + n[1] * 0.02, p[2] + n[2] * 0.02];
    let (t0, t1) = (4.0 * size, (8.0 * size).min(200.0));
    let side = rays.isqrt();
    let mut rng = Rng(0xABCD_EF01 ^ u64::from(size.to_bits()));
    let (mut c0, mut c) = (0.0f32, [0.0f32; 3]);
    for k in 0..rays {
        let u = ((k % side) as f32 + rng.next()) / side as f32;
        let v = ((k / side) as f32 + rng.next()) / side as f32;
        let d = uniform(n, u, v);
        let start = [from[0] + d[0] * t0, from[1] + d[1] * t0, from[2] + d[2] * t0];
        let l = match scene.first_hit(start, d, t1 - t0) {
            Some(hit) => shade_hit(scene, hit),
            None => parent.map_or(0.0, |(a, b)| sh_eval(n, a, b, d)),
        };
        c0 += l;
        for i in 0..3 { c[i] += l * d[i]; }
    }
    let w = std::f32::consts::TAU / rays as f32;
    Some((c0 * w * 0.282_094_8, c.map(|x| x * w * 0.488_602_5)))
}

fn sh_eval(n: [f32; 3], c0: f32, c: [f32; 3], d: [f32; 3]) -> f32 {
    let pi = std::f32::consts::PI;
    let i0 = c0 / 0.282_094_8;
    let idot = (c[0] * d[0] + c[1] * d[1] + c[2] * d[2]) / 0.488_602_5;
    let i1 = (c[0] * n[0] + c[1] * n[1] + c[2] * n[2]) / 0.488_602_5;
    let a = (2.0 * i0 - 3.0 * i1) / pi;
    let mu = d[0] * n[0] + d[1] * n[1] + d[2] * n[2];
    (a * (1.0 - 1.5 * mu) + (1.5 / pi) * idot).max(0.0)
}

/// Level-0 irradiance through the CPU chain: own [eps, 8s] hits, misses read the parent SH.
fn cpu_chain_e(scene: &Scene, p: [f32; 3], n: [f32; 3], s: f32, rays: u32) -> f32 {
    let parent = cpu_cell_sh(scene, p, n, 2.0 * s, rays);
    let from = [p[0] + n[0] * 0.02, p[1] + n[1] * 0.02, p[2] + n[2] * 0.02];
    let side = rays.isqrt();
    let mut rng = Rng(0x5555_1234);
    let mut sum = 0.0f32;
    for k in 0..rays {
        let u = ((k % side) as f32 + rng.next()) / side as f32;
        let v = ((k / side) as f32 + rng.next()) / side as f32;
        let d = uniform(n, u, v);
        let l = match scene.first_hit(from, d, 8.0 * s) {
            Some(hit) => shade_hit(scene, hit),
            None => parent.map_or(0.0, |(a, b)| sh_eval(n, a, b, d)),
        };
        sum += l * u;
    }
    sum * std::f32::consts::TAU / rays as f32
}

fn frame(n: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let up = if n[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
    let t = norm(cross(up, n));
    (t, cross(n, t))
}

fn bin_dir(n: [f32; 3], b: u32, j1: f32, j2: f32) -> [f32; 3] {
    let (t, bt) = frame(n);
    let u = ((b / 4) as f32 + j1) / 4.0;
    let phi = std::f32::consts::TAU * ((b % 4) as f32 + j2) / 4.0;
    let r = (1.0 - u * u).max(0.0).sqrt();
    norm([
        t[0] * r * phi.cos() + bt[0] * r * phi.sin() + n[0] * u,
        t[1] * r * phi.cos() + bt[1] * r * phi.sin() + n[1] * u,
        t[2] * r * phi.cos() + bt[2] * r * phi.sin() + n[2] * u,
    ])
}

fn bins_eval(n: [f32; 3], bins: &[f32; 16], d: [f32; 3]) -> f32 {
    let (t, bt) = frame(n);
    let u = (d[0] * n[0] + d[1] * n[1] + d[2] * n[2]).clamp(0.0, 1.0);
    let mut phi = (d[0] * bt[0] + d[1] * bt[1] + d[2] * bt[2]).atan2(d[0] * t[0] + d[1] * t[1] + d[2] * t[2]);
    if phi < 0.0 { phi += std::f32::consts::TAU; }
    let fu = (u * 4.0 - 0.5).clamp(0.0, 3.0);
    let fp = phi / std::f32::consts::TAU * 4.0 - 0.5;
    let r0 = fu.floor().min(2.0);
    let wu = fu - r0;
    let c0 = fp.floor();
    let wp = fp - c0;
    // The whole part of a value in 0..4, without a float to integer cast.
    let whole = |v: f32| usize::from(v >= 1.0) + usize::from(v >= 2.0) + usize::from(v >= 3.0);
    let ci = |c: f32| whole(c.rem_euclid(4.0));
    let r = whole(r0);
    let at = |row: usize, col: usize| bins[row * 4 + col];
    let a = at(r, ci(c0)) * (1.0 - wp) + at(r, ci(c0 + 1.0)) * wp;
    let b = at(r + 1, ci(c0)) * (1.0 - wp) + at(r + 1, ci(c0 + 1.0)) * wp;
    a * (1.0 - wu) + b * wu
}

fn cpu_cell_bins(scene: &Scene, p: [f32; 3], n: [f32; 3], size: f32, per_bin: u32) -> Option<[f32; 16]> {
    if size > 32.0 + 1e-6 {
        return None;
    }
    let parent = cpu_cell_bins(scene, p, n, size * 2.0, per_bin);
    let from = [p[0] + n[0] * 0.02, p[1] + n[1] * 0.02, p[2] + n[2] * 0.02];
    let (t0, t1) = (4.0 * size, (8.0 * size).min(200.0));
    let mut rng = Rng(0xABCD_EF01 ^ u64::from(size.to_bits()));
    let mut bins = [0.0f32; 16];
    let mut counts = [0u32; 16];
    let clear_only = std::env::var("PROBE_CLEAR").is_ok();
    for b in 0..16u32 {
        let mut sum = 0.0;
        for _ in 0..per_bin {
            let d = bin_dir(n, b, rng.next(), rng.next());
            let l = if clear_only {
                match scene.first_hit(from, d, t1) {
                    Some(hit) => {
                        let dist = ((hit.point[0] - from[0]).powi(2) + (hit.point[1] - from[1]).powi(2) + (hit.point[2] - from[2]).powi(2)).sqrt();
                        if dist < t0 { continue; }
                        shade_hit(scene, hit)
                    }
                    None => parent.as_ref().map_or(0.0, |pb| bins_eval(n, pb, d)),
                }
            } else {
                let start = [from[0] + d[0] * t0, from[1] + d[1] * t0, from[2] + d[2] * t0];
                match scene.first_hit(start, d, t1 - t0) {
                    Some(hit) => shade_hit(scene, hit),
                    None => parent.as_ref().map_or(0.0, |pb| bins_eval(n, pb, d)),
                }
            };
            sum += l;
            counts[b as usize] += 1;
        }
        bins[b as usize] = if counts[b as usize] > 0 { sum / counts[b as usize] as f32 } else { -1.0 };
    }
    let (mut tot, mut cn) = (0.0f32, 0u32);
    for b in 0..16 { if counts[b] > 0 { tot += bins[b]; cn += 1; } }
    let mean = if cn > 0 { tot / cn as f32 } else { 0.0 };
    for b in 0..16 { if counts[b] == 0 { bins[b] = mean; } }
    Some(bins)
}

fn cpu_chain_bins_e(scene: &Scene, p: [f32; 3], n: [f32; 3], s: f32, rays: u32) -> f32 {
    let parent = cpu_cell_bins(scene, p, n, 2.0 * s, 16);
    let from = [p[0] + n[0] * 0.02, p[1] + n[1] * 0.02, p[2] + n[2] * 0.02];
    let side = rays.isqrt();
    let mut rng = Rng(0x5555_1234);
    let mut sum = 0.0f32;
    for k in 0..rays {
        let u = ((k % side) as f32 + rng.next()) / side as f32;
        let v = ((k / side) as f32 + rng.next()) / side as f32;
        let d = uniform(n, u, v);
        let l = match scene.first_hit(from, d, 8.0 * s) {
            Some(hit) => shade_hit(scene, hit),
            None => parent.as_ref().map_or(0.0, |pb| bins_eval(n, pb, d)),
        };
        sum += l * u;
    }
    sum * std::f32::consts::TAU / rays as f32
}

#[test]
#[ignore = "diagnostic probe"]
fn level0_energy_closure() {
    let frames: u32 = std::env::var("PROBE_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(48);
    let bounce: u32 = std::env::var("PROBE_BOUNCE").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let mut s = settings();
    s.bounce.count = bounce;
    let mut sc = scene();
    sc.lighting = s.clone();
    let world = World::from_scene(sc.clone());
    renderer.set_lighting_settings(s);
    renderer.set_debug_view(DebugView { mode: ViewMode::Full, lamp: 0 });
    let cam = hall_camera();
    for _ in 0..frames {
        let _ = window.pump();
        renderer.draw(&world, &cam, true).expect("draw").expect("readback");
    }
    let voxels = renderer.lighting_level0_voxels().expect("voxels");
    let mut by_level: std::collections::BTreeMap<(u32, u32), Vec<_>> = std::collections::BTreeMap::new();
    for v in &voxels {
        if v.weight > 0.5 {
            by_level.entry((v.level, v.axis)).or_default().push(*v);
        }
    }
    eprintln!("probe bounce={bounce} frames={frames} voxels={}", voxels.len());
    let (mut se, mut s1, mut sn) = (0.0f64, 0.0f64, 0.0f64);
    for ((level, axis), list) in &by_level {
        let step = (list.len() / 6).max(1);
        let mut chain_sum = 0.0f32;
        let mut bins_sum = 0.0f32;
        let (mut e_sum, mut r1_sum, mut rn_sum, mut near_sum, mut far_sum, mut c) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32, 0u32);
        for v in list.iter().step_by(step).take(6) {
            let n = axis_vec(*axis);
            let size = 0.0625 * (1u32 << *level) as f32;
            let r1 = luma(bounce_irradiance(&sc, v.pos, n, 1024, 1, 7));
            let rn = luma(bounce_irradiance(&sc, v.pos, n, 1024, 16, 7));
            let (near, far) = split_one_bounce(&sc, v.pos, n, 8.0 * size, 2048);
            let chain = cpu_chain_e(&sc, v.pos, n, size, 1024);
            chain_sum += chain;
            bins_sum += cpu_chain_bins_e(&sc, v.pos, n, size, 1024);
            e_sum += v.luma();
            r1_sum += r1;
            rn_sum += rn;
            near_sum += near;
            far_sum += far;
            c += 1;
        }
        if let Some(v) = list.get(list.len() / 2) {
            let n = axis_vec(*axis);
            let mut size = 0.0625 * (2u32 << *level) as f32;
            let mut queries = Vec::new();
            while size <= 32.0 {
                queries.push(CacheQuery { position: v.pos, normal: n, size });
                size *= 2.0;
            }
            let cells = renderer.read_cache_cells(&queries).expect("cells");
            for (q, cell) in queries.iter().zip(cells.iter()) {
                let (clear, pass) = far_one_bounce(&sc, cell.position, n, 4.0 * q.size, 2048);
                eprintln!(
                    "probe   L{level} axis{axis} voxel {:?} E {:.4} | upper S={} slot {:?} pos {:?} w {:.1} E {:.4} ref[4S,inf) clear {:.4} pass {:.4}",
                    v.pos, v.luma(), q.size, cell.hit.slot, cell.position, cell.weight, luma(cell.radiance), clear, pass
                );
            }
        }
        let cf = c as f32;
        se += f64::from(e_sum);
        s1 += f64::from(r1_sum);
        sn += f64::from(rn_sum);
        eprintln!(
            "probe L{level} axis{axis} n={} sampled {c}: engine {:.4} ref1 {:.4} refN {:.4} | ref1 near[<=8s] {:.4} far {:.4} | engine/ref1 {:.3} engine/refN {:.3} cpu_sh_chain/ref1 {:.3} cpu_bins_chain/ref1 {:.3}",
            list.len(), e_sum / cf, r1_sum / cf, rn_sum / cf, near_sum / cf, far_sum / cf,
            e_sum / r1_sum.max(1e-6), e_sum / rn_sum.max(1e-6), chain_sum / r1_sum.max(1e-6), bins_sum / r1_sum.max(1e-6)
        );
    }
    eprintln!("probe total engine/ref1 {:.3} engine/refN {:.3}", se / s1.max(1e-9), se / sn.max(1e-9));
}
