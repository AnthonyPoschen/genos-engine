//! Small sun step on lavapipe. The shader bit is the proof. Times are not.

use genos_render::{Renderer, World};
use genos_scene::{Camera, Light, Scene, Vec3};

fn open() -> (std::sync::MutexGuard<'static, ()>, genos_window::Window, Renderer) {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = GPU.lock().unwrap_or_else(|err| err.into_inner());
    let mut window = genos_window::Window::open_proof(320, 180).expect("proof window");
    let frame = window.pump();
    let renderer = Renderer::open(window.display, window.surface, frame.width, frame.height)
        .expect("renderer");
    (guard, window, renderer)
}

fn hall() -> World {
    let scene: Scene = genos_scene::load_path(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/camera/scene.rhai"
    )))
    .expect("shipped scene");
    World::from_scene(scene)
}

fn sun_at(yaw: f32) -> Light {
    Light {
        position: Vec3::new(0.0, 7.0, 0.0),
        color: [1.0, 0.96, 0.9],
        direction: Vec3::new(yaw.sin(), -0.85, yaw.cos()),
    }
}

fn draw(window: &mut genos_window::Window, renderer: &mut Renderer, world: &World, camera: &Camera) {
    let _ = window.pump();
    renderer.draw(world, camera, false).expect("draw");
}

fn lighting_names(renderer: &Renderer) -> Vec<String> {
    renderer
        .pipeline_times()
        .into_iter()
        .map(|(name, _)| name)
        .filter(|name| name.starts_with("lighting"))
        .collect()
}

#[test]
fn a_small_sun_step_sets_the_shader_bit() {
    let (_gpu, mut window, mut renderer) = open();
    let mut world = hall();
    world.scene.lights.push(sun_at(0.0));
    let camera = Camera::opening();
    let names = lighting_names(&renderer);
    assert!(
        !names.iter().any(|name| name == "lighting sun compose"),
        "startup does not build the sun compose: {names:?}"
    );
    assert_eq!(
        names.len(),
        10,
        "nine compute pipelines plus the gbuffer: {names:?}"
    );

    for _ in 0..3 {
        draw(&mut window, &mut renderer, &world, &camera);
    }
    let frozen = renderer.lighting_sun_stats();
    assert!(!frozen.gpu_sun, "a frozen sun stays off the sun path: {frozen:?}");
    assert_eq!(frozen.host_frames, 0);
    assert_eq!(lighting_names(&renderer).len(), 10);

    // Full effort with no sun frames yet still writes the counter. The still
    // and moving benches never enter the path, so this is the proof the flag
    // changed the compose.
    std::env::set_var("GENOS_LIGHTING_FULL_EFFORT", "1");
    world.scene.lights.last_mut().unwrap().direction = sun_at(0.2_f32.to_radians()).direction;
    draw(&mut window, &mut renderer, &world, &camera);
    let early = renderer.lighting_sun_stats();
    assert!(!early.gpu_sun, "full effort stays off before any sun frame: {early:?}");
    assert_eq!(early.host_frames, 0);
    assert!(early.full_effort);
    assert_eq!(early.compose, 1, "picture compose, not the sun band: {early:?}");
    assert_eq!(early.dispatches, 1);
    let early_text = std::fs::read_to_string("genos-debug/sun/counter.txt").expect("sun counter");
    assert!(early_text.contains("full_effort 1\n"), "{early_text}");
    assert!(early_text.contains("compose full\n"), "{early_text}");
    assert!(early_text.contains("host_frames 0\n"), "{early_text}");
    std::env::remove_var("GENOS_LIGHTING_FULL_EFFORT");

    let yaw = 0.4_f32.to_radians();
    world.scene.lights.last_mut().unwrap().direction = sun_at(yaw).direction;
    let mut slices = Vec::new();
    for _ in 0..4 {
        draw(&mut window, &mut renderer, &world, &camera);
        let stats = renderer.lighting_sun_stats();
        assert!(stats.gpu_sun, "the shader bit is set: {stats:?}");
        slices.push(stats.slice);
    }
    let held = renderer.lighting_sun_stats();
    assert!(held.host_frames >= 4, "host counted the dispatches: {held:?}");
    assert!(
        slices.iter().collect::<std::collections::BTreeSet<_>>().len() > 1,
        "bands move: {slices:?}"
    );
    assert!(
        lighting_names(&renderer).iter().any(|name| name == "lighting sun compose"),
        "the sun compose pipeline is built on the first small step"
    );
    let counter = std::fs::read_to_string("genos-debug/sun/counter.txt").expect("sun counter");
    assert!(counter.contains("sun 1\n"), "{counter}");
    assert!(counter.contains("full_effort 0\n"), "{counter}");
    assert!(counter.contains("compose sun\n"), "{counter}");
    assert!(counter.contains("dispatches 1\n"), "{counter}");

    let host_before = renderer.lighting_sun_stats().host_frames;
    std::env::set_var("GENOS_LIGHTING_FULL_EFFORT", "1");
    world.scene.lights.last_mut().unwrap().direction = sun_at(yaw + 0.2_f32.to_radians()).direction;
    draw(&mut window, &mut renderer, &world, &camera);
    let full = renderer.lighting_sun_stats();
    assert!(!full.gpu_sun, "full effort leaves the sun path: {full:?}");
    assert!(full.full_effort, "the flag is on: {full:?}");
    assert_eq!(full.compose, 1, "full effort takes the picture compose: {full:?}");
    assert_eq!(full.dispatches, 1, "one compose dispatch: {full:?}");
    assert_eq!(full.host_frames, host_before);
    let forced = std::fs::read_to_string("genos-debug/sun/counter.txt").expect("sun counter");
    assert!(forced.contains("full_effort 1\n"), "{forced}");
    assert!(forced.contains("compose full\n"), "{forced}");
    assert!(forced.contains("sun 0\n"), "{forced}");
    std::env::remove_var("GENOS_LIGHTING_FULL_EFFORT");

    let point = world.scene.lights[0].position;
    world.scene.lights[0].position = Vec3::new(point.x + 0.4, point.y, point.z);
    draw(&mut window, &mut renderer, &world, &camera);
    world.scene.lights[0].position = point;
    draw(&mut window, &mut renderer, &world, &camera);
    let mut catch = Vec::new();
    for _ in 0..6 {
        draw(&mut window, &mut renderer, &world, &camera);
        catch.push(renderer.lighting_sun_stats().gpu_catch);
    }
    assert_eq!(
        catch,
        vec![true, true, true, true, false, false],
        "catch-up is four held frames, not the halfway wait"
    );

    std::env::set_var("GENOS_LIGHTING_CATCH_UP", "0");
    world.scene.lights[0].position = Vec3::new(point.x + 0.4, point.y, point.z);
    draw(&mut window, &mut renderer, &world, &camera);
    world.scene.lights[0].position = point;
    draw(&mut window, &mut renderer, &world, &camera);
    draw(&mut window, &mut renderer, &world, &camera);
    let off = renderer.lighting_sun_stats();
    assert!(!off.gpu_catch, "catch-up 0 stays off: {off:?}");
    std::env::remove_var("GENOS_LIGHTING_CATCH_UP");
}
