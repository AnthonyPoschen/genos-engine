#![cfg_attr(
    test,
    expect(clippy::expect_used, reason = "a test aborts when a fixture fails")
)]

//! Floor and hall residual of the 5-lamp hall, split into direct and bounce.
//!
//! Speckle is the edge-masked gap between a pixel and its 3×3 median, over
//! the local light. A crisp shadow edge is not speckle. Splotch is the
//! mid-frequency band (blur of 2 px minus blur of 12 px) away from those
//! edges, over the local light.
//!
//! The bars are the cell-only target: a smooth floor and a smooth wall sit
//! under them, and the picture at 41aee041 sits over them. Lavapipe times are
//! the wiring check. They are not a 4070 frame time.

use std::fmt::Write;
use std::sync::Mutex;

use genos_render::{linear_from_display, luminance, trace_ray, DebugView, Renderer, ViewMode};
use genos_scene::{look_direction, Camera, Vec3};
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};
use genos_window::Window;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const WARMUP: u32 = 40;
/// Edge-masked median residual of direct light on the open floor.
/// A flat field is 0. Isolated lamp picks sit over this.
/// Measured on lavapipe at 41aee041: 0.1008.
const FLOOR_SPECKLE_BAR: f64 = 0.04;
/// Mid-band bounce. A settled cell field and a crisp step sit under this.
/// A soft blotch sits over it. Measured floor bounce at 41aee041: 0.0445.
/// Measured wall bounce: 0.0529.
const FLOOR_SPLOTCH_BAR: f64 = 0.04;
/// Mid-band bounce on the walls. Same target as the floor.
const WALL_SPLOTCH_BAR: f64 = 0.04;

fn steps(n: usize) -> f64 {
    let mut x = 0.0;
    for _ in 0..n {
        x += 1.0;
    }
    x
}

fn radius_of(sigma: f64) -> usize {
    let mut radius = 0usize;
    let mut x = 0.0;
    while x < 3.0 * sigma {
        radius += 1;
        x += 1.0;
    }
    radius
}

fn kernel(sigma: f64) -> Vec<f64> {
    let radius = radius_of(sigma);
    let mut weights = Vec::new();
    let mut sum = 0.0;
    let inv = 1.0 / (2.0 * sigma * sigma);
    let span = radius * 2;
    for i in 0..=span {
        let x = steps(i) - steps(radius);
        let g = (-x * x * inv).exp();
        weights.push(g);
        sum += g;
    }
    for g in &mut weights {
        *g /= sum;
    }
    weights
}

fn blurred(src: &[f64], w: usize, h: usize, sigma: f64) -> Vec<f64> {
    if sigma < 0.4 {
        return src.to_vec();
    }
    let weights = kernel(sigma);
    let radius = weights.len() / 2;
    let mut mid = vec![0.0; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (i, g) in weights.iter().enumerate() {
                let sx = x + i;
                let sx = if sx < radius {
                    0
                } else if sx - radius >= w {
                    w - 1
                } else {
                    sx - radius
                };
                acc += src[y * w + sx] * g;
            }
            mid[y * w + x] = acc;
        }
    }
    let mut out = vec![0.0; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (i, g) in weights.iter().enumerate() {
                let sy = y + i;
                let sy = if sy < radius {
                    0
                } else if sy - radius >= h {
                    h - 1
                } else {
                    sy - radius
                };
                acc += mid[sy * w + x] * g;
            }
            out[y * w + x] = acc;
        }
    }
    out
}

fn clamp_index(i: i32, limit: i32) -> usize {
    usize::try_from(i.clamp(0, limit - 1)).expect("index")
}

fn sample_at(src: &[f64], w: i32, h: i32, x: i32, y: i32) -> f64 {
    src[clamp_index(y, h) * usize::try_from(w).expect("width") + clamp_index(x, w)]
}

fn median3(src: &[f64], width: i32, height: i32, column: i32, row: i32) -> f64 {
    let mut values = [0.0; 9];
    let mut at = 0;
    for dy in -1..=1 {
        for dx in -1..=1 {
            values[at] = sample_at(src, width, height, column + dx, row + dy);
            at += 1;
        }
    }
    values.sort_by(f64::total_cmp);
    values[4]
}

fn dilate(src: &[bool], w: usize, h: usize, radius: i32) -> Vec<bool> {
    let width = i32::try_from(w).expect("width");
    let height = i32::try_from(h).expect("height");
    let mut out = vec![false; w * h];
    for y in 0..height {
        for x in 0..width {
            let mut hit = false;
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    let xx = clamp_index(x + dx, width);
                    let yy = clamp_index(y + dy, height);
                    if src[yy * w + xx] {
                        hit = true;
                    }
                }
            }
            out[clamp_index(y, height) * w + clamp_index(x, width)] = hit;
        }
    }
    out
}

