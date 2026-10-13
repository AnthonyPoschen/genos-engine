//! Each lighting preset, and each feature turned off from high, draws a few frames.
//!
//! Validation layers are not wired in this renderer (`enabled_layers` stays 0).
//! `draw` returning `Ok` is the error check. A disabled pass must record no
//! dispatches and 0 GPU time. Timestamps come from `GENOS_GPU_TIMES`; a device
//! with no timestamp period fails this test.

use genos_render::{Renderer, World};
use genos_scene::{Camera, Floor, Light, LightingPreset, LightingSettings, Scene, Vec3};
use genos_window::Window;

fn open() -> (std::sync::MutexGuard<'static, ()>, Window, Renderer) {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = GPU.lock().unwrap_or_else(|err| err.into_inner());
    std::env::set_var("GENOS_GPU_TIMES", "30");
    let mut window = Window::open_proof(320, 180).expect("proof window");
    let frame = window.pump();
    let renderer = Renderer::open(window.display, window.surface, frame.width, frame.height)
        .expect("renderer");
    (guard, window, renderer)
}

fn room(settings: LightingSettings) -> World {
    World::from_scene(Scene {
        floor: Floor {
            position: Vec3::ZERO,
            half_x: 8.0,
            half_z: 8.0,
            color: [0.6, 0.6, 0.6],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights: vec![Light {
            position: Vec3::new(0.0, 3.0, 0.0),
            color: [1.0, 0.95, 0.85],
            direction: Vec3::new(0.2, -1.0, 0.1),
        }],
        ceiling: None,
        sky: None,
        lighting: settings,
    })
}

fn index(name: &str) -> usize {
    Renderer::lighting_bucket_names()
        .iter()
        .position(|bucket| *bucket == name)
        .unwrap_or_else(|| panic!("unknown bucket {name}"))
}

fn draw(window: &mut Window, renderer: &mut Renderer, world: &World, camera: &mut Camera) {
    // A moving camera keeps the frame from settling, so a cache round stays due.
    camera.position.x += 0.05;
    if camera.position.x > 1.5 {
        camera.position.x = -1.5;
    }
    let _ = window.pump();
    renderer.draw(world, camera, false).expect("draw");
}

/// Draw several frames. Disabled buckets stay at 0 dispatches and 0 ms.
/// Enabled buckets dispatch at least once. Returns whether any pass recorded time.
fn run_case(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &mut Camera,
    name: &str,
    run: &[&str],
    zero: &[&str],
) -> bool {
    let mut saw = [0u32; 12];
    let mut timed = false;
    let mut samples = 0u32;
    // The collected counters are a slot behind, and the first waits still
    // belong to the previous case. Skip those, then require the new frames.
    for frame in 0..12 {
        draw(window, renderer, world, camera);
        if frame < 4 {
            continue;
        }
        let counts = renderer.lighting_dispatches();
        let ms = renderer.lighting_pass_ms();
        if counts.iter().all(|count| *count == 0) {
            continue;
        }
        samples += 1;
        for (i, count) in counts.iter().enumerate() {
            saw[i] = saw[i].max(*count);
        }
        if ms.iter().any(|value| *value > 0.0) {
            timed = true;
        }
        for bucket in zero {
            let i = index(bucket);
            assert_eq!(counts[i], 0, "{name}: {bucket} dispatched {}", counts[i]);
            assert_eq!(ms[i], 0.0, "{name}: {bucket} recorded {} ms", ms[i]);
        }
    }
    assert!(samples >= 2, "{name}: no collected frame");
    for bucket in run {
        let i = index(bucket);
        assert!(saw[i] > 0, "{name}: {bucket} never dispatched");
    }
    timed
}

#[test]
fn presets_and_feature_off_skip_their_passes() {
    let (_gpu, mut window, mut renderer) = open();
    let mut camera = Camera::opening();
    let gi = ["compact", "place", "trace", "gather", "filter", "cache", "atrous", "cache_rays", "hit_light"];
    let picture = ["direct", "shadows", "compose"];

    let mut timed = false;
    let cases = [
        (LightingPreset::Potato.name(), LightingSettings::preset(LightingPreset::Potato), picture.as_slice(), gi.as_slice()),
        (
            LightingPreset::Low.name(),
            LightingSettings::preset(LightingPreset::Low),
            ["direct", "shadows", "compose", "place", "trace", "gather", "filter"].as_slice(),
            ["atrous"].as_slice(),
        ),
        (
            LightingPreset::Medium.name(),
            LightingSettings::preset(LightingPreset::Medium),
            ["direct", "shadows", "compose", "place", "trace", "gather", "filter", "cache", "cache_rays", "atrous", "hit_light"].as_slice(),
            [].as_slice(),
        ),
        (
            LightingPreset::High.name(),
            LightingSettings::preset(LightingPreset::High),
            ["direct", "shadows", "compose", "compact", "place", "trace", "gather", "filter", "cache", "atrous", "cache_rays", "hit_light"].as_slice(),
            [].as_slice(),
        ),
    ];
    for (name, settings, run, zero) in cases {
        let world = room(settings);
        timed |= run_case(&mut window, &mut renderer, &world, &mut camera, name, run, zero);
    }

    let mut off = LightingSettings::preset(LightingPreset::High);
    off.direct.enabled = false;
    let world = room(off);
    timed |= run_case(&mut window, &mut renderer, &world, &mut camera, "direct off", &["compose", "place", "hit_light"], &["direct"]);

    let mut off = LightingSettings::preset(LightingPreset::High);
    off.shadows.enabled = false;
    let world = room(off);
    timed |= run_case(&mut window, &mut renderer, &world, &mut camera, "shadows off", &["direct", "compose"], &["shadows"]);

    let mut off = LightingSettings::preset(LightingPreset::High);
    off.gi.enabled = false;
    let world = room(off);
    timed |= run_case(&mut window, &mut renderer, &world, &mut camera, "gi off", picture.as_slice(), gi.as_slice());

    let mut off = LightingSettings::preset(LightingPreset::High);
    off.bounce.count = 0;
    let world = room(off);
    timed |= run_case(
        &mut window,
        &mut renderer,
        &world,
        &mut camera,
        "bounces 0",
        &["direct", "compose", "place", "cache"],
        &["cache_rays", "atrous"],
    );

    let mut off = LightingSettings::preset(LightingPreset::High);
    off.probes.enabled = false;
    let world = room(off);
    timed |= run_case(
        &mut window,
        &mut renderer,
        &world,
        &mut camera,
        "probes off",
        &["direct", "compose", "cache"],
        &["place", "trace", "gather", "filter"],
    );

    let mut off = LightingSettings::preset(LightingPreset::High);
    off.world_cache.enabled = false;
    let world = room(off);
    timed |= run_case(
        &mut window,
        &mut renderer,
        &world,
        &mut camera,
        "world cache off",
        &["direct", "compose", "place", "cache_rays"],
        &["atrous"],
    );

    let mut off = LightingSettings::preset(LightingPreset::High);
    off.denoiser.enabled = false;
    let world = room(off);
    timed |= run_case(&mut window, &mut renderer, &world, &mut camera, "denoise off", &["direct", "compose", "cache"], &["atrous"]);

    for (name, edit) in [
        ("compose reuse off", &(|s: &mut LightingSettings| s.compose_reuse = false) as &dyn Fn(&mut LightingSettings)),
        ("sky off", &(|s: &mut LightingSettings| s.sky = false)),
        ("emissive off", &(|s: &mut LightingSettings| s.emissive = false)),
        ("prefill off", &(|s: &mut LightingSettings| s.prefill = false)),
    ] {
        let mut settings = LightingSettings::preset(LightingPreset::High);
        edit(&mut settings);
        let world = room(settings.clone());
        timed |= run_case(&mut window, &mut renderer, &world, &mut camera, name, &["direct", "compose", "cache"], &[]);
        assert_eq!(renderer.lighting_settings().compose_reuse, settings.compose_reuse, "{name}");
        assert_eq!(renderer.lighting_settings().sky, settings.sky, "{name}");
        assert_eq!(renderer.lighting_settings().emissive, settings.emissive, "{name}");
        assert_eq!(renderer.lighting_settings().prefill, settings.prefill, "{name}");
    }

    assert!(timed, "GPU timestamps did not record; GENOS_GPU_TIMES needs a device timestamp period");
}
