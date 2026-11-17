//! A resize replaces the swapchain instead of adding a second one.
//!
//! One `wl_surface` carries one `wp_fifo`. A driver that binds fifo-v1 per
//! swapchain loses the surface ("Surface already has a fifo",
//! `VK_ERROR_SURFACE_LOST_KHR`) when the old swapchain is still live at the
//! next create. Lavapipe does not bind fifo-v1, so the renderer refuses the
//! second create itself and this test fails on lavapipe too.

#![expect(clippy::expect_used, reason = "a failed fixture aborts the test")]

use genos_math::optics::ReflectionPreset;
use genos_render::{grade_camera, grade_world, Renderer};
use genos_window::Window;

#[test]
fn a_resize_replaces_the_swapchain_and_keeps_presenting() {
    let mut window = Window::open_proof(320, 180).expect("proof window");
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height)
            .expect("renderer");
    let mut owner = genos_window::Lifecycle::new(window, renderer, permit);
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let world = grade_world(ReflectionPreset::Potato);
    let camera = grade_camera();
    for (width, height) in [(320, 180), (256, 144), (320, 180)] {
        renderer.resize(width, height).expect("resize recreates the swapchain");
        for _ in 0..2 {
            let _ = window.pump();
            renderer
                .draw(&world, &camera, false)
                .expect("draw after the resize");
        }
        assert_eq!((renderer.width(), renderer.height()), (width, height));
    }
    assert!(
        genos_window::ShutdownDevice::frames_presented(renderer) >= 6,
        "presents stopped after a resize"
    );
}
