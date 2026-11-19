#![cfg_attr(test, expect(clippy::as_conversions, reason = "pixel index is width * height * 4"))]
#![cfg_attr(test, expect(clippy::cast_precision_loss, reason = "a mean over a face"))]
#![cfg_attr(test, expect(clippy::cast_possible_truncation, reason = "a window edge fits the frame"))]
#![cfg_attr(test, expect(clippy::cast_sign_loss, reason = "a window edge is non-negative"))]
#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! Frame-to-frame flicker of a moving box under a still sun.
//!
//! The red box spins about 0.57 degrees a frame. Its face is one world cell
//! across. A cell a mover is changing takes a ray every frame; that ray has
//! to keep the stored mean when it agrees. Replacing a fraction of the mean
//! with one noisy ray paints a cell-sized blotch that appears and vanishes.
//! The face's mean |delta luma| stays within a small factor of a static wall.

use std::sync::Mutex;

use genos_render::Renderer;
use genos_scene::{Camera, Vec3};
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};
use genos_window::Window;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
/// Past the 40-frame opening, and long enough for a 1-in-10 wall cell to settle.
/// The face keeps moving after that: it takes a ray every frame.
const WARMUP: usize = 120;
/// Consecutive presented frames. The blotch is every frame, so a short run sees it.
const MEASURE: usize = 40;
/// A 0.57 degree step of this face is a few tenths of a code. The wall is a
/// few hundredths. The factor covers that step. The catch path, which
/// replaces the mean with one ray, measured 0.95 mean-delta and 1.95 per
/// pixel on this harness, with 5 of 39 pairs at or above 2. The judged path
/// measured 0.45 to 0.63, 0.92 to 1.13 per pixel, and at most two such pairs.
const FACTOR: f64 = 16.0;
const SLACK: f64 = 0.28;
const HOT_LIMIT: usize = 3;

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

fn scene() -> (Stage, Camera) {
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
    stage.boxes_still = false;
    stage.lamps_still = true;
    stage.apply();
    // Eye (13, 1.9, 23.2), looking at (13, 0.5, 19.5): the red box's path.
    let eye = Vec3::new(13.0, 1.9, 23.2);
    let at = Vec3::new(13.0, 0.5, 19.5);
    let dx = at.x - eye.x;
    let dy = at.y - eye.y;
    let dz = at.z - eye.z;
    // Yaw 0 looks along -Z. Positive yaw turns toward +X.
    let yaw = dx.atan2(-dz);
    let pitch = dy.atan2((dx * dx + dz * dz).sqrt());
    let mut camera = Camera::opening();
    camera.set_pose(eye, yaw, pitch);
    (stage, camera)
}

fn draw(window: &mut Window, renderer: &mut Renderer, world: &genos_render::World, camera: &Camera) -> Vec<u8> {
    let _ = window.pump();
    renderer
        .draw(world, camera, true)
        .expect("draw")
        .expect("presented image: the mover gate reads the swapchain")
}

fn luma_at(bgra: &[u8], width: usize, x: usize, y: usize) -> f64 {
    let p = (y * width + x) * 4;
    0.2126 * f64::from(bgra[p + 2]) + 0.7152 * f64::from(bgra[p + 1]) + 0.0722 * f64::from(bgra[p])
}

/// Red-dominant pixels. A dark blotch on the red box still has more red than green.
fn red_pixel(bgra: &[u8], width: usize, x: usize, y: usize) -> bool {
    let p = (y * width + x) * 4;
    let b = u32::from(bgra[p]);
    let g = u32::from(bgra[p + 1]);
    let r = u32::from(bgra[p + 2]);
    r > 40 && r > g + 18 && r > b + 18
}

struct WindowBox {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
}

fn red_window(bgra: &[u8], width: usize, height: usize) -> WindowBox {
    let mut x0 = width;
    let mut y0 = height;
    let mut x1 = 0usize;
    let mut y1 = 0usize;
    let mut n = 0usize;
    for y in 0..height {
        for x in 0..width {
            if red_pixel(bgra, width, x, y) {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
                n += 1;
            }
        }
    }
    assert!(n >= 40, "mover flicker: the red box face was not in frame ({n} red pixels)");
    // Inset so the silhouette sliding a pixel does not enter the mean.
    let inset = 2usize;
    if x1 > x0 + inset * 2 && y1 > y0 + inset * 2 {
        x0 += inset;
        y0 += inset;
        x1 -= inset;
        y1 -= inset;
    }
    WindowBox { x0, y0, x1, y1 }
}

