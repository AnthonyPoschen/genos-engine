//! GI v2's frame path (gi2_gpu.rs): its pipelines build quickly, and its direct light
//! matches the current frame's direct light.

use std::time::Duration;

use genos_render::{DebugView, Renderer, ViewMode, World};
use genos_scene::{Camera, Scene};
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

fn shipped_hall() -> World {
    let scene: Scene = genos_scene::load_path(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/camera/scene.rhai"
    )))
    .expect("shipped scene");
    World::from_scene(scene)
}

/// Longest a GI v2 pipeline may take to build. The SDF march inlined into the current
/// shaders took more than four minutes on the 4070's compiler; each GI v2 pass has one
/// trace site and builds in well under a second there. Lavapipe is slower, so the
/// bound is loose; `GENOS_PIPELINE_LIMIT` (seconds) tightens it on a real GPU.
fn pipeline_limit() -> Duration {
    let seconds = std::env::var("GENOS_PIPELINE_LIMIT")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(10.0);
    Duration::from_secs_f64(seconds)
}

#[test]
fn gi_v2_pipelines_build_quickly() {
    let (_gpu, _window, mut renderer) = open();
    renderer.set_gi_v2(true).expect("GI v2");
    let times = renderer.pipeline_times();
    let gi2: Vec<_> = times
        .iter()
        .filter(|(name, _)| name.starts_with("gi2"))
        .collect();
    assert_eq!(gi2.len(), 8, "every GI v2 pipeline is timed: {times:?}");
    let limit = pipeline_limit();
    for (name, took) in &times {
        eprintln!("pipeline {name}: {:.3} s", took.as_secs_f64());
    }
    for (name, took) in gi2 {
        assert!(
            *took < limit,
            "{name} took {took:?} to build (limit {limit:?})"
        );
    }
}

fn draw(window: &mut Window, renderer: &mut Renderer, world: &World, camera: &Camera) -> Vec<u8> {
    let _ = window.pump();
    renderer
        .draw(world, camera, true)
        .expect("draw")
        .expect("readback")
}

#[test]
fn gi_v2_direct_light_matches_the_current_direct_light() {
    let (_gpu, mut window, mut renderer) = open();
    let world = shipped_hall();
    let camera = Camera::opening();
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Direct,
        ..DebugView::default()
    });
    let current = draw(&mut window, &mut renderer, &world, &camera);
    renderer.set_gi_v2(true).expect("GI v2");
    let v2 = draw(&mut window, &mut renderer, &world, &camera);
    assert_eq!(current.len(), v2.len());
    let mut sum = 0u64;
    let mut far = 0usize;
    for (a, b) in current.iter().zip(&v2) {
        let d = a.abs_diff(*b);
        sum += u64::from(d);
        if d > 8 {
            far += 1;
        }
    }
    let mean = sum as f64 / current.len() as f64;
    let far_share = far as f64 / current.len() as f64;
    assert!(mean < 1.0, "mean difference {mean:.3} of 255");
    assert!(
        far_share < 0.005,
        "{:.2}% of channels differ by more than 8",
        far_share * 100.0
    );

    // The full picture has light the direct view does not: the probes' bounce.
    renderer.set_debug_view(DebugView::default());
    let full = draw(&mut window, &mut renderer, &world, &camera);
    let lit = |p: &[u8]| p.iter().map(|&c| u64::from(c)).sum::<u64>();
    assert!(lit(&full) > lit(&v2), "GI v2's bounce adds light");
}