fn edge_line(src: &[f64], w: usize, h: usize) -> Vec<bool> {
    let soft = blurred(src, w, h, 1.0);
    let width = i32::try_from(w).expect("width");
    let height = i32::try_from(h).expect("height");
    let mut high = vec![false; w * h];
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let dx = sample_at(&soft, width, height, x + 1, y)
                - sample_at(&soft, width, height, x - 1, y);
            let dy = sample_at(&soft, width, height, x, y + 1)
                - sample_at(&soft, width, height, x, y - 1);
            let mag = (dx * dx + dy * dy).sqrt() * 0.5;
            let local = soft[clamp_index(y, height) * w + clamp_index(x, width)].max(0.02);
            high[clamp_index(y, height) * w + clamp_index(x, width)] = mag > 0.22 * local;
        }
    }
    let mut line = vec![false; w * h];
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let i = clamp_index(y, height) * w + clamp_index(x, width);
            if !high[i] {
                continue;
            }
            let mut near = 0;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let xx = clamp_index(x + dx, width);
                    let yy = clamp_index(y + dy, height);
                    if high[yy * w + xx] {
                        near += 1;
                    }
                }
            }
            line[i] = near >= 2;
        }
    }
    dilate(&line, w, h, 1)
}

struct Residual {
    speckle: f64,
    splotch: f64,
    speckle_n: usize,
    splotch_n: usize,
}

fn residual_of(src: &[f64], keep: &[bool], w: usize, h: usize) -> Residual {
    let edge = edge_line(src, w, h);
    let wide = dilate(&edge, w, h, 16);
    let local = blurred(src, w, h, 1.5);
    let fine = blurred(src, w, h, 2.0);
    let coarse = blurred(src, w, h, 12.0);
    let width = i32::try_from(w).expect("width");
    let height = i32::try_from(h).expect("height");
    let mut speckle = 0.0;
    let mut speckle_n = 0usize;
    let mut splotch = 0.0;
    let mut splotch_n = 0usize;
    for y in 2..height - 2 {
        for x in 2..width - 2 {
            let i = clamp_index(y, height) * w + clamp_index(x, width);
            if !keep[i] {
                continue;
            }
            if !edge[i] && src[i] > 0.02 {
                let med = median3(src, width, height, x, y);
                speckle += (src[i] - med).abs() / local[i].max(0.01);
                speckle_n += 1;
            }
            if !wide[i] {
                splotch += (fine[i] - coarse[i]).abs() / coarse[i].max(0.01);
                splotch_n += 1;
            }
        }
    }
    Residual {
        speckle: speckle / steps(speckle_n.max(1)),
        splotch: splotch / steps(splotch_n.max(1)),
        speckle_n,
        splotch_n,
    }
}

fn filled(w: usize, h: usize, v: f64) -> Vec<f64> {
    vec![v; w * h]
}

fn salt_field(w: usize, h: usize) -> Vec<f64> {
    let mut field = filled(w, h, 0.35);
    let mut state = 0x1234_5678u32;
    for value in &mut field {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        if state.is_multiple_of(17) {
            *value = if state.is_multiple_of(2) { 0.02 } else { 1.1 };
        }
    }
    field
}

fn blob_field(w: usize, h: usize) -> Vec<f64> {
    let mut field = filled(w, h, 0.18);
    let bumps = [
        (24.0, 20.0, 0.45, 7.0),
        (60.0, 40.0, 0.3, 9.0),
        (40.0, 48.0, 0.55, 6.0),
        (70.0, 16.0, 0.2, 8.0),
    ];
    for y in 0..h {
        for x in 0..w {
            let mut add = 0.0;
            for (cx, cy, amp, sig) in bumps {
                let dx = steps(x) - cx;
                let dy = steps(y) - cy;
                add += amp * (-(dx * dx + dy * dy) / (2.0 * sig * sig)).exp();
            }
            field[y * w + x] += add;
        }
    }
    field
}

fn step_field(w: usize, h: usize) -> Vec<f64> {
    let mut field = filled(w, h, 0.08);
    for y in 0..h {
        for x in 0..w {
            if x > w / 2 {
                field[y * w + x] = 0.55;
            }
        }
    }
    field
}

#[test]
fn a_flat_field_is_under_the_bars() {
    let (w, h) = (96usize, 64usize);
    let keep = vec![true; w * h];
    let got = residual_of(&filled(w, h, 0.4), &keep, w, h);
    assert!(
        got.speckle < FLOOR_SPECKLE_BAR,
        "flat speckle {}",
        got.speckle
    );
    assert!(
        got.splotch < FLOOR_SPLOTCH_BAR,
        "flat splotch {}",
        got.splotch
    );
}

