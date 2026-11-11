#![cfg_attr(test, expect(clippy::as_conversions, reason = "pixel index is width * height * 4"))]
#![cfg_attr(test, expect(clippy::cast_precision_loss, reason = "a mean over the frame"))]
#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! Presented-frame knockout for the surface-cache fill (stage F0).
//!
//! The test reads the swapchain image. With the cache writing, the hall moves
//! off the first frames as the cache replaces the probe mix. With
//! `--features knockout` and `GENOS_KNOCKOUT=cache_fill`, those value writes
//! are frozen and this test goes red. The passes still dispatch.
//!
//! `scripts/ci-fill-knockout.sh` runs both.

use std::sync::Mutex;

use genos_render::Renderer;
use genos_scene::{Camera, Vec3};
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};
use genos_window::Window;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
/// First presented frame kept. Frame 0 is often the clear.
const EARLY: usize = 2;
/// Frames from a cleared cache. Long enough for the cache to leave the probe mix.
const LATE: usize = 24;
/// Mean absolute channel change, in 8-bit codes, from EARLY to LATE.
/// Measured on lavapipe: the fill moves the picture by more than this, and
/// `cache_fill` knocked out stays under it. See the experiment note.
const MIN_DELTA_CODES: f64 = 4.0;

fn open() -> (MutexGuard, Window, Renderer) {
    static GPU: Mutex<()> = Mutex::new(());
    let guard = GPU.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut window = Window::open_proof(WIDTH, HEIGHT).expect("proof window");
    let frame = window.pump();
    let renderer =
        Renderer::open(window.display, window.surface, frame.width, frame.height).expect("renderer");
    (guard, window, renderer)
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

/// The presented hall changes as the cache fills. Knocking out `cache_fill`
/// leaves the probe mix, and the same frames stay within noise of each other.
#[test]
fn the_presented_frame_moves_when_the_cache_fills() {
    let (_gpu, mut window, mut renderer) = open();
    renderer
        .apply_lighting_preset("high")
        .expect("high preset");
    renderer.reset_lighting_cache();
    let (stage, camera) = hall();
    let mut early = Vec::new();
    let mut late = Vec::new();
    for frame in 0..LATE {
        let pixels = draw(&mut window, &mut renderer, &stage.world, &camera);
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
    let dispatches = renderer.lighting_dispatches();
    eprintln!(
        "fill knockout: early-to-late mean abs {delta:.3} codes, dispatches {dispatches:?}"
    );
    assert!(
        delta >= MIN_DELTA_CODES,
        "the presented frame did not move as the cache filled (delta {delta:.3} codes, need {MIN_DELTA_CODES}). \
         cache_fill knocked out stays on the probe mix"
    );
}