fn mean_luma(bgra: &[u8], width: usize, win: &WindowBox) -> f64 {
    let mut sum = 0.0f64;
    let mut n = 0u64;
    for y in win.y0..win.y1 {
        for x in win.x0..win.x1 {
            sum += luma_at(bgra, width, x, y);
            n += 1;
        }
    }
    if n == 0 { 0.0 } else { sum / n as f64 }
}

/// Same size as the face, in the top of the frame, off the red box.
fn static_window(face: &WindowBox, width: usize, height: usize) -> WindowBox {
    let w = face.x1 - face.x0;
    let h = face.y1 - face.y0;
    let x0 = (width / 2).saturating_sub(w / 2);
    let y0 = 2usize;
    WindowBox {
        x0,
        y0,
        x1: (x0 + w).min(width),
        y1: (y0 + h).min(height / 5),
    }
}

struct Run {
    avg: f64,
    max: f64,
    hot: usize,
    pairs: usize,
    /// Mean over pairs of the mean per-pixel |delta luma|. Cell blotches that
    /// cancel in the region mean still show up here.
    pixel: f64,
}

fn deltas(samples: &[f64]) -> Run {
    let mut sum = 0.0f64;
    let mut max = 0.0f64;
    let mut hot = 0usize;
    for pair in samples.windows(2) {
        let d = (pair[1] - pair[0]).abs();
        sum += d;
        if d > max {
            max = d;
        }
        if d >= 2.0 {
            hot += 1;
        }
    }
    let pairs = samples.len().saturating_sub(1);
    Run {
        avg: if pairs == 0 { 0.0 } else { sum / pairs as f64 },
        max,
        hot,
        pairs,
        pixel: 0.0,
    }
}

const ERODE: usize = 4;

fn interior(bgra: &[u8], width: usize, height: usize, x: usize, y: usize) -> bool {
    if x < ERODE || y < ERODE || x + ERODE >= width || y + ERODE >= height {
        return false;
    }
    let center = luma_at(bgra, width, x, y);
    for dy in 0..=ERODE * 2 {
        for dx in 0..=ERODE * 2 {
            let xx = x + dx - ERODE;
            let yy = y + dy - ERODE;
            if !red_pixel(bgra, width, xx, yy) {
                return false;
            }
            // The crease between two faces of the cube is a lighting step
            // the rotation sweeps across the mask. A flat neighbourhood is
            // one face. A cell blotch is flat too, so it still counts.
            if (luma_at(bgra, width, xx, yy) - center).abs() > 24.0 {
                return false;
            }
        }
    }
    true
}

/// Mean luma of the face interior, and the mean per-pixel |delta| where both
/// frames' interiors overlap. The silhouette is outside the erosion.
fn interior_pair(prev: &[u8], curr: &[u8], width: usize, height: usize) -> (f64, f64, usize) {
    let mut sum_c = 0.0f64;
    let mut sum_p = 0.0f64;
    let mut sum_d = 0.0f64;
    let mut n = 0u64;
    for y in ERODE..height.saturating_sub(ERODE) {
        for x in ERODE..width.saturating_sub(ERODE) {
            if interior(prev, width, height, x, y) && interior(curr, width, height, x, y) {
                let a = luma_at(prev, width, x, y);
                let b = luma_at(curr, width, x, y);
                sum_p += a;
                sum_c += b;
                sum_d += (a - b).abs();
                n += 1;
            }
        }
    }
    if n == 0 {
        (0.0, 0.0, 0)
    } else {
        let nf = n as f64;
        ((sum_c - sum_p).abs() / nf, sum_d / nf, n as usize)
    }
}

fn pixel_mad(prev: &[u8], curr: &[u8], width: usize, win: &WindowBox) -> f64 {
    let mut sum = 0.0f64;
    let mut n = 0u64;
    for y in win.y0..win.y1 {
        for x in win.x0..win.x1 {
            sum += (luma_at(curr, width, x, y) - luma_at(prev, width, x, y)).abs();
            n += 1;
        }
    }
    if n == 0 { 0.0 } else { sum / n as f64 }
}

struct Pass {
    face: Run,
    wall: Run,
    texels: usize,
    limit: f64,
    pixel_limit: f64,
}

