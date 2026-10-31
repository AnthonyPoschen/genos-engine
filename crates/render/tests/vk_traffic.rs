//! Steady-state Vulkan traffic stays flat. NVIDIA frees one-time command
//! recordings and `vkCmdUpdateBuffer` copies inside `vkDestroyDevice`, so a
//! count that grows with the frame count is the quit stall.

#![expect(clippy::expect_used, reason = "a failed fixture aborts the test")]

use genos_render::{grade_camera, grade_world, Renderer};
use genos_math::optics::ReflectionPreset;
use genos_window::Window;

#[test]
fn steady_frames_allocate_no_vulkan_objects() {
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
    for _ in 0..3 {
        let _ = window.pump();
        renderer.draw(&world, &camera, false).expect("warmup draw");
    }
    let after_3 = renderer.vk_traffic();
    for _ in 0..297 {
        let _ = window.pump();
        renderer.draw(&world, &camera, false).expect("steady draw");
    }
    let after_300 = renderer.vk_traffic();
    let delta = after_300.since(&after_3);
    eprintln!("vk_traffic_after_3 {}", after_3.line());
    eprintln!("vk_traffic_after_300 {}", after_300.line());
    eprintln!("vk_traffic_delta_3_to_300 {}", delta.line());
    assert!(
        delta.is_flat(),
        "frames 3..300 grew driver traffic {}",
        delta.line()
    );

    renderer
        .apply_lighting_preset("high")
        .expect("high preset");
    for _ in 0..8 {
        let _ = window.pump();
        renderer.draw(&world, &camera, false).expect("high warmup");
    }
    let high_start = renderer.vk_traffic();
    for _ in 0..16 {
        let _ = window.pump();
        renderer.draw(&world, &camera, false).expect("high steady");
    }
    let high_end = renderer.vk_traffic();
    let high = high_end.since(&high_start);
    eprintln!("vk_traffic_high_delta {}", high.line());
    assert!(
        high.is_flat(),
        "high preset steady frames grew driver traffic {}",
        high.line()
    );
}
