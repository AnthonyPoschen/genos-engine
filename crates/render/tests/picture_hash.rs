#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(test, expect(clippy::items_after_statements, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::redundant_closure_for_method_calls, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]

//! Still-hall picture hash. Printed once so a later change can be checked
//! against this tree's bytes. Not a permanent gate by itself.

use genos_render::{Renderer, World};
use genos_scene::Camera;
use genos_window::Window;

/// Picture checks hold the work-cap controller off. Play leaves it on.
fn present(mut scene: genos_scene::Scene) -> World {
    scene.lighting.controller = false;
    World::from_scene(scene)
}

fn open() -> (std::sync::MutexGuard<'static, ()>, Window, Renderer) {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = GPU.lock().unwrap_or_else(|err| err.into_inner());
    let mut window = Window::open_proof(320, 180).expect("proof window");
    let frame = window.pump();
    let renderer = Renderer::open(window.display, window.surface, frame.width, frame.height)
        .expect("renderer");
    (guard, window, renderer)
}

#[test]
fn print_still_hall_hash() {
    let (_gpu, mut window, mut renderer) = open();
    let scene = genos_scene::load_path(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/camera/scene.rhai"
    )))
    .expect("scene");
    let world = present(scene);
    let camera = Camera::opening();
    let mut last = Vec::new();
    for _ in 0..8 {
        let _ = window.pump();
        last = renderer.draw(&world, &camera, true).expect("draw").expect("pixels");
    }
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    last.hash(&mut hasher);
    eprintln!(
        "STILL_HALL hash {:016x} bytes {} w {} h {}",
        hasher.finish(),
        last.len(),
        renderer.width(),
        renderer.height()
    );
}
