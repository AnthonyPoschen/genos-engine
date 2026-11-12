#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! Nearby world cells the picture has not shown yet.
//!
//! A world cell (25 cm) is supposed to be claimed and filled around the camera
//! in every direction, so a turn reveals light that is already there. This
//! counts the cells behind and beside a still hall camera, and whether they
//! hold samples, before any turn.

use std::sync::Mutex;

use genos_render::{sees, trace_ray, CacheQuery, DirectSample, Renderer, StoredCell};
use genos_scene::{look_direction, viewport_uv, Camera, LightingPreset, LightingSettings, Vec3};
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};
use genos_window::Window;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const STILL_FRAMES: u32 = 24;
const SHADOW_FRAMES: u32 = 16;
const WORLD: f32 = 0.25;
const TEXEL: f32 = 0.125;
const NEAR: f32 = 6.0;

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

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn add_scaled(p: [f32; 3], n: [f32; 3], scale: f32) -> [f32; 3] {
    [p[0] + n[0] * scale, p[1] + n[1] * scale, p[2] + n[2] * scale]
}

fn off_screen(camera: &Camera, point: [f32; 3]) -> bool {
    let aspect = f32_u32(WIDTH) / f32_u32(HEIGHT);
    match viewport_uv(camera, aspect, point) {
        Some(uv) => !(0.0..=1.0).contains(&uv[0]) || !(0.0..=1.0).contains(&uv[1]),
        None => true,
    }
}

#[derive(Clone)]
struct Spot {
    position: [f32; 3],
    normal: [f32; 3],
    distance: f32,
    behind: bool,
}

fn push_spot(spots: &mut Vec<Spot>, camera: &Camera, forward: [f32; 3], point: [f32; 3], normal: [f32; 3]) {
    if !off_screen(camera, point) {
        return;
    }
    let eye = eye_of(camera);
    let distance = length(sub(point, eye));
    if distance > NEAR || distance < 0.05 {
        return;
    }
    let behind = dot(sub(point, eye), forward) < 0.0;
    if spots.iter().any(|spot| length(sub(spot.position, point)) < 0.2) {
        return;
    }
    spots.push(Spot {
        position: point,
        normal,
        distance,
        behind,
    });
}

fn nearby_spots(scene: &genos_scene::Scene, camera: &Camera) -> Vec<Spot> {
    let eye = eye_of(camera);
    let forward = look_direction(camera.yaw, camera.pitch);
    let forward_a = [forward.x, forward.y, forward.z];
    let mut spots = Vec::new();
    let mut turn = -8i32;
    while turn <= 8 {
        let dir = look_direction(camera.yaw + f32_i32(turn) * 0.4, 0.0);
        if let Some(hit) = trace_ray(scene, eye, [dir.x, dir.y, dir.z], NEAR) {
            push_spot(&mut spots, camera, forward_a, hit.point, hit.normal);
        }
        turn += 1;
    }
    let mut x = -5i32;
    while x <= 5 {
        let mut z = -5i32;
        while z <= 5 {
            let point = [eye[0] + f32_i32(x), 0.02, eye[2] + f32_i32(z)];
            push_spot(&mut spots, camera, forward_a, point, [0.0, 1.0, 0.0]);
            z += 1;
        }
        x += 1;
    }
    let mut behind: Vec<Spot> = spots.iter().filter(|spot| spot.behind).cloned().collect();
    let mut beside: Vec<Spot> = spots.iter().filter(|spot| !spot.behind).cloned().collect();
    behind.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap_or(std::cmp::Ordering::Equal));
    beside.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap_or(std::cmp::Ordering::Equal));
    behind.truncate(20);
    beside.truncate(12);
    behind.extend(beside);
    behind
}

fn queries(spots: &[Spot], size: f32) -> Vec<CacheQuery> {
    spots
        .iter()
        .map(|spot| CacheQuery {
            position: spot.position,
            normal: spot.normal,
            size,
        })
        .collect()
}

struct Band {
    exist: u32,
    filled: u32,
    missing: u32,
}

