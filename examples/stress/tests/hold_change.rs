#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! A still camera must not keep a stored bounce across a lamp or a mover.
//!
//! The 32-frame pass is the drift under one percent. A change the host already
//! hashed (`still == 0`) publishes that frame, and the texels it reaches walk.
//! The open fill walks every frame until that first pass, so it does not arrive
//! as one step.

use std::sync::Mutex;

use genos_render::Renderer;
use genos_scene::{Camera, LightingPreset, LightingSettings, Vec3};
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};
use genos_window::Window;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;

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

fn mean_codes(px: &[u8]) -> f64 {
    let n = px.len() / 4;
    if n == 0 {
        return 0.0;
    }
    let mut sum = 0.0f64;
    for i in 0..n {
        let p = i * 4;
        sum += f64::from(px[p]);
        sum += f64::from(px[p + 1]);
        sum += f64::from(px[p + 2]);
    }
    sum / (3.0 * n as f64)
}

fn max_step(means: &[f64]) -> f64 {
    means
        .windows(2)
        .map(|w| (w[0] - w[1]).abs())
        .fold(0.0, f64::max)
}

struct Drawn {
    mean: f64,
    walks: u32,
}

fn draw_one(window: &mut genos_window::Window, renderer: &mut Renderer, stage: &Stage, camera: &Camera) -> Drawn {
    let _ = window.pump();
    let pixels = renderer.draw(&stage.world, camera, true).expect("draw").expect("pixels");
    let stats = renderer.lighting_atrous_stats();
    Drawn {
        mean: mean_codes(&pixels),
        walks: stats.cache_walks,
    }
}

/// Past the fill and the first drift pass, so a later change is not the open.
const SETTLED: usize = 48;

fn settle(window: &mut genos_window::Window, renderer: &mut Renderer, stage: &Stage, camera: &Camera) -> Vec<f64> {
    let mut means = Vec::with_capacity(SETTLED);
    for _ in 0..SETTLED {
        means.push(draw_one(window, renderer, stage, camera).mean);
    }
    means
}

#[test]
fn an_untouched_still_view_does_not_pop_on_the_first_refresh() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    renderer.set_lighting_settings(LightingSettings::preset(LightingPreset::High));
    let (stage, camera) = hall();
    let means = settle(window, renderer, &stage, &camera);
    // Draws 1..31 are the old freeze. Draw 31 is frame 32, the old pop
    // (0.522 to 0.903 linear, 68 codes). The fill now moves with the cache.
    let step = max_step(&means[1..]);
    eprintln!("open fill means {means:?} max step {step:.2}");
    assert!(
        step < 30.0,
        "the open fill stepped {step:.2} codes; the held bounce used to jump about 68"
    );
}

#[test]
fn a_lamp_toggle_on_a_still_camera_updates_the_bounce_without_a_pop() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    renderer.set_lighting_settings(LightingSettings::preset(LightingPreset::High));
    let (mut stage, camera) = hall();
    let open_means = settle(window, renderer, &stage, &camera);
    let before = *open_means.last().expect("settle");
    stage.power = 0.0;
    stage.apply();
    let mut after = Vec::new();
    for _ in 0..40 {
        after.push(draw_one(window, renderer, &stage, &camera));
    }
    let means: Vec<f64> = after.iter().map(|d| d.mean).collect();
    eprintln!(
        "lamp toggle before {before:.2} walks {:?} means {means:?}",
        after.iter().map(|d| d.walks).collect::<Vec<_>>()
    );
    assert!(
        after[0].walks > 1000,
        "the toggle frame reused the stored bounce (walks {})",
        after[0].walks
    );
    let tail = means[24..].iter().sum::<f64>() / 16.0;
    let err_before = (before - tail).abs();
    let err_two = (means[2] - tail).abs();
    assert!(
        err_two < err_before * 0.5,
        "bounce had not landed by frame 2: before {before:.2} frame2 {err_two:.2} from tail {tail:.2}, start was {err_before:.2} away"
    );
    let late = max_step(&means[3..]);
    assert!(
        late < 15.0,
        "the bounce popped after the toggle (max step {late:.2} codes, the old drop was 75)"
    );
}

#[test]
fn a_box_slide_on_a_still_camera_unholds_the_bounce() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    renderer.set_lighting_settings(LightingSettings::preset(LightingPreset::High));
    let (mut stage, camera) = hall();
    let _ = settle(window, renderer, &stage, &camera);
    stage.boxes_still = false;
    let mut walks = Vec::new();
    let mut means = Vec::new();
    for _ in 0..8 {
        stage.advance(0.4);
        let drawn = draw_one(window, renderer, &stage, &camera);
        walks.push(drawn.walks);
        means.push(drawn.mean);
    }
    eprintln!("box slide walks {walks:?} means {means:?}");
    assert!(
        walks.iter().all(|w| *w > 100),
        "a moving box left the bounce held (walks {walks:?})"
    );
    stage.boxes_still = true;
    stage.apply();
    let mut stopped = Vec::new();
    for _ in 0..36 {
        stopped.push(draw_one(window, renderer, &stage, &camera).mean);
    }
    let late = max_step(&stopped);
    eprintln!("box stopped max step {late:.2}");
    assert!(
        late < 20.0,
        "the bounce popped after the box stopped (max step {late:.2} codes)"
    );
}

#[test]
fn a_slow_lamp_ramp_on_a_still_camera_does_not_wait_for_the_refresh() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    renderer.set_lighting_settings(LightingSettings::preset(LightingPreset::High));
    let (mut stage, camera) = hall();
    let open_means = settle(window, renderer, &stage, &camera);
    let start = *open_means.last().expect("settle");
    // 0.4 % a frame for 12 frames is about 4.9 % total. Each step is under
    // the one-percent test. Committing the tracking word first never marks.
    let mut ramp = vec![start];
    for _ in 0..12 {
        stage.power *= 1.004;
        stage.apply();
        ramp.push(draw_one(window, renderer, &stage, &camera).mean);
    }
    let end = *ramp.last().expect("ramp");
    let moved = (end - start).abs();
    let mut held = Vec::new();
    for _ in 0..36 {
        held.push(draw_one(window, renderer, &stage, &camera).mean);
    }
    let late = max_step(&held);
    eprintln!("drift ramp {start:.2} -> {end:.2} (move {moved:.2}) late step {late:.2} tail {held:?}");
    // A settled day hall sits within a few thousandths of a code. The ramp is
    // 0.4 % a frame. A frozen tracking word leaves that mean where it was.
    assert!(
        moved > 0.5,
        "the ramp did not reach the picture (move {moved:.2} codes)"
    );
    assert!(
        late < moved,
        "the ramp waited for the refresh (late step {late:.2} against a ramp of {moved:.2})"
    );
}
