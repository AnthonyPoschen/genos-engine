#![cfg_attr(
    test,
    expect(clippy::expect_used, reason = "a test aborts when a fixture fails")
)]

//! A held camera at a pitch that resolves the 12.5 cm texel must not keep the
//! first frame's black bounce after the shadow field has filled.
//!
//! 320×180 puts about 10 cm between samples, so one texel hides inside a
//! pixel. 960×540 puts about 3 cm between samples on the far wall, which is
//! the 1440p case: each texel is several pixels, and a stored miss stays a
//! black pixel.

use std::sync::Mutex;

use genos_render::{
    linear_from_display, luminance, trace_ray, CacheQuery, DebugView, Renderer, StoredCell,
    ViewMode,
};
use genos_scene::{look_direction, Camera, LightingPreset, LightingSettings, Vec3};
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};
use genos_window::Window;

const WIDTH: u32 = 960;
const HEIGHT: u32 = 540;
const FRAMES: u32 = 40;
/// Four direct-mask draws run before the timed loop, and the lighting frame
/// increments at the start of each draw. Loop index 27 is lighting frame 32.
/// The settled window is lighting frames 32–39.
const SETTLED_AT: usize = 27;
const SETTLED_END: usize = 35;
const STRIDE: u32 = 8;
const TEXEL: f32 = 0.125;
/// Presented linear luma below this is a black shadow pixel. Full indirect
/// shadow on this hall sits near 0.035 once the field has filled.
const BLACK: f64 = 0.02;
const ZERO: f64 = 1.0e-4;

fn open() -> (MutexGuard, genos_window::Lifecycle<Renderer>) {
    static GPU: Mutex<()> = Mutex::new(());
    let guard = GPU
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
}

fn surface_at(scene: &genos_scene::Scene, camera: &Camera, pixel: u32) -> Option<Surface> {
    let x = pixel % WIDTH;
    let y = pixel / WIDTH;
    let dir = ray_dir(camera, x, y);
    let hit = trace_ray(scene, eye_of(camera), dir, 40.0)?;
    Some(Surface {
        pixel,
        point: hit.point,
        normal: hit.normal,
    })
}

/// Surfaces whose presented direct light is black. `direct` is the tonemapped
/// direct view after a few frames, so a cleared opening frame is not the mask.
fn shadow_samples(scene: &genos_scene::Scene, camera: &Camera, direct: &[u8]) -> Vec<Surface> {
    let mut out = Vec::new();
    let mut hits = 0u32;
    let mut direct_y = Vec::new();
    let mut y = STRIDE;
    while y + STRIDE < HEIGHT {
        let mut x = STRIDE;
        while x + STRIDE < WIDTH {
            let pixel = y * WIDTH + x;
            if let Some(hit) = surface_at(scene, camera, pixel) {
                hits += 1;
                let luma = display_luma(direct, pixel);
                direct_y.push(luma);
                if luma < 0.01 {
                    out.push(hit);
                }
            }
            x += STRIDE;
        }
        y += STRIDE;
    }
    eprintln!(
        "shadow_grain surface_hits {hits} direct_median {:.4} direct_zero {}",
        median(&direct_y),
        out.len()
    );
    out
}

fn display_luma(pixels: &[u8], pixel: u32) -> f64 {
    let at = usize::try_from(pixel).unwrap_or(0).saturating_mul(4);
    let b = f32_u32(u32::from(pixels.get(at).copied().unwrap_or(0))) / 255.0;
    let g = f32_u32(u32::from(
        pixels.get(at.saturating_add(1)).copied().unwrap_or(0),
    )) / 255.0;
    let r = f32_u32(u32::from(
        pixels.get(at.saturating_add(2)).copied().unwrap_or(0),
    )) / 255.0;
    f64_f32(luminance([
        linear_from_display(r),
        linear_from_display(g),
        linear_from_display(b),
    ]))
}

fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    sorted.get(mid).copied().unwrap_or(0.0)
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

fn cell_luma(cell: &StoredCell) -> f32 {
    if cell.hit.slot.is_none() {
        return 0.0;
    }
    luminance(cell.field)
}

