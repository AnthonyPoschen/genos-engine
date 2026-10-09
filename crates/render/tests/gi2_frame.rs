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
    assert_eq!(gi2.len(), 9, "every GI v2 pipeline is timed: {times:?}");
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

/// Switching to GI v2 frees the current lighting's probe fields; switching back
/// rebuilds them and the current picture comes back.
#[test]
fn switching_back_from_gi_v2_restores_the_current_picture() {
    let (_gpu, mut window, mut renderer) = open();
    let world = shipped_hall();
    let camera = Camera::opening();
    let settle = |window: &mut Window, renderer: &mut Renderer| {
        let mut last = Vec::new();
        for _ in 0..40 {
            last = draw(window, renderer, &world, &camera);
        }
        last
    };
    let before = settle(&mut window, &mut renderer);
    renderer.set_gi_v2(true).expect("GI v2 on");
    let _ = settle(&mut window, &mut renderer);
    renderer.set_gi_v2(false).expect("GI v2 off");
    let after = settle(&mut window, &mut renderer);
    let sum: u64 = before
        .iter()
        .zip(&after)
        .map(|(a, b)| u64::from(a.abs_diff(*b)))
        .sum();
    let mean = sum as f64 / before.len() as f64;
    assert!(
        mean < 2.0,
        "the current picture changed by {mean:.2} of 255 after a GI v2 round trip"
    );
}

/// A dark room but for one emissive glTF cube: GI v2 draws the cube's own light,
/// and rays that hit it carry that light onto the floor in front of it.
#[test]
fn gi_v2_draws_emitted_light_and_bounces_it() {
    let (_gpu, mut window, mut renderer) = open();
    let gltf = genos_load::load_gltf_file(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../load/fixtures/cube.gltf"
    ))
    .expect("cube fixture");
    let settings = genos_bake::BakeSettings {
        mode: genos_bake::BakeMode::OnLoad,
        cache_dir: std::env::temp_dir().join("genos-gi2-emission-test"),
        ..genos_bake::BakeSettings::default()
    };
    let dark_room = |glow: f32| {
        let mut world = World::from_scene(Scene {
            floor: genos_scene::Floor {
                position: genos_scene::Vec3::ZERO,
                half_x: 20.0,
                half_z: 20.0,
                color: [0.8; 3],
            },
            walls: Vec::new(),
            solids: Vec::new(),
            lights: Vec::new(),
            ceiling: None,
            sky: None,
        });
        let mut lit = gltf.clone();
        for m in &mut lit.materials {
            m.emissive = [glow; 3];
            m.emissive_strength = 1.0;
        }
        // The fixture's cube spans x 8..12, y -2..2 at z 0: move it to stand on
        // the floor 6 m ahead of the eye.
        let mut place = genos_render::identity_pose();
        place[12] = -10.0;
        place[13] = 2.0;
        place[14] = -6.0;
        world
            .add_gltf_traced(&lit, &place, &settings)
            .expect("traced cube");
        world
    };
    let mut camera = Camera::opening();
    camera.set_pose(genos_scene::Vec3::new(0.0, 1.7, 4.0), 0.0, 0.0);
    renderer.set_gi_v2(true).expect("GI v2");
    let mut shot = |world: &World| {
        let mut last = Vec::new();
        for _ in 0..60 {
            last = draw(&mut window, &mut renderer, world, &camera);
        }
        last
    };
    let dark = shot(&dark_room(0.0));
    let glowing = shot(&dark_room(4.0));
    let (w, h) = (320usize, 180usize);
    assert_eq!(dark.len(), w * h * 4);
    let mean = |p: &[u8], rows: std::ops::Range<usize>, cols: std::ops::Range<usize>| {
        let mut sum = 0u64;
        let mut n = 0u64;
        for y in rows {
            for x in cols.clone() {
                let i = (y * w + x) * 4;
                sum += u64::from(p[i]) + u64::from(p[i + 1]) + u64::from(p[i + 2]);
                n += 3;
            }
        }
        sum as f64 / n as f64
    };
    // The cube face fills the middle; the floor in front of it the bottom rows.
    let face = (
        mean(&dark, 70..100, 140..180),
        mean(&glowing, 70..100, 140..180),
    );
    let floor = (
        mean(&dark, 135..180, 100..220),
        mean(&glowing, 135..180, 100..220),
    );
    eprintln!("face {face:?} floor {floor:?}");
    assert!(
        face.0 < 10.0 && face.1 > 150.0,
        "the cube shows its own light: {face:?}"
    );
    assert!(
        floor.1 > floor.0 + 10.0,
        "the floor catches the cube's light: {floor:?}"
    );
}

/// The screen probes average up to 32 frames of rays while nothing changes
/// (gi2_gather.comp). Any change (a lamp, an object, the camera) must restart that
/// average on the very next frame, so the picture never lags behind the scene: the
/// still count is 0 on the first frame after the change, the probes keep only that
/// frame's rays (weight 1 / (0 + 1)), and the picture already differs from the one
/// before.
#[test]
fn gi_v2_still_averaging_restarts_on_any_change() {
    let (_gpu, mut window, mut renderer) = open();
    renderer.set_gi_v2(true).expect("GI v2");
    let base: Scene = shipped_hall().scene;
    assert!(!base.lights.is_empty() && !base.solids.is_empty());
    let camera = Camera::opening();
    let mut moved_camera = Camera::opening();
    moved_camera.set_pose(genos_scene::Vec3::new(0.3, 1.7, 0.0), 0.2, 0.0);
    let mut relit = base.clone();
    relit.lights[0].color = [
        relit.lights[0].color[0] * 0.25,
        relit.lights[0].color[1] * 0.25,
        relit.lights[0].color[2] * 2.0,
    ];
    let mut moved_object = base.clone();
    moved_object.solids[0].position.x += 0.5;
    let changes: [(&str, Scene, &Camera); 3] = [
        ("a lamp's colour", relit, &camera),
        ("an object's place", moved_object, &camera),
        ("the camera", base.clone(), &moved_camera),
    ];
    let still_world = World::from_scene(base.clone());
    for (what, scene, cam) in changes {
        let mut before = Vec::new();
        for _ in 0..40 {
            before = draw(&mut window, &mut renderer, &still_world, &camera);
        }
        let held = renderer.gi_v2_still_frames();
        assert!(
            held >= 32,
            "{what}: the probes average while still ({held} frames)"
        );
        let changed = World::from_scene(scene);
        let after = draw(&mut window, &mut renderer, &changed, cam);
        assert_eq!(
            renderer.gi_v2_still_frames(),
            0,
            "{what}: the first frame after the change starts the average over"
        );
        let diff = before
            .iter()
            .zip(&after)
            .filter(|(a, b)| a.abs_diff(**b) > 2)
            .count();
        assert!(
            diff > 0,
            "{what}: the first frame after the change shows it"
        );
        let _ = draw(&mut window, &mut renderer, &changed, cam);
        assert_eq!(
            renderer.gi_v2_still_frames(),
            1,
            "{what}: and counts on from there while it holds"
        );
    }
}
