#![cfg_attr(test, expect(clippy::as_conversions, reason = "pixel index is width * height * 4"))]
#![cfg_attr(test, expect(clippy::cast_precision_loss, reason = "a mean over the frame"))]
#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! Presented-frame knockout for the surface-cache fill (stage F0).
//!
//! The test reads the swapchain image. With the cache writing, the hall moves
//! off the first frames as the cache replaces the probe mix, and its error
//! against the path reference falls. With
//! `--features knockout` and `GENOS_KNOCKOUT=cache_fill`, those value writes
//! are frozen and this test goes red. The passes still dispatch.
//!
//! `scripts/ci-fill-knockout.sh` runs both.

use std::sync::Mutex;

use genos_debug::image::{pixel_luminance, Image};
use genos_render::Renderer;
use genos_scene::{Camera, Vec3};
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};
use genos_window::Window;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
/// First presented frame kept. Frame 0 is often the clear.
const EARLY: usize = 2;
/// Frames from a cleared cache. The hall is not filled by then (error against
/// the reference is still about 0.53, a filled hall is about 0.15). The picture
/// has left the probe mix and is moving toward the reference.
const LATE: usize = 40;
/// Mean absolute channel change, in 8-bit codes, from EARLY to LATE.
/// Measured on lavapipe at 40 frames: 8.8 codes with the fill running, 0.0 with
/// `cache_fill` knocked out, 1.8 with one cache slot in eight written.
const MIN_DELTA_CODES: f64 = 4.0;
/// The relative luminance error against the path reference must fall by this
/// much from EARLY to LATE. Measured: 0.594 to 0.534 (0.060, repeatable to
/// 0.0002) with the fill running, 0.000 knocked out. A fill that writes four
/// times too bright moves the picture 71 codes, which the delta check alone
/// passes, and the error rises from 0.18 to 1.07.
const MIN_ERROR_GAIN: f64 = 0.03;
/// Prefix of every assertion message. `scripts/ci-fill-knockout.sh` requires it
/// in the knocked-out run, so a build failure does not count as red.
const RED: &str = "fill gate red:";
/// The 72 spp path reference of this pose (640 x 360, 2 x 2 means to match).
const REFERENCE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/systems/lighting/experiments/img/hall-path-reference.png"
);
/// Sky in the reference, sRGB. Pixels within 2 codes are left out.
const SKY: [u8; 3] = [28, 36, 48];

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

fn draw(window: &mut Window, renderer: &mut Renderer, world: &genos_render::World, camera: &Camera) -> Vec<u8> {
    let _ = window.pump();
    renderer
        .draw(world, camera, true)
        .expect("draw")
        .expect("presented image: a lighting gate reads the swapchain")
}

fn mean_abs_codes(a: &[u8], b: &[u8]) -> f64 {
    let n = a.len().min(b.len()) / 4;
    assert!(n > 0, "presented image was empty");
    let mut sum = 0u64;
    for i in 0..n {
        let p = i * 4;
        sum += u64::from(a[p].abs_diff(b[p]));
        sum += u64::from(a[p + 1].abs_diff(b[p + 1]));
        sum += u64::from(a[p + 2].abs_diff(b[p + 2]));
    }
    sum as f64 / (n as f64 * 3.0)
}

/// Linear luminance of the path reference, 2 x 2 means to the picture size, and
/// a mask that is false on sky. The reference is 640 x 360.
fn reference_plane(width: usize, height: usize) -> (Vec<f32>, Vec<bool>) {
    let img = Image::load(std::path::Path::new(REFERENCE)).expect("path reference png");
    assert_eq!(
        (img.width as usize, img.height as usize),
        (width * 2, height * 2),
        "the reference is the 2x size of the presented picture"
    );
    let rw = img.width as usize;
    let mut lum = vec![0.0f32; width * height];
    let mut keep = vec![true; width * height];
    for y in 0..height {
        for x in 0..width {
            let mut sum = 0.0f32;
            let mut sky = 0;
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let at = ((y * 2 + dy) * rw + x * 2 + dx) * 3;
                let px = &img.rgb[at..at + 3];
                sum += pixel_luminance(px);
                if px.iter().zip(SKY).all(|(a, b)| a.abs_diff(b) <= 2) {
                    sky += 1;
                }
            }
            lum[y * width + x] = sum / 4.0;
            keep[y * width + x] = sky < 2;
        }
    }
    (lum, keep)
}

/// Relative luminance error of a presented BGRA picture against the reference,
/// sum |live - ref| / sum ref over the pixels that are not sky.
fn error_vs_reference(bgra: &[u8], reference: &(Vec<f32>, Vec<bool>)) -> f64 {
    let (lum, keep) = reference;
    assert_eq!(bgra.len(), lum.len() * 4, "picture and reference sizes differ");
    let (mut diff, mut base) = (0.0f64, 0.0f64);
    for (i, px) in bgra.as_chunks::<4>().0.iter().enumerate() {
        if !keep[i] {
            continue;
        }
        let live = pixel_luminance(&[px[2], px[1], px[0]]);
        diff += f64::from((live - lum[i]).abs());
        base += f64::from(lum[i]);
    }
    diff / base.max(1.0e-6)
}

/// The presented hall changes as the cache fills, and the change is toward the
/// 72 spp path reference of this pose. Knocking out `cache_fill` leaves the probe
/// mix, and the same frames stay within noise of each other. A fill that
/// writes the wrong values moves the picture the wrong way and fails the second
/// assertion.
#[test]
fn the_presented_frame_moves_when_the_cache_fills() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    renderer
        .apply_lighting_preset("high")
        .expect("high preset");
    renderer.reset_lighting_cache();
    let (stage, camera) = hall();
    let mut early = Vec::new();
    let mut late = Vec::new();
    for frame in 0..LATE {
        let pixels = draw(&mut *window, &mut *renderer, &stage.world, &camera);
        assert_eq!(
            pixels.len(),
            (renderer.width() * renderer.height() * 4) as usize,
            "presented image size does not match the surface"
        );
        if frame + 1 == EARLY {
            early = pixels;
        } else if frame + 1 == LATE {
            late = pixels;
        }
    }
    assert!(!early.is_empty(), "early frame");
    assert!(!late.is_empty(), "late frame");
    let delta = mean_abs_codes(&early, &late);
    let reference = reference_plane(renderer.width() as usize, renderer.height() as usize);
    let early_error = error_vs_reference(&early, &reference);
    let late_error = error_vs_reference(&late, &reference);
    eprintln!(
        "fill knockout: early-to-late mean abs {delta:.3} codes, error vs path reference early {early_error:.4} late {late_error:.4}"
    );
    assert!(
        delta >= MIN_DELTA_CODES,
        "{RED} the presented frame did not move as the cache filled (delta {delta:.3} codes, need {MIN_DELTA_CODES}). \
         cache_fill knocked out stays on the probe mix"
    );
    let gain = early_error - late_error;
    assert!(
        gain >= MIN_ERROR_GAIN,
        "{RED} the presented frame moved but not toward the path reference (error {early_error:.4} to {late_error:.4}, \
         need a fall of {MIN_ERROR_GAIN})"
    );
}