/// Full-shadow samples on a still hall, at a pixel pitch finer than the texel.
/// The first frame is a cold cache. Later frames must show the filled indirect
/// instead of holding that first miss.
#[test]
fn a_held_shadow_does_not_keep_the_first_black_bounce() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    renderer.set_lighting_settings(LightingSettings::preset(LightingPreset::High));
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Direct,
        lamp: 0,
    });
    let (stage, camera) = hall();
    let mut direct_picture = Vec::new();
    for _ in 0..4 {
        let _ = window.pump();
        direct_picture = renderer
            .draw(&stage.world, &camera, true)
            .expect("direct draw")
            .expect("direct pixels");
    }
    let samples = shadow_samples(&stage.world.scene, &camera, &direct_picture);
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Full,
        lamp: 0,
    });
    renderer.reset_lighting_cache();
    assert!(
        samples.len() > 200,
        "shadow sample count {} is too small to judge grain",
        samples.len()
    );
    let watched_index = samples
        .iter()
        .position(|sample| sample.normal[1].abs() < 0.2)
        .unwrap_or(0);
    let watched = samples.get(watched_index).expect("shadow sample");
    let walls = samples
        .iter()
        .filter(|sample| sample.normal[1].abs() < 0.2)
        .count();
    eprintln!(
        "shadow_grain samples {} walls {walls} watched pixel {} at {:?} n {:?}",
        samples.len(),
        watched.pixel,
        watched.point,
        watched.normal
    );
    let mut series: Vec<Vec<f64>> = samples.iter().map(|_| Vec::new()).collect();
    let mut medians = Vec::new();
    for frame in 0..FRAMES {
        let _ = window.pump();
        let displayed = renderer
            .draw(&stage.world, &camera, true)
            .expect("draw")
            .expect("pixels");
        let mut frame_y = Vec::with_capacity(samples.len());
        for (sample, row) in samples.iter().zip(series.iter_mut()) {
            let y = display_luma(&displayed, sample.pixel);
            row.push(y);
            frame_y.push(y);
        }
        let dark = frame_y.iter().filter(|y| **y < BLACK).count();
        let zeros = frame_y.iter().filter(|y| **y < ZERO).count();
        medians.push(median(&frame_y));
        let query = CacheQuery {
            position: samples
                .get(watched_index)
                .map(|sample| sample.point)
                .unwrap_or([0.0, 0.0, 0.0]),
            normal: samples
                .get(watched_index)
                .map(|sample| sample.normal)
                .unwrap_or([0.0, 1.0, 0.0]),
            size: TEXEL,
        };
        let cells = renderer.read_cache_cells(&[query]).expect("cell");
        let field = cells.first().map(cell_luma).unwrap_or(0.0);
        let weight = cells.first().map(|cell| cell.weight).unwrap_or(0.0);
        eprintln!(
            "frame {frame} shadow_median {:.4} dark {dark}/{} zero {zeros} watched {:.4} field {:.4} weight {:.1}",
            median(&frame_y),
            samples.len(),
            series
                .get(watched_index)
                .and_then(|row| row.last())
                .copied()
                .unwrap_or(0.0),
            field,
            weight
        );
    }
    let late_start = 8usize;
    let mut black_px = 0usize;
    let mut zero_px = 0usize;
    let mut var_sum = 0.0f64;
    for row in &series {
        // Frames 8–15 are the climb. A stuck black bounce shows up here.
        // Variance of that climb is the fill, so the grain bound is the
        // settled window (lighting frames 32–39).
        let early = row.get(late_start..16).unwrap_or(&[]);
        let (mean, _) = mean_var(early);
        if mean < BLACK {
            black_px += 1;
        }
        if mean < ZERO {
            zero_px += 1;
        }
        let settled_row = row.get(SETTLED_AT..SETTLED_END).unwrap_or(&[]);
        let (_, var) = mean_var(settled_row);
        var_sum += var;
    }
    let n = f64_f32(f32_u32(u32::try_from(samples.len()).unwrap_or(0)));
    let var_mean = var_sum / n;
    let watched_late = series
        .get(watched_index)
        .and_then(|row| row.get(late_start..))
        .unwrap_or(&[]);
    let (watched_mean, watched_var) = mean_var(watched_late);
    eprintln!(
        "shadow_grain n={} black={black_px} zero={zero_px} watched_late_mean={watched_mean:.4} watched_late_var={watched_var:.6} var_mean={var_mean:.6}",
        samples.len()
    );
    let mut probes = Vec::new();
    for (sample, row) in samples.iter().zip(series.iter()) {
        let late = row.get(late_start..).unwrap_or(&[]);
        let (mean, _) = mean_var(late);
        if mean < ZERO {
            probes.push(CacheQuery {
                position: sample.point,
                normal: sample.normal,
                size: TEXEL,
            });
        }
        if probes.len() == 16 {
            break;
        }
    }
    if !probes.is_empty() {
        let cells = renderer.read_cache_cells(&probes).expect("zero cells");
        for (index, cell) in cells.iter().enumerate() {
            eprintln!(
                "zero_probe {index} field {:.4} weight {:.1} support {:.2} slot {:?}",
                cell_luma(cell),
                cell.weight,
                cell.field_support,
                cell.hit.slot
            );
        }
    }
    assert!(
        watched_mean > BLACK,
        "watched shadow pixel late mean {watched_mean:.4} stayed under {BLACK}"
    );
    // Exact display zero, not the dim indirect on a dark ceiling. Before the
    // empty-bounce fix this was 47 of 502, each with a field near 0.8–1.1.
    assert!(
        zero_px * 20 < samples.len(),
        "black shadow pixels {zero_px} of {} stayed at display zero after the field filled",
        samples.len()
    );
    assert!(
        var_mean < 1.0e-3,
        "mean temporal variance {var_mean:.6} of settled shadow pixels"
    );
    let early = medians.get(8..16).unwrap_or(&[]);
    let settled = medians.get(SETTLED_AT..SETTLED_END).unwrap_or(&[]);
    let (early_mean, _) = mean_var(early);
    let (settled_mean, settled_var) = mean_var(settled);
    let mut settled_zero = 0usize;
    let mut wiped = 0usize;
    for row in &series {
        let window = row.get(SETTLED_AT..SETTLED_END).unwrap_or(&[]);
        let (mean, _) = mean_var(window);
        if mean < ZERO {
            settled_zero += 1;
        }
        let before = row.get(12..16).unwrap_or(&[]);
        let (before_mean, _) = mean_var(before);
        if before_mean > BLACK && window.iter().any(|y| *y < ZERO) {
            wiped += 1;
        }
    }
    let mut max_step = 0.0f64;
    let mut prev = medians.get(4).copied().unwrap_or(0.0);
    for value in medians.iter().copied().skip(5) {
        let step = (value - prev).abs();
        if step > max_step {
            max_step = step;
        }
        prev = value;
    }
    eprintln!(
        "shadow_grain early_median {early_mean:.4} settled_median {settled_mean:.4} settled_var {settled_var:.6} settled_zero {settled_zero} wiped {wiped} max_step {max_step:.4}"
    );
    assert!(
        settled_zero == 0,
        "settled window kept {settled_zero} exact-zero shadow samples"
    );
    assert!(
        wiped == 0,
        "a later miss replaced a lit shadow sample on {wiped} pixels"
    );
    // The old hold stepped the median from about 0.08 to 0.35 at frame 32.
    assert!(
        max_step < 0.08,
        "shadow median stepped {max_step:.4} on a held camera"
    );
    assert!(
        settled_var < 1.0e-4,
        "settled window variance {settled_var:.6}"
    );
}

