#![cfg_attr(
    test,
    expect(
        clippy::as_conversions,
        reason = "presented image length is width * height * 4"
    )
)]
#![cfg_attr(
    test,
    expect(clippy::expect_used, reason = "a test aborts when a fixture fails")
)]

//! Picture gates must read the presented frame, and must go red when their
//! production guard is knocked out (`scripts/ci-gate-knockout.sh`).
//!
//! These two are the compose halves of `the_bounce_adds_light_to_the_direct_view`
//! in `lighting_frame.rs`.

use genos_render::{DebugView, Renderer, ViewMode, World, BOUNCE_IN_COMPOSE, DIRECT_IN_COMPOSE};
use genos_scene::{Camera, Scene};
use genos_window::Window;

struct PresentedFrame {
    pixels: Vec<u8>,
}

impl PresentedFrame {
    fn from_draw(
        window: &mut Window,
        renderer: &mut Renderer,
        world: &World,
        camera: &Camera,
    ) -> Self {
        let _ = window.pump();
        let pixels = renderer
            .draw(world, camera, true)
            .expect("draw")
            .expect("readback: a lighting/render gate must read the presented image");
        assert!(!pixels.is_empty(), "presented image was empty");
        let width = renderer.width();
        let height = renderer.height();
        assert_eq!(
            pixels.len(),
            (width * height * 4) as usize,
            "presented image size does not match the surface"
        );
        let _ = renderer.gpu_kind();
        Self { pixels }
    }

    fn sum_bytes(&self) -> u64 {
        self.pixels.iter().map(|&b| u64::from(b)).sum()
    }
}

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
    for _ in 0..3 {
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
