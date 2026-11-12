#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! Per-frame cache trace of a full-shadow pixel and a penumbra pixel, and the
//! seed a turn writes into a fine texel that did not exist yet.
//!
//! The direct mean on these pixels is already stable. What remains is which
//! texel compose reads, whether its ray origin sits inside the wall, and how
//! far the first seed is from the texel once it has its own samples.

use std::fmt::Write as _;
use std::sync::Mutex;

use genos_render::{
    linear_from_display, luminance, radiance, sees, solid_contains, trace_ray, CacheQuery, Renderer,
    StoredCell,
};
use genos_scene::{look_direction, viewport_uv, Camera, LightingPreset, LightingSettings, Vec3};
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};
use genos_window::Window;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const TRACE_FRAMES: u32 = 36;
const TEXEL: f32 = 0.125;
const WORLD: f32 = 0.25;
const NEAR: f32 = 6.0;
const RAY_LIFT: f32 = 0.02;

fn open() -> (MutexGuard, genos_window::Lifecycle<Renderer>) {
    static GPU: Mutex<()> = Mutex::new(());
    let guard = GPU.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut window = Window::open_proof(WIDTH, HEIGHT).expect("proof window");
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height).expect("renderer");
    (guard, genos_window::Lifecycle::new(window, renderer, permit))
}

type MutexGuard = std::sync::MutexGuard<'static, ()>;

fn f32_u32(v: u32) -> f32 {
    let mut acc = 0.0f32;
    let mut bit = 1.0f32;
    let mut n = v;
    while n != 0 {
        if n & 1 == 1 {
            acc += bit;
        }
        bit += bit;
        n >>= 1;
    }
    acc
}

fn f32_i32(v: i32) -> f32 {
    let mag = f32_u32(v.unsigned_abs());
    if v < 0 {
        -mag
    } else {
        mag
    }
}

fn f64_f32(v: f32) -> f64 {
    f64::from(v)
}

fn hall() -> (Stage, Camera) {
    let mut stage = Stage::new(
        Scale::SMALL,
        1,
        LightMix {
            count: 5,
            dynamic_pct: 0,
            layout: Layout::Spread,
        },
        1.0,
        0.12,
        120.0,
    );
    stage.sun_frozen = true;
    stage.boxes_still = true;
    stage.lamps_still = true;
    stage.apply();
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(14.5, 1.7, 23.5), -0.3554, 0.08);
    (stage, camera)
}

fn eye_of(camera: &Camera) -> [f32; 3] {
    [camera.position.x, camera.position.y, camera.position.z]
}