/// The sun band reuses a stored bounce outside this frame's row. A frozen sun
/// never takes that path. A one-tick sun step must recover the same shadow
/// and then walk a band, not every dark pixel.
#[test]
fn a_small_sun_step_does_not_keep_the_first_black_bounce() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    renderer.set_lighting_settings(LightingSettings::preset(LightingPreset::High));
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Direct,
        lamp: 0,
    });
    let (mut stage, camera) = hall();
    let mut direct_picture = Vec::new();
    for _ in 0..4 {
        let _ = window.pump();
        direct_picture = renderer
            .draw(&stage.world, &camera, true)
            .expect("direct draw")
            .expect("direct pixels");
    }
    let samples = shadow_samples(&stage.world.scene, &camera, &direct_picture);
    assert!(
        samples.len() > 200,
        "shadow sample count {} is too small to judge the sun band",
        samples.len()
    );
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Full,
        lamp: 0,
    });
    renderer.reset_lighting_cache();
    stage.sun_frozen = false;
    let mut series: Vec<Vec<f64>> = samples.iter().map(|_| Vec::new()).collect();
    let mut saw_sun = false;
    let mut late_walks = u32::MAX;
    for frame in 0..FRAMES {
        stage.advance(1.0 / 60.0);
        let _ = window.pump();
        let displayed = renderer
            .draw(&stage.world, &camera, true)
            .expect("draw")
            .expect("pixels");
        let mut frame_y = Vec::with_capacity(samples.len());
        for (sample, row) in samples.iter().zip(series.iter_mut()) {
            let y = display_luma(&displayed, sample.pixel);
            row.push(y);
            frame_y.push(y);
        }
        let zeros = frame_y.iter().filter(|y| **y < ZERO).count();
        let sun = renderer.lighting_sun_stats();
        if frame >= 8 && sun.gpu_sun {
            saw_sun = true;
            late_walks = late_walks.min(sun.walks);
        }
        eprintln!(
            "sun frame {frame} shadow_median {:.4} zero {zeros}/{} gpu_sun {} walks {}",
            median(&frame_y),
            samples.len(),
            u32::from(sun.gpu_sun),
            sun.walks
        );
    }
    let mut zero_px = 0usize;
    for row in &series {
        let late = row.get(8..).unwrap_or(&[]);
        let (mean, _) = mean_var(late);
        if mean < ZERO {
            zero_px += 1;
        }
    }
    eprintln!(
        "sun_grain n={} zero={zero_px} saw_sun {saw_sun} late_walks {late_walks}",
        samples.len()
    );
    assert!(saw_sun, "a one-tick sun step never took the sun compose");
    let pixels = WIDTH * HEIGHT;
    assert!(
        late_walks * 4 < pixels,
        "sun compose walked {late_walks} of {pixels} (a band is about an eighth)"
    );
    assert!(
        zero_px * 20 < samples.len(),
        "black shadow pixels {zero_px} of {} stayed at display zero on the sun path",
        samples.len()
    );
}
