#![cfg_attr(
    test,
    expect(
        clippy::as_conversions,
        reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md"
    )
)]
#![cfg_attr(
    test,
    expect(clippy::cast_precision_loss, reason = "a mean over bytes")
)]
#![cfg_attr(
    test,
    expect(clippy::expect_used, reason = "a test aborts when a fixture fails")
)]

//! A readback is BGRA8 whatever the swapchain holds. On an HDR10 or scRGB
//! surface the presented words are PQ or fp16, and reading them as bytes
//! drew rainbow contour rings in every `shot()`.

use genos_render::{Renderer, World};
use genos_scene::{Camera, HdrSettings, LightingPreset, OutputMode};
use genos_window::Window;
use std::path::Path;

fn hall(output: OutputMode) -> World {
    let scene = genos_scene::load_path(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/camera/scene.rhai"
    )))
    .expect("hall scene");
    let mut world = World::from_scene(scene);
    let mut hdr = HdrSettings::for_preset(LightingPreset::High);
    hdr.exposure_mode = genos_scene::ExposureMode::Fixed;
    hdr.display = genos_scene::DisplayTransform::Agx;
    hdr.bloom = false;
    hdr.output = output;
    hdr.legacy_bytes = false;
    world.scene.hdr = hdr;
    world
}

fn picture(window: &mut Window, renderer: &mut Renderer, world: &World) -> Vec<u8> {
    let camera = Camera::opening();
    let mut last = Vec::new();
    for _ in 0..3 {
        let _ = window.pump();
        last = renderer
            .draw(world, &camera, true)
            .expect("draw")
            .expect("readback");
    }
    last
}

#[test]
fn an_hdr_readback_matches_the_sdr_picture() {
    let mut window = Window::open_proof(320, 180).expect("proof window");
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height)
            .expect("renderer");
    let mut owner = genos_window::Lifecycle::new(window, renderer, permit);
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let sdr = picture(window, renderer, &hall(OutputMode::Sdr));
    let hdr = picture(window, renderer, &hall(OutputMode::Auto));
    let (format, space) = renderer.hdr_surface();
    assert_eq!(sdr.len(), hdr.len());
    let mut sum = 0u64;
    let mut worst = 0u8;
    for (a, b) in sdr.iter().zip(&hdr) {
        let d = a.abs_diff(*b);
        sum += u64::from(d);
        worst = worst.max(d);
    }
    let mean = sum as f64 / sdr.len().max(1) as f64;
    println!("surface format {format} space {space}: mean byte gap {mean:.3}, worst {worst}");
    // Dither and a lit-light settle differ by a code or two; PQ read as bytes is tens.
    assert!(
        mean < 2.0,
        "HDR readback is not the SDR picture: mean gap {mean:.3}"
    );
}