fn length(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(v: [f32; 3], s: f32) -> [f32; 3] {
    [v[0] * s, v[1] * s, v[2] * s]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn add_scaled(p: [f32; 3], n: [f32; 3], scale_n: f32) -> [f32; 3] {
    add(p, scale(n, scale_n))
}

fn luma(rgb: [f32; 3]) -> f32 {
    luminance(rgb)
}

fn mean_var(samples: &[f64]) -> (f64, f64) {
    if samples.is_empty() {
        return (0.0, 0.0);
    }
    let n = f64_f32(f32_u32(u32::try_from(samples.len()).unwrap_or(0)));
    let sum: f64 = samples.iter().copied().sum();
    let mean = sum / n;
    let var = samples.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / n;
    (mean, var)
}

fn inside_scene(scene: &genos_scene::Scene, p: [f32; 3]) -> bool {
    for wall in &scene.walls {
        if p[1] > wall.base + 1.0e-4
            && p[1] < wall.base + wall.height - 1.0e-4
            && (p[0] - wall.position.x).abs() < wall.half_x - 1.0e-4
            && (p[2] - wall.position.z).abs() < wall.half_z - 1.0e-4
        {
            return true;
        }
    }
    scene.solids.iter().any(|solid| solid_contains(solid, p))
}

fn ray_dir(camera: &Camera, x: u32, y: u32) -> [f32; 3] {
    let forward = look_direction(camera.yaw, camera.pitch);
    let right = forward.cross(Vec3::Y).normalize();
    let up = right.cross(forward).normalize();
    let width = f32_u32(WIDTH);
    let height = f32_u32(HEIGHT);
    let ndc_x = (f32_u32(x) + 0.5) / width * 2.0 - 1.0;
    let ndc_y = 1.0 - (f32_u32(y) + 0.5) / height * 2.0;
    let tan_half = 1.0 / 3.0_f32.sqrt();
    let aspect = width / height;
    let dir = (forward + right * (ndc_x * tan_half * aspect) + up * (ndc_y * tan_half)).normalize();
    [dir.x, dir.y, dir.z]
}

struct Surface {
    pixel: u32,
    point: [f32; 3],
    normal: [f32; 3],
    albedo: [f32; 3],
    facing: u32,
    visible: u32,
    blocked: u32,
}

fn point_lamps(scene: &genos_scene::Scene, from: [f32; 3], normal: [f32; 3]) -> (u32, u32, u32) {
    let mut facing = 0u32;
    let mut visible = 0u32;
    let mut blocked = 0u32;
    let surface = add_scaled(from, normal, 0.001);
    for light in &scene.lights {
        let dir = light.direction;
        if dir.x * dir.x + dir.y * dir.y + dir.z * dir.z > 1.0e-8 {
            continue;
        }
        let pos = [light.position.x, light.position.y, light.position.z];
        if dot(sub(pos, surface), normal) <= 0.0 {
            continue;
        }
        facing += 1;
        if sees(scene, surface, pos) {
            visible += 1;
        } else {
            blocked += 1;
        }
    }
    (facing, visible, blocked)
}

fn surface_at(scene: &genos_scene::Scene, camera: &Camera, pixel: u32) -> Option<Surface> {
    let x = pixel % WIDTH;
    let y = pixel / WIDTH;
    let dir = ray_dir(camera, x, y);
    let hit = trace_ray(scene, eye_of(camera), dir, 40.0)?;
    let (facing, visible, blocked) = point_lamps(scene, hit.point, hit.normal);
    Some(Surface {
        pixel,
        point: hit.point,
        normal: hit.normal,
        albedo: hit.albedo,
        facing,
        visible,
        blocked,
    })
}

fn find_surface(
    scene: &genos_scene::Scene,
    camera: &Camera,
    want_shadow: bool,
) -> Option<Surface> {
    let mut y = 8u32;
    while y + 8 < HEIGHT {
        let mut x = 8u32;
        while x + 8 < WIDTH {
            if let Some(hit) = surface_at(scene, camera, y * WIDTH + x) {
                let shadow = hit.facing > 0 && hit.visible == 0;
                let penumbra = hit.visible > 0 && hit.blocked > 0;
                if want_shadow && shadow || !want_shadow && penumbra {
                    return Some(hit);
                }
            }
            x += 8;
        }
        y += 8;
    }
    None
}

struct Series {
    pixel_y: Vec<f64>,
    display_y: Vec<f64>,
    own_y: Vec<f64>,
    field_y: Vec<f64>,
    seed_y: Vec<f64>,
    coarse_y: Vec<f64>,
    weight: Vec<f64>,
    support: Vec<f64>,
    inside: u32,
    missed: u32,
}

fn cell_luma(cell: &StoredCell) -> f32 {
    if cell.hit.slot.is_none() {
        return 0.0;
    }
    luma(cell.radiance)
}

fn query(point: [f32; 3], normal: [f32; 3], size: f32) -> CacheQuery {
    CacheQuery {
        position: point,
        normal,
        size,
    }
}

fn display_luma(pixels: &[u8], pixel: u32) -> f64 {
    let at = usize::try_from(pixel).unwrap_or(0).saturating_mul(4);
    let b = f32_u32(u32::from(pixels.get(at).copied().unwrap_or(0))) / 255.0;
    let g = f32_u32(u32::from(pixels.get(at.saturating_add(1)).copied().unwrap_or(0))) / 255.0;
    let r = f32_u32(u32::from(pixels.get(at.saturating_add(2)).copied().unwrap_or(0))) / 255.0;
    f64_f32(luminance([
        linear_from_display(r),
        linear_from_display(g),
        linear_from_display(b),
    ]))
}

fn scene_luma(rgb: &[f64], pixel: u32) -> f64 {
    let at = usize::try_from(pixel).unwrap_or(0).saturating_mul(3);
    let r = rgb.get(at).copied().unwrap_or(0.0);
    let g = rgb.get(at.saturating_add(1)).copied().unwrap_or(0.0);
    let b = rgb.get(at.saturating_add(2)).copied().unwrap_or(0.0);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

fn note_frame(
    scene: &genos_scene::Scene,
    surface: &Surface,
    picture: &[f64],
    displayed: &[u8],
    fine: &StoredCell,
    coarse: &StoredCell,
    series: &mut Series,
) {
    series.pixel_y.push(scene_luma(picture, surface.pixel));
    series.display_y.push(display_luma(displayed, surface.pixel));
    series.weight.push(f64_f32(fine.weight));
    series.support.push(f64_f32(fine.field_support));
    if fine.hit.slot.is_none() {
        series.missed += 1;
        series.own_y.push(0.0);
        series.field_y.push(0.0);
        series.seed_y.push(0.0);
    } else {
        series.own_y.push(f64_f32(cell_luma(fine)));
        series.field_y.push(f64_f32(luma(fine.field)));
        series.seed_y.push(f64_f32(luma(fine.seed)));
        let origin = add_scaled(fine.position, fine.normal, RAY_LIFT);
        if inside_scene(scene, origin) || inside_scene(scene, fine.position) {
            series.inside += 1;
        }
    }
    series.coarse_y.push(f64_f32(cell_luma(coarse)));
}

fn print_series(name: &str, surface: &Surface, series: &Series, reference: f64) {
    let settled = if series.display_y.len() > 12 {
        &series.display_y[12..]
    } else {
        &series.display_y[..]
    };
    let (d_mean, d_var) = mean_var(settled);
    let d_min = settled.iter().copied().fold(f64::MAX, f64::min);
    let d_max = settled.iter().copied().fold(0.0_f64, f64::max);
    let (p_mean, p_var) = mean_var(&series.pixel_y);
    let own_settled = if series.own_y.len() > 12 {
        &series.own_y[12..]
    } else {
        &series.own_y[..]
    };
    let (o_mean, o_var) = mean_var(own_settled);
    let (f_mean, f_var) = mean_var(&series.field_y);
    let (s_mean, s_var) = mean_var(&series.seed_y);
    let (c_mean, c_var) = mean_var(&series.coarse_y);
    let p_min = series.pixel_y.iter().copied().fold(f64::MAX, f64::min);
    let p_max = series.pixel_y.iter().copied().fold(0.0_f64, f64::max);
    let o_min = own_settled.iter().copied().fold(f64::MAX, f64::min);
    let o_max = own_settled.iter().copied().fold(0.0_f64, f64::max);
    let black = series
        .pixel_y
        .iter()
        .filter(|y| **y < 0.2 * p_mean.max(reference).max(1.0e-4) && p_mean.max(reference) > 0.01)
        .count();
    eprintln!(
        "{name} pixel {} facing {} visible {} blocked {} ref {reference:.4}",
        surface.pixel, surface.facing, surface.visible, surface.blocked
    );
    eprintln!(
        "{name} display_y settled mean {d_mean:.4} var {d_var:.6} min {d_min:.4} max {d_max:.4}"
    );
    eprintln!(
        "{name} pixel_y mean {p_mean:.4} var {p_var:.6} min {p_min:.4} max {p_max:.4} black_frames {black}"
    );
    eprintln!(
        "{name} own_y mean {o_mean:.4} var {o_var:.6} min {o_min:.4} max {o_max:.4} inside {} missed {}",
        series.inside, series.missed
    );
    eprintln!("{name} field_y mean {f_mean:.4} var {f_var:.6} seed_y mean {s_mean:.4} var {s_var:.6} coarse_y mean {c_mean:.4} var {c_var:.6}");
}

fn blank_series() -> Series {
    Series {
        pixel_y: Vec::new(),
        display_y: Vec::new(),
        own_y: Vec::new(),
        field_y: Vec::new(),
        seed_y: Vec::new(),
        coarse_y: Vec::new(),
        weight: Vec::new(),
        support: Vec::new(),
        inside: 0,
        missed: 0,
    }
}

struct TraceRun {
    shadow: Series,
    penumbra: Series,
    lit_pixels: u32,
    drops: u32,
    drop_note: String,
}

fn collect_trace(
    window: &mut Window,
    renderer: &mut Renderer,
    stage: &Stage,
    camera: &Camera,
    shadow: &Surface,
    penumbra: &Surface,
) -> TraceRun {
    let scene = &stage.world.scene;
    let mut shadow_series = blank_series();
    let mut penumbra_series = blank_series();
    let mut baseline = Vec::<f64>::new();
    let mut drops = 0u32;
    let mut lit_pixels = 0u32;
    let mut drop_note = String::new();
    for frame in 0..TRACE_FRAMES {
        let _ = window.pump();
        let displayed = renderer
            .draw(&stage.world, camera, true)
            .expect("draw")
            .expect("pixels");
        let picture = renderer.read_scene().expect("scene");
        let queries = [
            query(shadow.point, shadow.normal, TEXEL),
            query(shadow.point, shadow.normal, WORLD),
            query(penumbra.point, penumbra.normal, TEXEL),
            query(penumbra.point, penumbra.normal, WORLD),
        ];
        let cells = renderer.read_cache_cells(&queries).expect("cells");
        note_frame(
            scene,
            shadow,
            &picture.rgb,
            &displayed,
            &cells[0],
            &cells[1],
            &mut shadow_series,
        );
        note_frame(
            scene,
            penumbra,
            &picture.rgb,
            &displayed,
            &cells[2],
            &cells[3],
            &mut penumbra_series,
        );
        if frame == 16 {
            baseline = (0..WIDTH * HEIGHT)
                .map(|pixel| display_luma(&displayed, pixel))
                .collect();
            lit_pixels = u32::try_from(baseline.iter().filter(|y| **y > 0.05).count()).unwrap_or(0);
        }
        if frame > 16 {
            for (pixel, before) in baseline.iter().enumerate() {
                if *before <= 0.05 {
                    continue;
                }
                let now = display_luma(&displayed, u32::try_from(pixel).unwrap_or(0));
                if now < *before * 0.2 {
                    drops += 1;
                    if drops <= 4 {
                        let _ = write!(
                            drop_note,
                            " frame {frame} pixel {pixel} {before:.4}->{now:.4}"
                        );
                    }
                }
            }
        }
        if frame == 0 || frame + 1 == TRACE_FRAMES || frame % 4 == 0 {
            log_watch(frame, scene, shadow, penumbra, &picture, &displayed, &cells);
        }
    }
    TraceRun {
        shadow: shadow_series,
        penumbra: penumbra_series,
        lit_pixels,
        drops,
        drop_note,
    }
}

fn log_watch(
    frame: u32,
    scene: &genos_scene::Scene,
    shadow: &Surface,
    penumbra: &Surface,
    picture: &genos_render::SceneRead,
    displayed: &[u8],
    cells: &[StoredCell],
) {
    let fine = &cells[0];
    let origin = add_scaled(fine.position, fine.normal, RAY_LIFT);
    eprintln!(
        "frame {frame} scene {}x{} shadow px {:.4} disp {:.4} own {:.4} field {:.4} seed {:.4} w {:.1} sup {:.2} coarse {:.4} slot {:?} inside {} lift {:?}",
        picture.width,
        picture.height,
        scene_luma(&picture.rgb, shadow.pixel),
        display_luma(displayed, shadow.pixel),
        cell_luma(fine),
        luma(fine.field),
        luma(fine.seed),
        fine.weight,
        fine.field_support,
        cell_luma(&cells[1]),
        fine.hit.slot,
        inside_scene(scene, origin),
        origin
    );
    let pen = &cells[2];
    eprintln!(
        "frame {frame} penumbra px {:.4} disp {:.4} own {:.4} field {:.4} seed {:.4} w {:.1} sup {:.2} coarse {:.4} slot {:?}",
        scene_luma(&picture.rgb, penumbra.pixel),
        display_luma(displayed, penumbra.pixel),
        cell_luma(pen),
        luma(pen.field),
        luma(pen.seed),
        pen.weight,
        pen.field_support,
        cell_luma(&cells[3]),
        pen.hit.slot
    );
}

/// Full shadow and penumbra, one still hall view. Each frame records the
/// picture, the 12.5 cm texel, the 25 cm cell, the seed, and whether the
/// cache ray origin is inside the wall.
#[test]
fn shadow_and_penumbra_pixels_trace_the_cache() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    renderer.set_lighting_settings(LightingSettings::preset(LightingPreset::High));
    renderer.reset_lighting_cache();
    let (stage, camera) = hall();
    let scene = &stage.world.scene;
    let shadow = find_surface(scene, &camera, true).expect("full shadow pixel");
    let penumbra = find_surface(scene, &camera, false).expect("penumbra pixel");
    eprintln!(
        "watch shadow {} at {:?} n {:?} penumbra {} at {:?} n {:?}",
        shadow.pixel, shadow.point, shadow.normal, penumbra.pixel, penumbra.point, penumbra.normal
    );
    let traced = collect_trace(window, renderer, &stage, &camera, &shadow, &penumbra);
    let shadow_series = &traced.shadow;
    let penumbra_series = &traced.penumbra;
    let shadow_ref = f64_f32(luma(radiance(
        scene,
        shadow.point,
        shadow.normal,
        shadow.albedo,
    )));
    let penumbra_ref = f64_f32(luma(radiance(
        scene,
        penumbra.point,
        penumbra.normal,
        penumbra.albedo,
    )));
    print_series("shadow", &shadow, shadow_series, shadow_ref);
    print_series("penumbra", &penumbra, penumbra_series, penumbra_ref);
    eprintln!(
        "lit_pixels {} black_drops {}{}",
        traced.lit_pixels, traced.drops, traced.drop_note
    );
    assert_eq!(
        shadow_series.inside, 0,
        "shadow cache ray origin was inside geometry"
    );
    assert_eq!(
        penumbra_series.inside, 0,
        "penumbra cache ray origin was inside geometry"
    );
    let (_shadow_mean, shadow_var) = mean_var(&shadow_series.display_y[12..]);
    let (_penumbra_mean, penumbra_var) = mean_var(&penumbra_series.display_y[12..]);
    assert!(
        shadow_var < 0.001,
        "settled shadow picture variance {shadow_var} (a black flicker is far above this)"
    );
    assert!(
        penumbra_var < 1.0e-6,
        "settled penumbra picture variance {penumbra_var}"
    );
    assert_eq!(
        traced.drops, 0,
        "a lit pixel fell below a fifth of its frame-16 value"
    );
}

#[derive(Clone)]
struct Spot {
    position: [f32; 3],
    normal: [f32; 3],
    distance: f32,
}

fn off_screen(camera: &Camera, point: [f32; 3]) -> bool {
    let aspect = f32_u32(WIDTH) / f32_u32(HEIGHT);
    match viewport_uv(camera, aspect, point) {
        Some(uv) => !(0.0..=1.0).contains(&uv[0]) || !(0.0..=1.0).contains(&uv[1]),
        None => true,
    }
}

fn nearby_spots(scene: &genos_scene::Scene, camera: &Camera) -> Vec<Spot> {
    let eye = eye_of(camera);
    let mut spots = Vec::new();
    let mut turn = -8i32;
    while turn <= 8 {
        let dir = look_direction(camera.yaw + f32_i32(turn) * 0.4, 0.0);
        if let Some(hit) = trace_ray(scene, eye, [dir.x, dir.y, dir.z], NEAR) {
            push_spot(&mut spots, camera, hit.point, hit.normal);
        }
        turn += 1;
    }
    let mut x = -5i32;
    while x <= 5 {
        let mut z = -5i32;
        while z <= 5 {
            push_spot(
                &mut spots,
                camera,
                [eye[0] + f32_i32(x), 0.02, eye[2] + f32_i32(z)],
                [0.0, 1.0, 0.0],
            );
            z += 1;
        }
        x += 1;
    }
    spots.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap_or(std::cmp::Ordering::Equal));
    spots.truncate(32);
    spots
}

fn push_spot(spots: &mut Vec<Spot>, camera: &Camera, point: [f32; 3], normal: [f32; 3]) {
    if !off_screen(camera, point) {
        return;
    }
    let distance = length(sub(point, eye_of(camera)));
    if !(0.05..=NEAR).contains(&distance) {
        return;
    }
    if spots.iter().any(|spot| length(sub(spot.position, point)) < 0.2) {
        return;
    }
    spots.push(Spot {
        position: point,
        normal,
        distance,
    });
}

fn aim(eye: [f32; 3], target: [f32; 3]) -> (f32, f32) {
    let d = sub(target, eye);
    let len = length(d).max(1.0e-4);
    let pitch = (d[1] / len).clamp(-0.4, 0.4).asin();
    let yaw = d[0].atan2(-d[2]);
    (yaw, pitch)
}

/// The 12.5 cm texel does not exist until a pixel shows the surface. This
/// records the seed written then against the 25 cm cell and against the fine
/// texel after it has had frames of its own.
#[test]
fn a_turn_seeds_fine_texels_from_the_world_cell() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    renderer.set_lighting_settings(LightingSettings::preset(LightingPreset::High));
    renderer.reset_lighting_cache();
    let (stage, mut camera) = hall();
    let spots = nearby_spots(&stage.world.scene, &camera);
    assert!(spots.len() >= 8, "need off-screen surfaces to turn toward");
    for _ in 0..16 {
        let _ = window.pump();
        renderer
            .draw(&stage.world, &camera, true)
            .expect("draw")
            .expect("pixels");
    }
    let coarse_q: Vec<CacheQuery> = spots
        .iter()
        .map(|spot| query(spot.position, spot.normal, WORLD))
        .collect();
    let coarse = renderer.read_cache_cells(&coarse_q).expect("coarse");
    let mut acc = [0.0f32; 3];
    for spot in &spots {
        acc = add(acc, spot.position);
    }
    let n = f32_u32(u32::try_from(spots.len()).unwrap_or(1)).max(1.0);
    let (yaw, pitch) = aim(eye_of(&camera), scale(acc, 1.0 / n));
    camera.set_pose(camera.position, yaw, pitch);
    let on_screen: Vec<bool> = spots
        .iter()
        .map(|spot| !off_screen(&camera, spot.position))
        .collect();
    let shown = on_screen.iter().filter(|on| **on).count();
    eprintln!("turn yaw {yaw:.3} pitch {pitch:.3} on_screen {shown} of {}", spots.len());
    for _ in 0..4 {
        let _ = window.pump();
        renderer
            .draw(&stage.world, &camera, true)
            .expect("draw")
            .expect("pixels");
    }
    let fine_q: Vec<CacheQuery> = spots
        .iter()
        .map(|spot| query(spot.position, spot.normal, TEXEL))
        .collect();
    let early = renderer.read_cache_cells(&fine_q).expect("early fine");
    for _ in 0..20 {
        let _ = window.pump();
        renderer
            .draw(&stage.world, &camera, true)
            .expect("draw")
            .expect("pixels");
    }
    let late = renderer.read_cache_cells(&fine_q).expect("late fine");
    let mut compared = 0u32;
    let mut seed_gap = 0.0f64;
    let mut coarse_gap = 0.0f64;
    for (i, spot) in spots.iter().enumerate() {
        if !on_screen[i] {
            continue;
        }
        let seed = luma(early[i].seed);
        let own = cell_luma(&late[i]);
        let parent = cell_luma(&coarse[i]);
        let denom = own.max(parent).max(0.02);
        let seeded = late[i].weight >= 1.0;
        let seed_rel = f64_f32((seed - own).abs() / denom);
        let parent_rel = f64_f32((parent - own).abs() / denom);
        if seeded {
            seed_gap = seed_gap.max(seed_rel);
            coarse_gap = coarse_gap.max(parent_rel);
        }
        compared += 1;
        eprintln!(
            "spot {i} dist {:.2} early_slot {:?} late_w {:.1} seed {:.4} conf {:.2} parent {:.4} own {:.4} seed_rel {seed_rel:.3} parent_rel {parent_rel:.3}",
            spot.distance,
            early[i].hit.slot,
            late[i].weight,
            seed,
            early[i].seed_weight,
            parent,
            own
        );
    }
    eprintln!("compared {compared} max_seed_rel {seed_gap:.3} max_parent_rel {coarse_gap:.3}");
    assert!(compared >= 4, "the turn did not bring nearby surfaces on screen");
}

