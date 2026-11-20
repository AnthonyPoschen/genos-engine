#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]
#![expect(clippy::cast_precision_loss, reason = "probe means")]
#![expect(clippy::as_conversions, reason = "probe means")]
//! Level-0 voxels on the side faces of one box, against the CPU path reference.
//! Run with --ignored --nocapture. `BOX_YAW` turns the box (radians, default 0.35).

use genos_debug::reference::bounce_irradiance;
use genos_render::{CacheQuery, DebugView, Level0Voxel, Renderer, Surfaces, ViewMode, World};
use genos_scene::{
    AmbientMode, Camera, Ceiling, Floor, Light, LightingPreset, LightingSettings, Scene, Shape,
    Solid, Vec3, Wall,
};
use genos_window::Window;

const WIDTH: u32 = 640;
const HEIGHT: u32 = 360;
const BOX_AT: [f32; 3] = [0.0, 0.0, 6.0];
const BOX_SIZE: f32 = 1.0;
const BOX_HEIGHT: f32 = 1.0;

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
    if std::env::var("PROBE_HIGH").is_ok() {
        let mut settings = LightingSettings::preset(LightingPreset::High);
        settings.sky = false;
        settings.gi.enabled = true;
        settings.world_cache.enabled = true;
        return settings;
    }
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

fn wall(x: f32, z: f32, half_x: f32, half_z: f32) -> Wall {
    Wall {
        position: Vec3::new(x, 0.0, z),
        half_x,
        half_z,
        height: 4.0,
        base: 0.0,
        color: [0.7, 0.7, 0.7],
        absorption: 0.0,
        reflectance: -1.0,
        color_mix: -1.0,
    }
}

fn scene(yaw: f32) -> Scene {
    Scene {
        floor: Floor { position: Vec3::ZERO, half_x: 8.0, half_z: 16.0, color: [0.72, 0.72, 0.72] },
        walls: vec![
            wall(0.0, -16.0, 8.0, 0.1),
            wall(0.0, 16.0, 8.0, 0.1),
            wall(-8.0, 0.0, 0.1, 16.0),
            wall(8.0, 0.0, 0.1, 16.0),
        ],
        solids: vec![Solid {
            shape: Shape::Square,
            position: Vec3::new(BOX_AT[0], BOX_AT[1], BOX_AT[2]),
            size: BOX_SIZE,
            height: BOX_HEIGHT,
            yaw,
            color: [0.8, 0.2, 0.15],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }],
        lights: vec![Light { position: Vec3::new(1.5, 3.0, 4.0), color: [5.0, 4.6, 4.0], direction: Vec3::ZERO }],
        ceiling: Some(Ceiling { height: 4.0, color: [0.45, 0.45, 0.45] }),
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

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn add(a: [f32; 3], b: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] + b[0] * k, a[1] + b[1] * k, a[2] + b[2] * k]
}

fn axis_of(n: [f32; 3]) -> usize {
    let a = n.map(f32::abs);
    if a[0] >= a[1] && a[0] >= a[2] {
        0
    } else if a[1] >= a[2] {
        1
    } else {
        2
    }
}

fn axis_vec(n: [f32; 3]) -> [f32; 3] {
    let ax = axis_of(n);
    let mut v = [0.0; 3];
    v[ax] = n[ax].signum();
    v
}

/// The side face of the box a point lies on, as its outward normal.
fn face_normal(p: [f32; 3], solid: &Solid) -> Option<[f32; 3]> {
    let (s, c) = solid.yaw.sin_cos();
    let ex = [c, 0.0, -s];
    let ez = [s, 0.0, c];
    let d = [p[0] - solid.position.x, p[1], p[2] - solid.position.z];
    let lx = d[0] * ex[0] + d[2] * ex[2];
    let lz = d[0] * ez[0] + d[2] * ez[2];
    let half = BOX_SIZE * 0.5;
    if !(0.0..=BOX_HEIGHT).contains(&p[1]) || lx.abs() > half + 0.08 || lz.abs() > half + 0.08 {
        return None;
    }
    let gap_x = (lx.abs() - half).abs();
    let gap_z = (lz.abs() - half).abs();
    if gap_x.min(gap_z) >= 0.06 {
        return None;
    }
    if gap_x < gap_z {
        Some(ex.map(|v| v * lx.signum()))
    } else {
        Some(ez.map(|v| v * lz.signum()))
    }
}

