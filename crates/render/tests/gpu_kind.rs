#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! The renderer reports which device drew the frame, so a gate can refuse to
//! grade a software device as if it were the RTX 4070.

use genos_render::{GpuKind, Renderer};
use genos_window::Window;

#[test]
#[expect(clippy::disallowed_methods, reason = "the test reads which ICD the harness picked")]
fn the_renderer_names_its_device_kind() {
    let mut window = Window::open_proof(64, 64).expect("proof window");
    let frame = window.pump();
    let (renderer, permit) = Renderer::open(window.display, window.surface, frame.width, frame.height)
        .expect("renderer");
    let owner = genos_window::Lifecycle::new(window, renderer, permit);
    let renderer = &*owner.device;
    let lavapipe = std::env::var("VK_ICD_FILENAMES").is_ok_and(|v| v.contains("lvp"));
    if lavapipe {
        assert_eq!(renderer.gpu_kind(), GpuKind::Software);
    }
    eprintln!("gpu_kind {:?}", renderer.gpu_kind());
}

#[test]
fn a_cpu_device_type_is_the_software_split() {
    assert_eq!(GpuKind::from_vk_device_type(4), GpuKind::Software);
    assert_eq!(GpuKind::from_vk_device_type(2), GpuKind::Hardware);
    assert!(GpuKind::Software.post_ms_passes(1.689, 1.50));
    assert!(!GpuKind::Hardware.post_ms_passes(1.689, 1.50));
    assert!(!GpuKind::Software.post_ms_passes(0.0, 1.50));
}