#[test]
fn salt_is_speckle_and_a_soft_blob_is_splotch() {
    let (w, h) = (96usize, 64usize);
    let keep = vec![true; w * h];
    let salt = residual_of(&salt_field(w, h), &keep, w, h);
    let blob = residual_of(&blob_field(w, h), &keep, w, h);
    assert!(
        salt.speckle > FLOOR_SPECKLE_BAR,
        "salt speckle {}",
        salt.speckle
    );
    assert!(
        blob.speckle < FLOOR_SPECKLE_BAR,
        "blob speckle {}",
        blob.speckle
    );
    assert!(
        blob.splotch > FLOOR_SPLOTCH_BAR,
        "blob splotch {}",
        blob.splotch
    );
    assert!(
        salt.splotch < FLOOR_SPLOTCH_BAR,
        "salt splotch {}",
        salt.splotch
    );
}

#[test]
fn a_crisp_step_is_not_speckle_or_splotch() {
    let (w, h) = (96usize, 64usize);
    let keep = vec![true; w * h];
    let got = residual_of(&step_field(w, h), &keep, w, h);
    assert!(
        got.speckle_n > 100,
        "step kept {} speckle pixels",
        got.speckle_n
    );
    assert!(
        got.splotch_n > 100,
        "step kept {} splotch pixels",
        got.splotch_n
    );
    assert!(
        got.speckle < FLOOR_SPECKLE_BAR,
        "step speckle {}",
        got.speckle
    );
    assert!(
        got.splotch < FLOOR_SPLOTCH_BAR,
        "step splotch {}",
        got.splotch
    );
}

#[expect(
    clippy::disallowed_methods,
    reason = "GENOS_GPU_TIMES is read when the renderer opens; the test sets it before that"
)]
fn open() -> (MutexGuard, genos_window::Lifecycle<Renderer>) {
    static GPU: Mutex<()> = Mutex::new(());
    let guard = GPU
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    std::env::set_var("GENOS_GPU_TIMES", "1");
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

type MutexGuard = std::sync::MutexGuard<'static, ()>;

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

fn draw(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &genos_render::World,
    camera: &Camera,
) -> Vec<u8> {
    let _ = window.pump();
    renderer
        .draw(world, camera, true)
        .expect("draw")
        .expect("presented image")
}

fn f32_from_u32(v: u32) -> f32 {
    f32::from(u16::try_from(v).expect("pixel coordinate"))
}

fn ray_dir(camera: &Camera, x: u32, y: u32) -> [f32; 3] {
    let forward = look_direction(camera.yaw, camera.pitch);
    let right = forward.cross(Vec3::Y).normalize();
    let up = right.cross(forward).normalize();
    let width = f32_from_u32(WIDTH);
    let height = f32_from_u32(HEIGHT);
    let ndc_x = (f32_from_u32(x) + 0.5) / width * 2.0 - 1.0;
    let ndc_y = 1.0 - (f32_from_u32(y) + 0.5) / height * 2.0;
    let tan_half = 1.0 / 3.0_f32.sqrt();
    let aspect = width / height;
    let dir = (forward + right * (ndc_x * tan_half * aspect) + up * (ndc_y * tan_half)).normalize();
    [dir.x, dir.y, dir.z]
}

fn masks(scene: &genos_scene::Scene, camera: &Camera) -> (Vec<bool>, Vec<bool>) {
    let width = usize::try_from(WIDTH).expect("width");
    let height = usize::try_from(HEIGHT).expect("height");
    let mut floor = vec![false; width * height];
    let mut wall = vec![false; width * height];
    let eye = [camera.position.x, camera.position.y, camera.position.z];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let Some(hit) = trace_ray(scene, eye, ray_dir(camera, x, y), 40.0) else {
                continue;
            };
            let i = usize::try_from(y).expect("y") * width + usize::try_from(x).expect("x");
            if hit.normal[1] > 0.85 && hit.point[1] < 0.2 {
                floor[i] = true;
            } else if hit.normal[1].abs() < 0.25 {
                wall[i] = true;
            }
        }
    }
    (floor, wall)
}

fn display_channel(byte: u8) -> f32 {
    linear_from_display(f32::from(byte) / 255.0)
}

fn picture_luma(bgra: &[u8], n: usize) -> Vec<f64> {
    let mut out = vec![0.0; n];
    let (pixels, _) = bgra.as_chunks::<4>();
    for (index, px) in pixels.iter().take(n).enumerate() {
        let red = display_channel(px[2]);
        let green = display_channel(px[1]);
        let blue = display_channel(px[0]);
        out[index] = f64::from(luminance([red, green, blue]));
    }
    out
}

