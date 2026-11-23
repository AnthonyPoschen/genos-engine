#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(
    test,
    expect(clippy::as_conversions, reason = "pixel index is width * height * 4")
)]
#![cfg_attr(
    test,
    expect(
        clippy::cast_precision_loss,
        reason = "solid pixel counts fit the f64 mantissa for proof frames"
    )
)]

//! Geometry coverage: the stress building must not be sky where mesh sits.
//!
//! Catches the Instance SSBO stride class of failure (sky wedges through walls)
//! that lavapipe may hide when OOB reads return zero. Run with
//! `GENOS_BUFFER_POISON=1` in CI so host-visible buffers start as NaN/garbage.

use std::sync::Mutex;

use genos_render::{DebugView, Renderer, ViewMode};
use genos_scene::{Camera, Vec3};
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};
use genos_window::Window;

fn open() -> (
    std::sync::MutexGuard<'static, ()>,
    genos_window::Lifecycle<Renderer>,
) {
    static GPU: Mutex<()> = Mutex::new(());
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

fn stage() -> Stage {
    let mut stage = Stage::new(
        Scale::SMALL,
        1,
        LightMix {
            count: 5,
            dynamic_pct: 50,
            layout: Layout::Spread,
        },
        1.0,
        0.12,
        120.0,
    );
    stage.sun_frozen = true;
    stage.apply();
    stage
}

fn aim(eye: [f32; 3], target: [f32; 3]) -> Camera {
    let d = [
        target[0] - eye[0],
        target[1] - eye[1],
        target[2] - eye[2],
    ];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1.0e-6);
    let pitch = (d[1] / len).clamp(-1.0, 1.0).asin();
    let yaw = d[0].atan2(-d[2]);
    let mut camera = Camera::new(eye[0], eye[2], yaw);
    camera.set_pose(Vec3::new(eye[0], eye[1], eye[2]), yaw, pitch);
    camera
}

fn draw_albedo(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &genos_render::World,
    camera: &Camera,
) -> Vec<u8> {
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Albedo,
        lamp: 0,
    });
    let _ = window.pump();
    renderer
        .draw(world, camera, true)
        .expect("draw")
        .expect("presented albedo")
}

/// Fraction of pixels that are not near-black (sky / miss / clear).
fn solid_fraction(pixels: &[u8]) -> f64 {
    let n = pixels.len() / 4;
    assert!(n > 0, "empty picture");
    let mut solid = 0usize;
    for px in pixels.as_chunks::<4>().0 {
        let sum = u16::from(px[0]) + u16::from(px[1]) + u16::from(px[2]);
        let luma = f32::from(sum) / (3.0 * 255.0);
        if luma > 0.04 {
            solid += 1;
        }
    }
    solid as f64 / n as f64
}

#[test]
fn static_geometry_has_coverage_across_poses_with_movers() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let mut stage = stage();
    // Advance movers so instance buffers are rewritten under motion.
    for _ in 0..8 {
        stage.advance(1.0 / 30.0);
    }
    let world = &stage.world;
    let poses = [
        ("hall", [14.5, 1.7, 23.5], [12.0, 1.2, 18.0]),
        ("doorway", [12.0, 1.6, 20.5], [22.0, 1.2, 20.5]),
        ("corner", [3.5, 1.6, 3.5], [0.2, 0.4, 0.2]),
        ("outside", [12.5, 1.7, 34.0], [12.5, 1.5, 22.0]),
    ];
    for (name, eye, target) in poses {
        let camera = aim(eye, target);
        let mut frame = Vec::new();
        for _ in 0..3 {
            frame = draw_albedo(window, renderer, world, &camera);
        }
        let frac = solid_fraction(&frame);
        eprintln!("{name}: solid albedo fraction {frac:.3}");
        // Indoor poses are mostly walls/floor. A stride desync leaves large
        // sky wedges (fraction collapses toward the clear). Outside still
        // shows the facade.
        let min = if name == "outside" { 0.08 } else { 0.35 };
        assert!(
            frac >= min,
            "{name}: solid fraction {frac:.3} < {min} — sky where geometry should be \
             (Instance SSBO stride desync)"
        );
    }
}
