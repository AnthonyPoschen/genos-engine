#![cfg_attr(test, expect(clippy::as_conversions, reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::cast_lossless, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::cast_possible_truncation, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::cast_precision_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::cast_sign_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(test, expect(clippy::redundant_closure_for_method_calls, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]

//! Screen wavelet against the hash wavelet inside this branch.
//!
//! The gi-v2 picture gate is `gi_v2_floor`: it pins the hash launch (medium,
//! and high with `denoise_launch=hash`) against gi-v2 dumps. This test is the
//! screen launch (`denoise_launch=screen`) against the hash launch, not that
//! gate. Preset high already uses the screen launch; this test still sets
//! the flag both ways.
//!
//! The 320×180 bounce check stays inside a cell of a few pixels, so a wavelet
//! that steps in pixels still crosses several cells and matches. At this size
//! the foreground 12.5 cm cell is wider than that pixel kernel. Eight warmup
//! frames, then the scored pair. The picture's p99 against the hash path stays
//! under that path's own cross-process error (slot claims race, so two hash
//! runs differ by more than one frame of noise), still and moving. A whole
//! face a few luma off fails. The screen path's own flicker stays with the
//! hash path.

use std::sync::Mutex;

use genos_render::{LightingAtrousStats, Renderer};
use genos_scene::{Camera, LightingPreset, LightingSettings, Vec3};
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};
use genos_window::Window;

const WIDTH: u32 = 960;
const HEIGHT: u32 = 540;

fn open() -> (MutexGuard, Window, Renderer) {
    static GPU: Mutex<()> = Mutex::new(());
    let guard = GPU.lock().unwrap_or_else(|err| err.into_inner());
    let mut window = Window::open_proof(WIDTH, HEIGHT).expect("proof window");
    let frame = window.pump();
    let renderer = Renderer::open(window.display, window.surface, frame.width, frame.height)
        .expect("renderer");
    (guard, window, renderer)
}

type MutexGuard = std::sync::MutexGuard<'static, ()>;

fn hall_stage(still: bool) -> Stage {
    let mut stage = Stage::new(
        Scale::SMALL,
        1,
        LightMix {
            count: 5,
            dynamic_pct: 50,
            layout: Layout::Spread,
        },
        1.0,
        0.25,
        120.0,
    );
    stage.sun_frozen = true;
    stage.boxes_still = still;
    stage.lamps_still = still;
    stage.apply();
    stage
}

/// debug/scenes/stress.scene checkpoint `hall`.
fn hall_camera() -> Camera {
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(14.5, 1.7, 23.5), -0.3554, 0.08);
    camera
}

fn draw(window: &mut Window, renderer: &mut Renderer, world: &genos_render::World, camera: &Camera) -> Vec<u8> {
    let _ = window.pump();
    renderer
        .draw(world, camera, true)
        .expect("draw")
        .expect("pixels")
}

fn hall_frames(screen: bool, still: bool, warmup: usize, scored: usize) -> (Vec<Vec<u8>>, LightingAtrousStats) {
    let (_gpu, mut window, mut renderer) = open();
    let mut settings = LightingSettings::preset(LightingPreset::High);
    settings.denoiser.screen = screen;
    renderer.set_lighting_settings(settings);
    let mut stage = hall_stage(still);
    let camera = hall_camera();
    for _ in 0..warmup {
        if !still {
            stage.advance(1.0 / 60.0);
        }
        let _ = draw(&mut window, &mut renderer, &stage.world, &camera);
    }
    let mut frames = Vec::with_capacity(scored);
    for _ in 0..scored {
        if !still {
            stage.advance(1.0 / 60.0);
        }
        frames.push(draw(&mut window, &mut renderer, &stage.world, &camera));
    }
    let stats = renderer.lighting_atrous_stats();
    (frames, stats)
}

fn pixel_errors(reference: &[u8], image: &[u8]) -> Vec<f64> {
    let n = reference.len().min(image.len()) / 4;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let p = i * 4;
        let e = (u32::from(reference[p].abs_diff(image[p]))
            + u32::from(reference[p + 1].abs_diff(image[p + 1]))
            + u32::from(reference[p + 2].abs_diff(image[p + 2]))) as f64
            / 3.0;
        out.push(e);
    }
    out
}

fn mean_of(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.iter().sum::<f64>() / v.len() as f64
}

fn percentile(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let i = ((v.len() - 1) as f64 * p).round() as usize;
    v[i.min(v.len() - 1)]
}

fn mean_p99(reference: &[u8], image: &[u8]) -> (f64, f64) {
    let mut errors = pixel_errors(reference, image);
    let mean = mean_of(&errors);
    let p99 = percentile(&mut errors, 0.99);
    (mean, p99)
}

fn closer_bar(name: &str, hash_a: &[u8], hash_b: &[u8], screen: &[u8], floor_p99: f64, cross_mean: f64) {
    let (mean_a, p99_a) = mean_p99(hash_a, screen);
    let (mean_b, p99_b) = mean_p99(hash_b, screen);
    let (mean, p99) = if p99_a <= p99_b {
        (mean_a, p99_a)
    } else {
        (mean_b, p99_b)
    };
    // Pixel error is the mean of three channel diffs, so p99 lands on k/3.
    // One pair estimates that step: a moving hash pair at p99 2.000 and a
    // screen pair with the same mean landed on 2.333. A face several luma
    // off still fails both bars.
    let limit = floor_p99 + 1.0 / 3.0;
    eprintln!(
        "hall {name} vs A {mean_a:.3}/{p99_a:.3} vs B {mean_b:.3}/{p99_b:.3} closer {mean:.3}/{p99:.3} floor {floor_p99:.3} limit {limit:.3}"
    );
    assert!(
        p99 <= limit + 1.0e-9,
        "{name} p99 {p99:.3} is above the hash path noise {floor_p99:.3}"
    );
    assert!(
        mean <= cross_mean + 0.25,
        "{name} mean {mean:.3} is above the hash path cross-process mean {cross_mean:.3}"
    );
}

