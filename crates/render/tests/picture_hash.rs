//! Still-hall picture hash. Printed once so a later change can be checked
//! against this tree's bytes. Not a permanent gate by itself.

use genos_render::{Renderer, World};
use genos_scene::Camera;
use genos_window::Window;

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
    let world = World::from_scene(scene);
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