fn measure(
    window: &mut Window,
    renderer: &mut Renderer,
    stage: &mut Stage,
    camera: &Camera,
    label: &str,
    settle: usize,
) -> Pass {
    let width = renderer.width() as usize;
    let height = renderer.height() as usize;
    for _ in 0..settle {
        stage.advance(1.0 / 60.0);
        let _ = draw(window, renderer, &stage.world, camera);
    }
    let mut face_l = Vec::with_capacity(MEASURE);
    let mut wall_l = Vec::with_capacity(MEASURE);
    let mut face_px = 0.0f64;
    let mut wall_px = 0.0f64;
    let mut px_pairs = 0usize;
    let mut texels = 0usize;
    let mut wall = None;
    let mut prev: Option<Vec<u8>> = None;
    let mut prev_mean = 0.0f64;
    let mut have_mean = false;
    for _ in 0..MEASURE {
        stage.advance(1.0 / 60.0);
        let pixels = draw(window, renderer, &stage.world, camera);
        if wall.is_none() {
            let found = red_window(&pixels, width, height);
            wall = Some(static_window(&found, width, height));
            eprintln!(
                "mover flicker {label} windows: face {}..{} x {}..{} wall {}..{} x {}..{}",
                found.x0,
                found.x1,
                found.y0,
                found.y1,
                wall.as_ref().expect("wall").x0,
                wall.as_ref().expect("wall").x1,
                wall.as_ref().expect("wall").y0,
                wall.as_ref().expect("wall").y1
            );
        }
        let wall = wall.as_ref().expect("wall window");
        if let Some(before) = prev.as_ref() {
            let (mean_delta, pixel, n) = interior_pair(before, &pixels, width, height);
            texels += n;
            if n >= 80 {
                face_px += pixel;
                px_pairs += 1;
                if !have_mean {
                    face_l.push(0.0);
                    have_mean = true;
                }
                prev_mean += mean_delta;
                face_l.push(prev_mean);
                wall_px += pixel_mad(before, &pixels, width, wall);
            }
        }
        wall_l.push(mean_luma(&pixels, width, wall));
        prev = Some(pixels);
    }
    let mut face = deltas(&face_l);
    let mut wall_run = deltas(&wall_l);
    if px_pairs > 0 {
        face.pixel = face_px / px_pairs as f64;
        wall_run.pixel = wall_px / px_pairs as f64;
    }
    let limit = wall_run.avg * FACTOR + SLACK;
    let pixel_limit = wall_run.pixel * FACTOR + SLACK;
    let texels = if px_pairs == 0 { 0 } else { texels / px_pairs };
    eprintln!(
        "mover flicker {label}: face mean-delta avg {:.3} max {:.3} hot {}/{} pixel {:.3} texels {texels} | wall avg {:.3} max {:.3} pixel {:.3} | limit {:.3} pixel-limit {:.3}",
        face.avg, face.max, face.hot, face.pairs, face.pixel, wall_run.avg, wall_run.max, wall_run.pixel, limit, pixel_limit
    );
    Pass {
        face,
        wall: wall_run,
        texels,
        limit,
        pixel_limit,
    }
}

#[test]
fn a_moving_box_face_stays_as_steady_as_the_wall() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    renderer.apply_lighting_preset("high").expect("high preset");
    renderer.reset_lighting_cache();
    let (mut stage, camera) = scene();
    for _ in 0..WARMUP {
        stage.advance(1.0 / 60.0);
        let _ = draw(window, renderer, &stage.world, &camera);
    }
    let full = measure(window, renderer, &mut stage, &camera, "full", 0);
    let gate = full;
    assert!(
        gate.wall.avg < 0.15,
        "mover flicker: the wall did not settle (avg {:.3})",
        gate.wall.avg
    );
    assert!(
        gate.face.avg <= gate.limit && gate.face.pixel <= gate.pixel_limit && gate.face.hot <= HOT_LIMIT,
        "mover flicker: face |delta luma| avg {:.3} max {:.3} pixel {:.3} ({}/{} pairs >= 2) vs wall avg {:.3} pixel {:.3}. \
         A cell a mover is changing must keep its mean when the fresh ray agrees. limit {:.3} pixel-limit {:.3} hot-limit {HOT_LIMIT} texels {}",
        gate.face.avg,
        gate.face.max,
        gate.face.pixel,
        gate.face.hot,
        gate.face.pairs,
        gate.wall.avg,
        gate.wall.pixel,
        gate.limit,
        gate.pixel_limit,
        gate.texels
    );
}