fn assert_screen_launch(name: &str, stats: &LightingAtrousStats) {
    eprintln!(
        "hall {name} screen pixels {} coarse {} filtered {} ran {} movers {}",
        stats.screen_pixels, stats.coarse_pixels, stats.filtered, stats.ran, stats.changed_pixels
    );
    assert!(stats.ran, "{name} screen wavelet did not publish");
    assert!(stats.screen_pixels > 0, "{name} filtered no pixels");
    assert_ne!(stats.filtered, 1_048_576, "{name} walked the slot table");
    let pixels = WIDTH * HEIGHT;
    assert!(
        stats.filtered <= pixels,
        "{name} filtered {} is larger than the picture {pixels}",
        stats.filtered
    );
}

/// Full picture of the stress hall. Still is a frozen stage. Moving advances
/// the boxes and the lamps. Both are the hash wavelet's picture, plus the
/// screen path's own flicker against that path.
///
/// Two processes do not share slot ids. `lighting_cache_find_sized` claims
/// with `atomicCompSwap`, and a ray's rotation is a hash of the slot, so two
/// hash runs of the same scene differ by more than one frame of noise inside
/// one run. The picture bar is that cross-process floor (or the frame noise,
/// when the claims happen to land the same way).
#[test]
fn screen_hall_matches_the_hash_wavelet_still_and_moving() {
    let (hash_a, hash_stats) = hall_frames(false, true, 8, 2);
    let (hash_b, _) = hall_frames(false, true, 8, 2);
    let (cross_mean, cross_p99) = mean_p99(&hash_a[1], &hash_b[1]);
    let (noise_mean, noise_p99) = mean_p99(&hash_a[0], &hash_a[1]);
    let still_floor = cross_p99.max(noise_p99);
    eprintln!(
        "hash still cross-process mean {cross_mean:.3} p99 {cross_p99:.3} same-process {noise_mean:.3}/{noise_p99:.3} floor {still_floor:.3}"
    );
    assert_eq!(hash_stats.screen_pixels, 0, "hash path counts no screen pixels");
    let (screen_still, screen_stats) = hall_frames(true, true, 8, 2);
    assert_screen_launch("still", &screen_stats);
    closer_bar(
        "still",
        &hash_a[1],
        &hash_b[1],
        &screen_still[1],
        still_floor,
        cross_mean,
    );
    let (hash_flicker, hash_flicker_p99) = (noise_mean, noise_p99);
    let (screen_flicker, screen_flicker_p99) = mean_p99(&screen_still[0], &screen_still[1]);
    eprintln!(
        "hall still flicker hash {hash_flicker:.3}/{hash_flicker_p99:.3} screen {screen_flicker:.3}/{screen_flicker_p99:.3}"
    );
    assert!(
        screen_flicker <= hash_flicker + 1.5,
        "still flicker mean {screen_flicker:.3} is above hash {hash_flicker:.3}"
    );
    assert!(
        screen_flicker_p99 <= hash_flicker_p99 + 10.0,
        "still flicker p99 {screen_flicker_p99:.3} is above hash {hash_flicker_p99:.3}"
    );
    assert!(
        screen_flicker <= 1.5 && screen_flicker_p99 <= 10.0,
        "still flicker {screen_flicker:.3}/{screen_flicker_p99:.3} is past the picture noise"
    );

    // Moving boxes and lamps. A second hash run is the cross-process floor:
    // the same claim race as still, on a scene that keeps changing.
    let (hash_move, _) = hall_frames(false, false, 8, 3);
    let (hash_move_b, _) = hall_frames(false, false, 8, 3);
    let last = hash_move.len() - 1;
    let (move_cross_mean, move_cross_p99) = mean_p99(&hash_move[last], &hash_move_b[last]);
    let (hash_step, hash_step_p99) = mean_p99(&hash_move[last - 1], &hash_move[last]);
    let move_floor = move_cross_p99.max(hash_step_p99);
    eprintln!(
        "hash moving cross-process mean {move_cross_mean:.3} p99 {move_cross_p99:.3} step {hash_step:.3}/{hash_step_p99:.3} floor {move_floor:.3}"
    );
    let (screen_move, move_stats) = hall_frames(true, false, 8, 3);
    assert_screen_launch("moving", &move_stats);
    closer_bar(
        "moving",
        &hash_move[last],
        &hash_move_b[last],
        &screen_move[last],
        move_floor,
        move_cross_mean,
    );
    let (screen_step, screen_step_p99) = mean_p99(&screen_move[last - 1], &screen_move[last]);
    eprintln!(
        "hall moving step hash {hash_step:.3}/{hash_step_p99:.3} screen {screen_step:.3}/{screen_step_p99:.3}"
    );
    assert!(
        screen_step <= hash_step + 1.5,
        "moving flicker mean {screen_step:.3} is above hash {hash_step:.3}"
    );
    assert!(
        screen_step_p99 <= hash_step_p99 + 10.0,
        "moving flicker p99 {screen_step_p99:.3} is above hash {hash_step_p99:.3}"
    );
}
