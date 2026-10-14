//! Stress hall on high, full picture, against the hash wavelet.
//!
//! The 320×180 bounce check stays inside a cell of a few pixels, so a wavelet
//! that steps in pixels still crosses several cells and matches. At this size
//! the foreground 12.5 cm cell is wider than that pixel kernel. Eight warmup
//! frames, then the scored pair: mean and p99 against the hash path stay
//! inside the picture bar, still and moving, and the screen path's own
//! frame-to-frame change stays with the hash path.

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

fn assert_bar(name: &str, hash: &[u8], screen: &[u8]) {
    let (mean, p99) = mean_p99(hash, screen);
    eprintln!("hall {name} mean {mean:.3} p99 {p99:.3}");
    assert!(mean <= 1.5, "{name} mean {mean:.3} is past the picture bar 1.5");
    assert!(p99 <= 10.0, "{name} p99 {p99:.3} is past the picture bar 10");
}

fn assert_screen_launch(name: &str, stats: &LightingAtrousStats) {
    eprintln!(
        "hall {name} screen pixels {} coarse {} filtered {} ran {}",
        stats.screen_pixels, stats.coarse_pixels, stats.filtered, stats.ran
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
#[test]
fn screen_hall_matches_the_hash_wavelet_still_and_moving() {
    let (hash_still, hash_stats) = hall_frames(false, true, 8, 2);
    let (screen_still, screen_stats) = hall_frames(true, true, 8, 2);
    assert_eq!(hash_stats.screen_pixels, 0, "hash path counts no screen pixels");
    assert_screen_launch("still", &screen_stats);
    assert_bar("still", &hash_still[1], &screen_still[1]);
    let (hash_flicker, hash_flicker_p99) = mean_p99(&hash_still[0], &hash_still[1]);
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

    let (hash_move, _) = hall_frames(false, false, 3, 3);
    let (screen_move, move_stats) = hall_frames(true, false, 3, 3);
    assert_screen_launch("moving", &move_stats);
    let last = hash_move.len() - 1;
    assert_bar("moving", &hash_move[last], &screen_move[last]);
    let (hash_step, hash_step_p99) = mean_p99(&hash_move[last - 1], &hash_move[last]);
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