fn band_of(spots: &[Spot], cells: &[StoredCell], limit: f32, behind: Option<bool>) -> Band {
    let mut band = Band {
        exist: 0,
        filled: 0,
        missing: 0,
    };
    for (spot, cell) in spots.iter().zip(cells.iter()) {
        if spot.distance > limit {
            continue;
        }
        if behind.is_some_and(|want| spot.behind != want) {
            continue;
        }
        if cell.hit.slot.is_none() {
            band.missing += 1;
            continue;
        }
        band.exist += 1;
        if cell.weight >= 1.0 {
            band.filled += 1;
        }
    }
    band
}

fn draw_frames(window: &mut Window, renderer: &mut Renderer, stage: &Stage, camera: &Camera, frames: u32) {
    for _ in 0..frames {
        let _ = window.pump();
        renderer.draw(&stage.world, camera, true).expect("draw").expect("pixels");
    }
}

/// Still view, before any turn: world cells within 6 m behind and beside the
/// camera exist and already hold samples. A cell that was never claimed, or
/// claimed and left at weight 0, is the pop-in.
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "GENOS_GPU_TIMES is read when the renderer opens; the test sets it before that"
)]
fn nearby_offscreen_world_cells_hold_samples_before_they_are_seen() {
    std::env::set_var("GENOS_GPU_TIMES", "1");
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    renderer.set_lighting_settings(LightingSettings::preset(LightingPreset::High));
    renderer.reset_lighting_cache();
    let (stage, camera) = hall();
    let spots = nearby_spots(&stage.world.scene, &camera);
    assert!(spots.len() >= 4, "the hall has no nearby off-screen surfaces");
    let mut scout_frames = 0u32;
    let mut max_thr = 0u32;
    for frame in 0..STILL_FRAMES {
        let _ = window.pump();
        renderer
            .draw(&stage.world, &camera, true)
            .expect("draw")
            .expect("pixels");
        let round = renderer.read_cache_round().expect("cache round");
        if round.scouts > 0 {
            scout_frames += 1;
        }
        max_thr = max_thr.max(round.thr);
        eprintln!(
            "frame {frame} rays {} due {} waiting {} scouts {} claimed {} thr {} live {}",
            round.rays, round.due, round.waiting, round.scouts, round.claimed, round.thr, round.live
        );
    }
    let world = renderer
        .read_cache_cells(&queries(&spots, WORLD))
        .expect("world cells");
    let fine = renderer
        .read_cache_cells(&queries(&spots, TEXEL))
        .expect("fine texels");
    for (limit, name) in [(1.0, "1m"), (3.0, "3m"), (NEAR, "6m")] {
        let behind = band_of(&spots, &world, limit, Some(true));
        let beside = band_of(&spots, &world, limit, Some(false));
        eprintln!(
            "band {name} behind exist {} filled {} missing {} | beside exist {} filled {} missing {}",
            behind.exist, behind.filled, behind.missing, beside.exist, beside.filled, beside.missing
        );
    }
    let fine_exist = fine.iter().filter(|cell| cell.hit.slot.is_some()).count();
    eprintln!(
        "probes {} scout_frames {scout_frames} max_thr {max_thr} fine_exist {fine_exist}",
        spots.len()
    );
    for (spot, cell) in spots.iter().zip(world.iter()) {
        eprintln!(
            "cell behind {} dist {:.2} slot {:?} weight {:.2} updated {} shown {} pos {:?} n {:?}",
            spot.behind,
            spot.distance,
            cell.hit.slot,
            cell.weight,
            cell.updated,
            cell.shown_frame,
            spot.position,
            spot.normal
        );
    }
    let passes = renderer.lighting_pass_ms();
    let restir = renderer.lighting_restir_ms();
    eprintln!(
        "pass_ms compact {:.3} place {:.3} trace {:.3} direct {:.3} gather {:.3} filter {:.3} cache {:.3} compose {:.3} atrous {:.3} cache_rays {:.3} hit_light {:.3} shadows {:.3}",
        passes[0], passes[1], passes[2], passes[3], passes[4], passes[5], passes[6], passes[7], passes[8], passes[9], passes[10], passes[11]
    );
    eprintln!(
        "restir_ms initial {:.3} temporal {:.3} spatial {:.3} prepare {:.3} shade {:.3}",
        restir[0], restir[1], restir[2], restir[3], restir[4]
    );
    let near = band_of(&spots, &world, NEAR, None);
    assert!(
        near.exist >= 4,
        "scouts claimed {exist} nearby off-screen world cells (missing {missing}, scout frames {scout_frames})",
        exist = near.exist,
        missing = near.missing
    );
    assert!(
        near.filled * 2 >= near.exist,
        "nearby off-screen world cells were claimed and left empty: {filled} of {exist} hold a sample (thr {max_thr})",
        filled = near.filled,
        exist = near.exist
    );
}

