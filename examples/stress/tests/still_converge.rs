#![cfg_attr(test, expect(clippy::as_conversions, reason = "pixel index is width * height * 4"))]
#![cfg_attr(test, expect(clippy::cast_precision_loss, reason = "a mean over the frame"))]
#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! A still hall keeps converging.
//!
//! Path-reference error at frame 2N is lower than at frame N, and the surface
//! cache is still tracing rays. A resolved cell used to take no rays for 900
//! frames, and a low-change cell past 64 sets left on a 4096-entry trickle
//! list whose overflow got zero rays, so a still picture froze.

use std::sync::Mutex;

use genos_debug::image::{pixel_luminance, Image};
use genos_render::Renderer;
use genos_scene::{Camera, Vec3};
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};
use genos_window::Window;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
/// Mid frame. Past the point where the old scheduler parks a resolved cell.
const N: usize = 192;
/// The relative luminance error must fall by at least this from N to 2N.
/// Old scheduler on this pose: 0.2121 at frame 192, 0.2118 at frame 384
/// (drop 0.0003). Measured with this scheduler: 0.2119 to 0.2108 (drop 0.0012).
const MIN_DROP: f64 = 0.0008;
/// Late cache rays. The old scheduler was under 25k here; a refining scene stays above this.
const MIN_LATE_RAYS: u32 = 100_000;
/// First late frame whose cache-ray counter must stay non-zero. By then a
/// full set count is in, which is where the old scheduler went idle.
const RAY_FROM: usize = N + N / 2;
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

/// A frozen camera, sun, and lamps. The picture at 2N is closer to the path
/// reference than the picture at N, and every late still frame traced cache rays.
#[test]
fn a_still_hall_keeps_converging() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    renderer.apply_lighting_preset("high").expect("high preset");
    renderer.reset_lighting_cache();
    let (stage, camera) = hall();
    let reference = reference_plane(renderer.width() as usize, renderer.height() as usize);
    let mut at_n = Vec::new();
    let mut at_2n = Vec::new();
    let mut rays = Vec::new();
    for frame in 0..N * 2 {
        let pixels = draw(window, renderer, &stage.world, &camera);
        let stats = renderer.settle_stats();
        if frame + 1 >= RAY_FROM {
            rays.push(stats.cache_rays);
        }
        if frame + 1 == N || frame + 1 == N * 2 || (frame + 1) % 64 == 0 {
            let err = error_vs_reference(&pixels, &reference);
            eprintln!(
                "still converge frame {} error {err:.4} cache_rays {} round_entries {}",
                frame + 1,
                stats.cache_rays,
                stats.round_entries
            );
        }
        if frame + 1 == N {
            at_n = pixels;
        } else if frame + 1 == N * 2 {
            at_2n = pixels;
        }
    }
    let error_n = error_vs_reference(&at_n, &reference);
    let error_2n = error_vs_reference(&at_2n, &reference);
    let min_rays = rays.iter().copied().min().unwrap_or(0);
    eprintln!(
        "still converge: error at {N} {error_n:.4} at {} {error_2n:.4} drop {:.4} min cache rays {min_rays}",
        N * 2,
        error_n - error_2n
    );
    assert!(
        error_2n < error_n - MIN_DROP,
        "a still hall stopped converging (error {error_n:.4} at {N} to {error_2n:.4} at {}, need a drop of {MIN_DROP})",
        N * 2
    );
    assert!(
        min_rays > MIN_LATE_RAYS,
        "a still hall stopped tracing cache rays (min {min_rays}, need {MIN_LATE_RAYS})"
    );
}
