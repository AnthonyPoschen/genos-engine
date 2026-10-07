//! Device timestamps for one real draw. Pixel checks stay in the UI graph test.

use std::time::Duration;

use genos_render::{Renderer, World};
use genos_scene::{Camera, Floor, Scene, Vec3};
use genos_window::Window;

fn open() -> (std::sync::MutexGuard<'static, ()>, Window, Renderer) {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = GPU.lock().unwrap_or_else(|err| err.into_inner());
    let mut window = Window::open_proof(1280, 720).expect("proof window");
    let frame = window.pump();
    let renderer = Renderer::open(window.display, window.surface, frame.width, frame.height)
        .expect("renderer");
    (guard, window, renderer)
}

#[test]
fn a_profiled_draw_reports_device_time_for_the_draw() {
    let (_gpu, mut window, mut renderer) = open();
    let world = World::from_scene(Scene {
        floor: Floor {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 8.0,
            half_z: 8.0,
            color: [1.0, 1.0, 1.0],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights: Vec::new(),
        ceiling: None,
        sky: None,
    });
    let camera = Camera::opening();
    let _ = window.pump();
    let (pixels, profile) = renderer
        .draw_profiled(&world, &camera, &[], true, false)
        .expect("profiled draw");
    let pixels = pixels.expect("readback");
    assert_eq!(
        pixels.len(),
        (renderer.width() * renderer.height() * 4) as usize
    );
    let gpu = profile.gpu.expect("timestamp span");
    assert!(
        profile.cpu > Duration::ZERO,
        "draw cpu time was {:?}",
        profile.cpu
    );
    assert!(gpu > Duration::ZERO, "draw gpu time was {gpu:?}");
    assert!(
        profile.cpu > gpu,
        "gpu time followed the cpu clock: cpu {:?} gpu {:?}",
        profile.cpu,
        gpu
    );
    let again = renderer.finish_gpu_times().expect("finish");
    assert!(
        again.is_empty(),
        "the draw already returned its gpu time: {again:?}"
    );

    let _ = renderer.draw(&world, &camera, false).expect("plain draw");
    let plain = renderer.finish_gpu_times().expect("plain finish");
    assert!(
        plain.is_empty(),
        "a draw without the profiler stored a gpu time: {plain:?}"
    );
}