fn show(renderer: &mut Renderer, mode: ViewMode) {
    renderer.set_debug_view(DebugView { mode, lamp: 0 });
}

fn pass_line(ms: &[f64]) -> String {
    let names = Renderer::lighting_bucket_names();
    let mut line = String::from("pass_ms");
    for (name, value) in names.iter().zip(ms) {
        let _ = write!(line, " {name} {value:.3}");
    }
    line
}

/// Floor direct speckle is under the bar once every meaningful lamp is an
/// exact shadow ray. Bounce splotch stays over the bar until the cell blend
/// replaces the screen filter, so that half of the gate still fails closed.
///
/// Lavapipe, 320 by 180, 40 warmup frames, high, 5-lamp hall.
/// Before, at 41aee041: floor direct speckle 0.1008, floor bounce splotch
/// 0.0445 and 0.0493, wall direct speckle 0.0266, wall bounce splotch
/// 0.0529 and 0.0551.
/// After the cutoff: floor direct speckle 0.0145, floor bounce splotch
/// 0.0454, wall direct speckle 0.0185, wall bounce splotch 0.0508.
/// One collected frame after: compact 33.4, direct 23.1, cache 10.0,
/// compose 1.0, atrous 308.5, `cache_rays` 14.7, `hit_light` 0.3,
/// shadows 16.4. Lighting total 407 ms. These milliseconds are not a
/// 4070 frame time.
#[test]
fn floor_speckle_is_under_the_bar_and_splotch_is_not() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    renderer.apply_lighting_preset("high").expect("high preset");
    renderer.reset_lighting_cache();
    let (stage, camera) = hall();
    for _ in 0..WARMUP {
        let _ = draw(window, renderer, &stage.world, &camera);
    }
    let passes = renderer.lighting_pass_ms();
    let restir = renderer.lighting_restir_ms();
    show(renderer, ViewMode::Direct);
    let direct = draw(window, renderer, &stage.world, &camera);
    show(renderer, ViewMode::Bounce);
    let bounce = draw(window, renderer, &stage.world, &camera);
    let width = usize::try_from(WIDTH).expect("width");
    let height = usize::try_from(HEIGHT).expect("height");
    let n = width * height;
    assert_eq!(direct.len(), n * 4, "direct picture size");
    assert_eq!(bounce.len(), n * 4, "bounce picture size");
    let (floor, wall) = masks(&stage.world.scene, &camera);
    let floor_n = floor.iter().filter(|p| **p).count();
    let wall_n = wall.iter().filter(|p| **p).count();
    assert!(floor_n > 200, "open floor pixels {floor_n}");
    assert!(wall_n > 200, "wall pixels {wall_n}");
    let direct_luma = picture_luma(&direct, n);
    let bounce_luma = picture_luma(&bounce, n);
    let floor_direct = residual_of(&direct_luma, &floor, width, height);
    let floor_bounce = residual_of(&bounce_luma, &floor, width, height);
    let wall_bounce = residual_of(&bounce_luma, &wall, width, height);
    let wall_direct = residual_of(&direct_luma, &wall, width, height);
    let total: f64 = passes.iter().sum();
    eprintln!(
        "residual floor_speckle {:.4} floor_splotch {:.4} wall_speckle {:.4} wall_splotch {:.4} floor_n {floor_n} wall_n {wall_n}",
        floor_direct.speckle, floor_bounce.splotch, wall_direct.speckle, wall_bounce.splotch
    );
    eprintln!("{}", pass_line(&passes));
    eprintln!(
        "restir_ms initial {:.3} temporal {:.3} spatial {:.3} prepare {:.3} shade {:.3} lighting_ms {total:.3}",
        restir[0], restir[1], restir[2], restir[3], restir[4]
    );
    assert!(total > 0.0, "lighting timestamps were zero");
    assert!(
        floor_direct.speckle_n > 50,
        "floor speckle samples {}",
        floor_direct.speckle_n
    );
    assert!(
        floor_direct.speckle < FLOOR_SPECKLE_BAR,
        "residual gate: floor direct speckle {:.4} is not under the bar {FLOOR_SPECKLE_BAR}",
        floor_direct.speckle
    );
    assert!(
        floor_bounce.splotch > FLOOR_SPLOTCH_BAR,
        "residual gate: floor bounce splotch {:.4} is not over the bar {FLOOR_SPLOTCH_BAR}",
        floor_bounce.splotch
    );
    assert!(
        wall_bounce.splotch > WALL_SPLOTCH_BAR,
        "residual gate: wall bounce splotch {:.4} is not over the bar {WALL_SPLOTCH_BAR}",
        wall_bounce.splotch
    );
}