fn hemi(n: [f32; 3], u: f32, v: f32) -> [f32; 3] {
    let up = if n[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
    let t = {
        let c = [up[1] * n[2] - up[2] * n[1], up[2] * n[0] - up[0] * n[2], up[0] * n[1] - up[1] * n[0]];
        let l = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
        c.map(|x| x / l)
    };
    let b = [n[1] * t[2] - n[2] * t[1], n[2] * t[0] - n[0] * t[2], n[0] * t[1] - n[1] * t[0]];
    let r = (1.0 - u * u).max(0.0).sqrt();
    let phi = std::f32::consts::TAU * v;
    [0, 1, 2].map(|i| t[i] * r * phi.cos() + b[i] * r * phi.sin() + n[i] * u)
}

fn hit_share(sc: &Scene, from: [f32; 3], n: [f32; 3], reach: f32) -> f32 {
    let side = 16u32;
    let mut hits = 0u32;
    for k in 0..side * side {
        let d = hemi(n, ((k % side) as f32 + 0.5) / side as f32, ((k / side) as f32 + 0.5) / side as f32);
        if sc.first_hit(from, d, reach).is_some() {
            hits += 1;
        }
    }
    hits as f32 / (side * side) as f32
}

struct Corner {
    found: bool,
    filled: bool,
    inside: bool,
    blocked: bool,
    plane: f32,
    luma: f32,
    reference: f32,
    position: [f32; 3],
    stored_n: [f32; 3],
}

fn corners(renderer: &mut Renderer, sc: &Scene, v: &Level0Voxel, n: [f32; 3], s: f32) -> Vec<Corner> {
    let size = 2.0 * s;
    let ax = axis_of(n);
    let (a1, a2) = ((ax + 1) % 3, (ax + 2) % 3);
    let uv = v.pos.map(|x| x / size - 0.5);
    let mut queries = Vec::new();
    for i in 0..4 {
        let mut q = [0.0; 3];
        let o1 = (i & 1) as f32;
        let o2 = ((i >> 1) & 1) as f32;
        q[a1] = (uv[a1].floor() + o1 + 0.5) * size;
        q[a2] = (uv[a2].floor() + o2 + 0.5) * size;
        let slope = ((q[a1] - v.pos[a1]) * n[a1] + (q[a2] - v.pos[a2]) * n[a2]) / n[ax];
        q[ax] = v.pos[ax] - slope.clamp(-size, size);
        queries.push(CacheQuery { position: q, normal: axis_vec(n), size });
    }
    let cells = renderer.read_cache_cells(&queries).expect("cells");
    let from = add(v.pos, n, 0.02);
    cells
        .iter()
        .map(|c| {
            let at = add(c.position, n, 0.02);
            Corner {
                found: c.hit.slot.is_some(),
                filled: c.weight > 0.0,
                inside: sc.inside(c.position) || sc.inside(at),
                blocked: c.hit.slot.is_some() && sc.blocked(from, at),
                plane: (0..3).map(|i| (c.position[i] - v.pos[i]) * n[i]).sum::<f32>(),
                luma: luma(c.radiance),
                position: c.position,
                stored_n: c.normal,
                reference: if c.hit.slot.is_some() { luma(bounce_irradiance(sc, c.position, n, 256, 16, 7)) } else { 0.0 },
            }
        })
        .collect()
}

fn run(yaw: f32) {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let mut sc = scene(yaw);
    let mut world = World::from_scene(sc.clone());
    renderer.set_lighting_settings(sc.lighting.clone());
    renderer.set_debug_view(DebugView { mode: ViewMode::Full, lamp: 0 });
    let mut cam = Camera::opening();
    let cam_yaw: f32 = std::env::var("PROBE_CAM_YAW").ok().and_then(|v| v.parse().ok()).unwrap_or(0.35);
    let cam_x: f32 = std::env::var("PROBE_CAM_X").ok().and_then(|v| v.parse().ok()).unwrap_or(1.2);
    let cam_z: f32 = std::env::var("PROBE_CAM_Z").ok().and_then(|v| v.parse().ok()).unwrap_or(8.6);
    cam.set_pose(Vec3::new(cam_x, 1.4, cam_z), cam_yaw, -0.2);
    let moving: u32 = std::env::var("PROBE_MOVE").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let spin: u32 = std::env::var("PROBE_SPIN").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    for frame in 0..64u32 {
        if frame + moving >= 64 {
            let k = (frame + moving - 64) as f32;
            cam.set_pose(Vec3::new(cam_x - 0.03 * k, 1.4, cam_z - 0.03 * k), cam_yaw - 0.02 * k, -0.2);
        }
        let hold: u32 = std::env::var("PROBE_HOLD").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        if frame + spin >= 64 && frame + hold < 64 {
            let k = (frame + spin - 64) as f32 + 1.0;
            let solid = &mut world.scene.solids[0];
            solid.yaw = yaw + 0.01 * k;
            solid.position.x = BOX_AT[0] + 0.02 * k;
            sc.solids[0] = solid.clone();
        }
        let _ = window.pump();
        renderer.draw(&world, &cam, true).expect("draw").expect("readback");
    }
    let voxels = renderer.lighting_level0_voxels().expect("voxels");
    if let Ok(path) = std::env::var("PROBE_SHOT") {
        renderer.set_debug_view(DebugView { mode: ViewMode::Bounce, lamp: 0 });
        let _ = window.pump();
        let pixels = renderer.draw(&world, &cam, true).expect("draw").expect("readback");
        let (w, h) = (renderer.width(), renderer.height());
        let mut ppm = format!("P6 {w} {h} 255\n").into_bytes();
        for px in pixels.chunks_exact(4) {
            ppm.extend_from_slice(&px[..3]);
        }
        std::fs::write(path, ppm).expect("shot");
    }
    if std::env::var("PROBE_LAG").is_ok() {
        for lag in 0..4 {
            let mut solid = sc.solids[0].clone();
            solid.yaw -= 0.01 * lag as f32;
            solid.position.x -= 0.02 * lag as f32;
            let (sn, cs) = solid.yaw.sin_cos();
            let (ex, ez) = ([cs, 0.0, -sn], [sn, 0.0, cs]);
            let mut sum = 0.0f32;
            let mut count = 0u32;
            for v in voxels.iter().filter(|v| v.weight > 0.5 && v.axis != 2 && v.axis != 3) {
                let d = [v.pos[0] - solid.position.x, v.pos[2] - solid.position.z];
                let lx = d[0] * ex[0] + d[1] * ex[2];
                let lz = d[0] * ez[0] + d[1] * ez[2];
                let gap = (lx.abs() - 0.5).abs().min((lz.abs() - 0.5).abs());
                if lx.abs() < 0.6 && lz.abs() < 0.6 && v.pos[1] < BOX_HEIGHT {
                    sum += gap;
                    count += 1;
                }
            }
            eprintln!("lag {lag}: {count} side voxels, mean distance to face {:.4} m", sum / count.max(1) as f32);
        }
    }
    let mut rows = Vec::new();
    for v in voxels.iter().filter(|v| v.weight > 0.5) {
        let Some(n) = face_normal(v.pos, &sc.solids[0]) else { continue };
        let ax = axis_of(n);
        if v.axis != (ax as u32) * 2 + u32::from(n[ax] < 0.0) {
            continue;
        }
        let s = 0.0625 * 2f32.powi(v.level as i32);
        let reference = luma(bounce_irradiance(&sc, v.pos, n, 512, 16, 7));
        let from = add(v.pos, n, 0.02);
        let cs = corners(renderer, &sc, v, n, s);
        let usable = cs.iter().filter(|c| c.found && c.filled && !c.inside && !c.blocked && c.plane.abs() <= s).count();
        rows.push((
            luma(v.radiance) / reference.max(1e-4),
            *v,
            n,
            s,
            reference,
            sc.inside(from),
            hit_share(&sc, from, n, 8.0 * s),
            usable,
            cs,
        ));
    }
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    let count = rows.len();
    eprintln!("box yaw {yaw:.2}: {count} face voxels");
    if count == 0 {
        return;
    }
    let median = rows[count / 2].0;
    let dark = rows.iter().filter(|r| r.0 < 0.5).count();
    let inside = rows.iter().filter(|r| r.5).count();
    let no_parent = rows.iter().filter(|r| r.7 == 0).count();
    let dark_inside = rows.iter().filter(|r| r.0 < 0.5 && r.5).count();
    let dark_no_parent = rows.iter().filter(|r| r.0 < 0.5 && r.7 == 0).count();
    eprintln!(
        "box yaw {yaw:.2}: median engine/ref {median:.2}, dark(<0.5) {dark}, origin inside {inside} (dark {dark_inside}), no usable parent {no_parent} (dark {dark_no_parent})"
    );
    if let Some(r) = rows.first() {
        for c in &r.8 {
            if c.luma < 0.5 * c.reference {
                eprintln!("  dark cell pos {:?} stored n {:?} plane {:.3} engine {:.2} ref {:.2}", c.position, c.stored_n, c.plane, c.luma, c.reference);
            }
        }
    }
    for r in rows.iter().take(12).chain(rows.iter().skip(count.saturating_sub(3))) {
        let flags: String = r
            .8
            .iter()
            .map(|c| {
                if !c.found {
                    '-'
                } else if !c.filled {
                    'e'
                } else if c.inside {
                    'i'
                } else if c.blocked {
                    'b'
                } else if c.plane.abs() > r.3 {
                    'p'
                } else {
                    'o'
                }
            })
            .collect();
        eprintln!(
            "  ratio {:.2} engine {:.3} ref {:.3} s {:.3} pos [{:.2} {:.2} {:.2}] n [{:.2} {:.2}] inside {} hit<=8s {:.2} corners {flags} cell(engine,ref) {:?}",
            r.0, luma(r.1.radiance), r.4, r.3, r.1.pos[0], r.1.pos[1], r.1.pos[2], r.2[0], r.2[2], r.5, r.6,
            r.8.iter().map(|c| ((c.luma * 100.0).round() / 100.0, (c.reference * 100.0).round() / 100.0)).collect::<Vec<_>>()
        );
    }
}

#[test]
#[ignore = "probe: run by hand"]
fn box_face_voxels_against_the_reference() {
    let yaw: f32 = std::env::var("BOX_YAW").ok().and_then(|v| v.parse().ok()).unwrap_or(0.35);
    run(yaw);
}