fn trunc_u32(v: f32) -> u32 {
    if !v.is_finite() || v <= 0.0 {
        return 0;
    }
    let bits = v.to_bits();
    let biased = i32::from(u8::try_from((bits >> 23) & 0xff).unwrap_or(0));
    let exp = biased - 127;
    if exp < 0 {
        return 0;
    }
    if exp >= 31 {
        return u32::MAX;
    }
    let frac = (bits & 0x7f_ffff) | 0x80_0000;
    let shift = 23 - exp;
    if shift >= 0 {
        let places = u32::try_from(shift).unwrap_or(31);
        frac >> places
    } else {
        let places = u32::try_from(-shift).unwrap_or(0);
        frac << places
    }
}

fn pixel_of(camera: &Camera, point: [f32; 3]) -> Option<u32> {
    let aspect = f32_u32(WIDTH) / f32_u32(HEIGHT);
    let uv = viewport_uv(camera, aspect, point)?;
    if !(0.0..=1.0).contains(&uv[0]) || !(0.0..=1.0).contains(&uv[1]) {
        return None;
    }
    let x = trunc_u32(uv[0] * f32_u32(WIDTH)).min(WIDTH - 1);
    let y = trunc_u32((1.0 - uv[1]) * f32_u32(HEIGHT)).min(HEIGHT - 1);
    Some(y * WIDTH + x)
}

