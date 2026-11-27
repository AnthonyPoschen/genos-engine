#![cfg_attr(
    test,
    expect(clippy::expect_used, reason = "a test aborts when a fixture fails")
)]

//! Picture gates must read the presented frame, and must go red when their
//! production guard is knocked out (`scripts/ci-gate-knockout.sh`).
//!
//! These two are the compose halves of `the_bounce_adds_light_to_the_direct_view`
//! in `lighting_frame.rs`.

mod common;

use common::PresentedFrame;
use genos_render::{DebugView, Renderer, ViewMode, World, BOUNCE_IN_COMPOSE, DIRECT_IN_COMPOSE};
use genos_scene::{Camera, Scene};
use genos_window::Window;

fn open() -> (
    std::sync::MutexGuard<'static, ()>,
    genos_window::Lifecycle<Renderer>,
) {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = GPU
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut window = Window::open_proof(320, 180).expect("proof window");
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height)
            .expect("renderer");
    (
        guard,
        genos_window::Lifecycle::new(window, renderer, permit),
    )
}

fn shipped_hall() -> World {
    let scene: Scene = genos_scene::load_path(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/camera/scene.rhai"
    )))
    .expect("shipped scene");
    World::from_scene(scene)
}

/// Frames drawn after a cache reset before a capture is graded.
///
/// Level 0 traces one of four direction sets per frame (`lighting_frame() % 4`)
/// and upper cells refresh on `(key + frame) % 4`. After three frames one set
/// was still empty, and which one depended on the frame the reset landed on:
/// two bounce-only captures differed by up to 0.71% on lavapipe, against the
/// 1% the knocked-out direct gate must stay under. Sixteen frames, four turns
/// of both schedules, kept four captures at any start phase within 0.10%.
const SETTLE_FRAMES: usize = 16;

fn settled(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
    view: DebugView,
) -> PresentedFrame {
    renderer.set_debug_view(view);
    renderer.reset_lighting_cache();
    let mut frame = None;
    for _ in 0..SETTLE_FRAMES {
        frame = Some(PresentedFrame::from_draw(window, renderer, world, camera));
    }
    frame.expect("settled frame")
}

fn adds_at_least_one_percent(full: u64, part: u64) -> bool {
    full > part && (full - part) * 100 > part
}

/// Direct lamp and sun light is in the presented frame. This fails when
/// `direct_in_compose` is knocked out (compose is bounce only).
#[test]
fn the_direct_light_is_in_the_presented_frame() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let world = shipped_hall();
    let camera = Camera::opening();
    let bounce = settled(
        window,
        renderer,
        &world,
        &camera,
        DebugView {
            mode: ViewMode::Bounce,
            ..DebugView::default()
        },
    );
    let full = settled(window, renderer, &world, &camera, DebugView::default());
    eprintln!(
        "direct gate: full={} bounce={}",
        full.sum_bytes(),
        bounce.sum_bytes()
    );
    assert!(
        adds_at_least_one_percent(full.sum_bytes(), bounce.sum_bytes()),
        "direct light did not add to the presented frame (guard {DIRECT_IN_COMPOSE}): full {} bounce {}",
        full.sum_bytes(),
        bounce.sum_bytes()
    );
}

/// The existing bounce-in-the-picture gate (`the_bounce_adds_light_to_the_direct_view`).
///
/// The presented full frame must be brighter than the direct view. This fails
/// when `bounce_in_compose` is knocked out.
#[test]
fn the_bounce_adds_light_to_the_presented_frame() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let world = shipped_hall();
    let camera = Camera::opening();
    let direct = settled(
        window,
        renderer,
        &world,
        &camera,
        DebugView {
            mode: ViewMode::Direct,
            ..DebugView::default()
        },
    );
    let full = settled(window, renderer, &world, &camera, DebugView::default());
    eprintln!(
        "bounce gate: full={} direct={}",
        full.sum_bytes(),
        direct.sum_bytes()
    );
    assert!(
        adds_at_least_one_percent(full.sum_bytes(), direct.sum_bytes()),
        "the bounce did not add light to the presented frame (guard {BOUNCE_IN_COMPOSE}): full {} direct {}",
        full.sum_bytes(),
        direct.sum_bytes()
    );
}