struct ShadeClass {
    pixel: u32,
    facing: u32,
    visible: u32,
    blocked: u32,
    disagree: u32,
}

fn point_lamps(scene: &genos_scene::Scene, from: [f32; 3], normal: [f32; 3]) -> (u32, u32, u32, u32) {
    let mut facing = 0u32;
    let mut visible = 0u32;
    let mut blocked = 0u32;
    let mut disagree = 0u32;
    let surface = add_scaled(from, normal, 0.001);
    let offset = add_scaled(from, normal, 0.02);
    for light in &scene.lights {
        let dir = light.direction;
        if dir.x * dir.x + dir.y * dir.y + dir.z * dir.z > 1.0e-8 {
            continue;
        }
        let pos = [light.position.x, light.position.y, light.position.z];
        let toward = sub(pos, surface);
        if dot(toward, normal) <= 0.0 {
            continue;
        }
        facing += 1;
        let from_surface = sees(scene, surface, pos);
        let from_offset = sees(scene, offset, pos);
        if from_surface != from_offset {
            disagree += 1;
        }
        if from_surface {
            visible += 1;
        } else {
            blocked += 1;
        }
    }
    (facing, visible, blocked, disagree)
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

fn classify_pixels(
    scene: &genos_scene::Scene,
    camera: &Camera,
) -> (Option<ShadeClass>, Option<ShadeClass>, Option<ShadeClass>) {
    let eye = eye_of(camera);
    let mut lit = None;
    let mut shadow = None;
    let mut penumbra = None;
    let mut y = 8u32;
    while y + 8 < HEIGHT {
        let mut x = 8u32;
        while x + 8 < WIDTH {
            let dir = ray_dir(camera, x, y);
            if let Some(hit) = trace_ray(scene, eye, dir, 40.0) {
                let (facing, visible, blocked, disagree) = point_lamps(scene, hit.point, hit.normal);
                let class = ShadeClass {
                    pixel: y * WIDTH + x,
                    facing,
                    visible,
                    blocked,
                    disagree,
                };
                if facing > 0 && blocked == 0 && lit.is_none() {
                    lit = Some(class);
                } else if facing > 0 && visible == 0 && shadow.is_none() {
                    shadow = Some(class);
                } else if visible > 0 && blocked > 0 && penumbra.is_none() {
                    penumbra = Some(class);
                }
            }
            if lit.is_some() && shadow.is_some() && penumbra.is_some() {
                return (lit, shadow, penumbra);
            }
            x += 16;
        }
        y += 12;
    }
    (lit, shadow, penumbra)
}

fn point_sum(scene: &genos_scene::Scene, origin: [f32; 3], normal: [f32; 3]) -> [f32; 3] {
    // `LAMP_UNIT` and `LAMP_RADIUS` in `crates/render/src/trace.rs`. Point lamps only:
    // the visibility mean does not include the sun.
    const LAMP_UNIT: f32 = 72.0;
    const LAMP_RADIUS: f32 = 0.1;
    let mut sum = [0.0f32; 3];
    let from = add_scaled(origin, normal, 0.001);
    for light in &scene.lights {
        let dir = light.direction;
        if dir.x * dir.x + dir.y * dir.y + dir.z * dir.z > 1.0e-8 {
            continue;
        }
        let pos = [light.position.x, light.position.y, light.position.z];
        let toward = sub(pos, from);
        let dist = length(toward).max(1.0e-4);
        let nd = dot(toward, normal) / dist;
        if nd <= 0.0 || !sees(scene, from, pos) {
            continue;
        }
        let scale = nd * LAMP_UNIT / (dist * dist).max(LAMP_RADIUS * LAMP_RADIUS);
        sum[0] += light.color[0] * scale;
        sum[1] += light.color[1] * scale;
        sum[2] += light.color[2] * scale;
    }
    sum
}

fn cpu_point(scene: &genos_scene::Scene, camera: &Camera, pixel: u32) -> [f32; 3] {
    let x = pixel % WIDTH;
    let y = pixel / WIDTH;
    let Some(hit) = trace_ray(scene, eye_of(camera), ray_dir(camera, x, y), 40.0) else {
        return [0.0; 3];
    };
    point_sum(scene, hit.point, hit.normal)
}

fn rec709(rgb: [f32; 3]) -> f32 {
    rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722
}

fn print_sample(label: &str, class: &ShadeClass, sample: &DirectSample, cpu: [f32; 3]) {
    eprintln!(
        "{label} pixel {} facing {} visible {} blocked {} disagree {} mean_m {} mean_vis {} mean_y {:.4} cpu_y {:.4} mean_rgb {:?} cpu_rgb {:?} restir_m {} restir_w {:.4} restir_target {:.4}",
        class.pixel,
        class.facing,
        class.visible,
        class.blocked,
        class.disagree,
        sample.mean_m,
        sample.mean_vis,
        rec709(sample.mean_rgb),
        rec709(cpu),
        sample.mean_rgb,
        cpu,
        sample.restir_m,
        sample.restir_w,
        sample.restir_target
    );
}

/// A fully lit pixel and a fully shadowed pixel keep a visibility mean.
/// The shadow ray starts 2 cm along the normal; a disagreement with the
/// surface is the ray starting inside the solid.
#[test]
fn shadow_and_lit_pixels_keep_a_visibility_mean() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    renderer.set_lighting_settings(LightingSettings::preset(LightingPreset::High));
    renderer.reset_lighting_cache();
    let (stage, camera) = hall();
    let (lit, shadow, penumbra) = classify_pixels(&stage.world.scene, &camera);
    let lit = lit.expect("a fully lit hall pixel");
    let shadow = shadow.expect("a fully shadowed hall pixel");
    let penumbra = penumbra.expect("a penumbra hall pixel");
    assert_eq!(lit.disagree, 0, "lit shadow ray origin disagrees with the surface");
    assert_eq!(shadow.disagree, 0, "shadow ray origin disagrees with the surface");
    assert_eq!(penumbra.disagree, 0, "penumbra shadow ray origin disagrees with the surface");
    draw_frames(window, renderer, &stage, &camera, SHADOW_FRAMES);
    let samples = renderer
        .read_direct_samples(&[lit.pixel, shadow.pixel, penumbra.pixel])
        .expect("direct samples");
    let lit_sample = samples.first().expect("lit sample");
    let shadow_sample = samples.get(1).expect("shadow sample");
    let penumbra_sample = samples.get(2).expect("penumbra sample");
    let scene = &stage.world.scene;
    print_sample("lit", &lit, lit_sample, cpu_point(scene, &camera, lit.pixel));
    print_sample("shadow", &shadow, shadow_sample, cpu_point(scene, &camera, shadow.pixel));
    print_sample("penumbra", &penumbra, penumbra_sample, cpu_point(scene, &camera, penumbra.pixel));
    for (label, sample) in [("lit", lit_sample), ("shadow", shadow_sample), ("penumbra", penumbra_sample)] {
        assert!(
            sample.restir_w.is_finite() && sample.restir_w >= 0.0,
            "{label} reservoir weight is not a finite weight"
        );
        assert!(
            sample.mean_kind == 3 && sample.mean_m >= 8,
            "{label} mean did not accumulate: kind {} m {} rgb {:?}",
            sample.mean_kind,
            sample.mean_m,
            sample.mean_rgb
        );
    }
}