fn draw_presented(
    window: &mut Window,
    renderer: &mut Renderer,
    stage: &Stage,
    camera: &Camera,
) -> Vec<u8> {
    let _ = window.pump();
    renderer
        .draw(&stage.world, camera, true)
        .expect("draw")
        .expect("pixels")
}

struct YawTurn {
    shown: u32,
    filled: u32,
    mean_weight: f64,
    pixel_delta: f64,
}

fn yaw_once(window: &mut Window, renderer: &mut Renderer, stage: &Stage, fill: bool) -> YawTurn {
    let mut settings = LightingSettings::preset(LightingPreset::High);
    if !fill {
        settings.surface_cache.fine_lod_create = 0.0;
        settings.surface_cache.fine_lod_drop = 0.0;
        settings.surface_cache.fine_budget_share = 0.0;
        settings.surface_cache.fine_screen_boost = 1.0;
        settings.surface_cache.fine_close_boost = 0.0;
    }
    settings.sanitize();
    renderer.set_lighting_settings(settings);
    renderer.reset_lighting_cache();
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(14.5, 1.7, 23.5), -0.3554, 0.08);
    let spots = nearby_spots(&stage.world.scene, &camera);
    for _ in 0..24 {
        let _ = draw_presented(window, renderer, stage, &camera);
    }
    let fine_q: Vec<CacheQuery> = spots
        .iter()
        .map(|spot| query(spot.position, spot.normal, TEXEL))
        .collect();
    let before = renderer.read_cache_cells(&fine_q).expect("fine before the turn");
    let mut acc = [0.0f32; 3];
    for spot in &spots {
        acc = add(acc, spot.position);
    }
    let nspots = f32_u32(u32::try_from(spots.len()).unwrap_or(1)).max(1.0);
    let (yaw, pitch) = aim(eye_of(&camera), scale(acc, 1.0 / nspots));
    camera.set_pose(camera.position, yaw, pitch);
    let mut shown = 0u32;
    let mut filled = 0u32;
    let mut weight_sum = 0.0f64;
    let mut on_screen = Vec::with_capacity(spots.len());
    for (i, spot) in spots.iter().enumerate() {
        let on = !off_screen(&camera, spot.position);
        on_screen.push(on);
        if !on {
            continue;
        }
        shown += 1;
        let ready = before[i].hit.slot.is_some() && before[i].weight >= 4.0;
        if ready {
            filled += 1;
            weight_sum += f64_f32(before[i].weight);
        }
        eprintln!(
            "fill {fill} spot {i} dist {:.2} slot {:?} weight {:.1} luma {:.4} ready {ready}",
            spot.distance,
            before[i].hit.slot,
            before[i].weight,
            cell_luma(&before[i])
        );
    }
    let entry = draw_presented(window, renderer, stage, &camera);
    let mut later = entry.clone();
    for _ in 0..16 {
        later = draw_presented(window, renderer, stage, &camera);
    }
    let mut delta = 0.0f64;
    let mut counted = 0u32;
    for (i, spot) in spots.iter().enumerate() {
        if !on_screen[i] {
            continue;
        }
        let Some(px) = pixel_of(&camera, spot.position) else {
            continue;
        };
        let a = display_luma(&entry, px);
        let b = display_luma(&later, px);
        delta += (a - b).abs();
        counted += 1;
        eprintln!("fill {fill} spot {i} pixel {px} entry {a:.4} later {b:.4}");
    }
    let pixel_delta = if counted == 0 {
        0.0
    } else {
        delta / f64::from(counted)
    };
    let mean_weight = if filled == 0 {
        0.0
    } else {
        weight_sum / f64::from(filled)
    };
    eprintln!(
        "fill {fill} shown {shown} filled {filled} mean_weight {mean_weight:.2} pixel_delta {pixel_delta:.4} counted {counted}"
    );
    YawTurn {
        shown,
        filled,
        mean_weight,
        pixel_delta,
    }
}

/// Surfaces near the player already hold a fine texel before the camera turns
/// toward them, so the presented pixel does not feed in on entry.
#[test]
fn a_yaw_turn_finds_nearby_surfaces_already_filled() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let (stage, _) = hall();
    let off = yaw_once(window, renderer, &stage, false);
    let on = yaw_once(window, renderer, &stage, true);
    assert!(off.shown >= 4, "the turn brought {} surfaces on screen", off.shown);
    assert!(on.shown >= 4, "the turn brought {} surfaces on screen", on.shown);
    assert!(
        on.filled * 2 >= on.shown,
        "fine texels before the turn: {} of {} (mean weight {:.2})",
        on.filled,
        on.shown,
        on.mean_weight
    );
    assert!(
        off.pixel_delta > 0.01,
        "the unfilled turn moved the picture by only {:.4}",
        off.pixel_delta
    );
    assert!(
        on.pixel_delta < off.pixel_delta * 0.6,
        "entry change {:.4} with the band, {:.4} without",
        on.pixel_delta,
        off.pixel_delta
    );
}
