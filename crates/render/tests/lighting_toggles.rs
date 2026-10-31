#![cfg_attr(test, expect(clippy::as_conversions, reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::cast_lossless, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::cast_possible_truncation, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::cast_possible_wrap, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::cast_precision_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::cast_sign_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::chunks_exact_to_as_chunks, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::disallowed_methods, reason = "TODO(lint-2026-10-14): publish intent with the CLI argument, not set_var. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(test, expect(clippy::panic, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(test, expect(clippy::redundant_closure_for_method_calls, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::too_many_lines, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::uninlined_format_args, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]

//! Each lighting preset, and each feature turned off from high, draws a few frames.
//!
//! Validation layers are not wired in this renderer (`enabled_layers` stays 0).
//! `draw` returning `Ok` is the error check. A disabled pass must record no
//! dispatches and 0 GPU time. Timestamps come from `GENOS_GPU_TIMES`; a device
//! with no timestamp period fails this test.

use genos_render::{Renderer, World};
use genos_scene::{
    viewport_uv, AmbientMode, Camera, Ceiling, Floor, Light, LightingPreset, LightingSettings,
    Scene, Shape, Sky, Solid, Vec3, Wall,
};
use genos_window::Window;

fn open() -> (std::sync::MutexGuard<'static, ()>, genos_window::Lifecycle<Renderer>) {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = GPU.lock().unwrap_or_else(|err| err.into_inner());
    std::env::set_var("GENOS_GPU_TIMES", "30");
    let mut window = Window::open_proof(320, 180).expect("proof window");
    let frame = window.pump();
    let (renderer, permit) = Renderer::open(window.display, window.surface, frame.width, frame.height)
        .expect("renderer");
    (guard, genos_window::Lifecycle::new(window, renderer, permit))
}

/// Picture checks hold the work-cap controller off. Play leaves it on.
fn present(mut scene: Scene) -> World {
    scene.lighting.controller = false;
    World::from_scene(scene)
}

fn room(settings: LightingSettings) -> World {
    present(Scene {
        floor: Floor {
            position: Vec3::ZERO,
            half_x: 8.0,
            half_z: 8.0,
            color: [0.6, 0.6, 0.6],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights: vec![
            Light {
                position: Vec3::new(0.0, 3.0, 0.0),
                color: [1.0, 0.95, 0.85],
                direction: Vec3::new(0.2, -1.0, 0.1),
            },
            // A point lamp. The dispatch test moves it. A camera move is not
            // a change record, so without this the probe-only presets settle
            // and stop placing probes.
            Light {
                position: Vec3::new(0.0, 2.5, 1.0),
                color: [0.4, 0.4, 0.35],
                direction: Vec3::ZERO,
            },
        ],
        ceiling: None,
        sky: None,
        lighting: settings,
        debug: genos_scene::DebugSettings::default(),
        fps_counter: genos_scene::FpsCounterSettings::default(),
        event_loop: genos_scene::EventLoopSettings::default(),
        reflections: genos_scene::ReflectionSettings::default(),
        hdr: genos_scene::HdrSettings::legacy_bytes(),
        fog: genos_scene::FogSettings::default(),
        menu: genos_scene::MenuSettings::default(),
    })
}

fn index(name: &str) -> usize {
    Renderer::lighting_bucket_names()
        .iter()
        .position(|bucket| *bucket == name)
        .unwrap_or_else(|| panic!("unknown bucket {name}"))
}

fn draw(window: &mut Window, renderer: &mut Renderer, world: &mut World, camera: &mut Camera) {
    // A camera move is not a change record. The point lamp is. While it
    // moves, stillness stays 0 and a probe-only preset keeps placing probes.
    camera.position.x += 0.05;
    if camera.position.x > 1.5 {
        camera.position.x = -1.5;
    }
    if let Some(lamp) = world.scene.lights.iter_mut().find(|lamp| {
        let d = lamp.direction;
        d.x * d.x + d.y * d.y + d.z * d.z < 1.0e-8
    }) {
        lamp.position.x += 0.05;
        if lamp.position.x > 1.0 {
            lamp.position.x = -1.0;
        }
    }
    let _ = window.pump();
    renderer.draw(world, camera, false).expect("draw");
}

/// Draw several frames. Disabled buckets stay at 0 dispatches and 0 ms.
/// Enabled buckets dispatch at least once. Returns whether any pass recorded time.
fn run_case(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &mut World,
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
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let mut camera = Camera::opening();
    let gi = [
        "compact",
        "place",
        "trace",
        "gather",
        "filter",
        "cache",
        "atrous",
        "cache_rays",
        "hit_light",
    ];
    let picture = ["direct", "shadows", "compose"];

    let mut timed = false;
    let cases = [
        (
            LightingPreset::Potato.name(),
            LightingSettings::preset(LightingPreset::Potato),
            picture.as_slice(),
            gi.as_slice(),
        ),
        (
            LightingPreset::DayCycle.name(),
            LightingSettings::preset(LightingPreset::DayCycle),
            picture.as_slice(),
            gi.as_slice(),
        ),
        (
            LightingPreset::Gi1Bounce.name(),
            LightingSettings::preset(LightingPreset::Gi1Bounce),
            [
                "direct", "shadows", "compose", "place", "trace", "gather", "filter",
            ]
            .as_slice(),
            ["compact", "cache", "cache_rays", "atrous", "hit_light"].as_slice(),
        ),
        (
            LightingPreset::Low.name(),
            LightingSettings::preset(LightingPreset::Low),
            [
                "direct", "shadows", "compose", "place", "trace", "gather", "filter",
            ]
            .as_slice(),
            ["atrous"].as_slice(),
        ),
        (
            LightingPreset::Medium.name(),
            LightingSettings::preset(LightingPreset::Medium),
            [
                "direct",
                "shadows",
                "compose",
                "place",
                "trace",
                "gather",
                "filter",
                "cache",
                "cache_rays",
                "atrous",
                "hit_light",
            ]
            .as_slice(),
            [].as_slice(),
        ),
        (
            LightingPreset::High.name(),
            LightingSettings::preset(LightingPreset::High),
            [
                "direct",
                "shadows",
                "compose",
                "compact",
                "place",
                "trace",
                "gather",
                "filter",
                "cache",
                "atrous",
                "cache_rays",
                "hit_light",
            ]
            .as_slice(),
            [].as_slice(),
        ),
    ];
    for (name, settings, run, zero) in cases {
        let mut world = room(settings);
        timed |= run_case(
            window,
            renderer,
            &mut world,
            &mut camera,
            name,
            run,
            zero,
        );
    }

    let mut off = LightingSettings::preset(LightingPreset::High);
    off.direct.enabled = false;
    let mut world = room(off);
    timed |= run_case(
        window,
        renderer,
        &mut world,
        &mut camera,
        "direct off",
        &["compose", "place", "hit_light"],
        &["direct"],
    );

    let mut off = LightingSettings::preset(LightingPreset::High);
    off.shadows.enabled = false;
    let mut world = room(off);
    timed |= run_case(
        window,
        renderer,
        &mut world,
        &mut camera,
        "shadows off",
        &["direct", "compose"],
        &["shadows"],
    );

    let mut off = LightingSettings::preset(LightingPreset::High);
    off.gi.enabled = false;
    let mut world = room(off);
    timed |= run_case(
        window,
        renderer,
        &mut world,
        &mut camera,
        "gi off",
        picture.as_slice(),
        gi.as_slice(),
    );

    let mut off = LightingSettings::preset(LightingPreset::High);
    off.bounce.count = 0;
    let mut world = room(off);
    timed |= run_case(
        window,
        renderer,
        &mut world,
        &mut camera,
        "bounces 0",
        picture.as_slice(),
        gi.as_slice(),
    );

    let mut one = LightingSettings::preset(LightingPreset::High);
    one.bounce.count = 1;
    let mut world = room(one);
    timed |= run_case(
        window,
        renderer,
        &mut world,
        &mut camera,
        "bounces 1",
        &["direct", "shadows", "compose", "place", "trace", "gather", "filter"],
        &["compact", "cache", "cache_rays", "atrous", "hit_light"],
    );

    let mut two = LightingSettings::preset(LightingPreset::High);
    two.bounce.count = 2;
    let mut world = room(two);
    timed |= run_case(
        window,
        renderer,
        &mut world,
        &mut camera,
        "bounces 2",
        &[
            "direct",
            "shadows",
            "compose",
            "compact",
            "place",
            "trace",
            "gather",
            "filter",
            "cache",
            "cache_rays",
            "atrous",
            "hit_light",
        ],
        &[],
    );

    let mut off = LightingSettings::preset(LightingPreset::High);
    off.probes.enabled = false;
    let mut world = room(off);
    timed |= run_case(
        window,
        renderer,
        &mut world,
        &mut camera,
        "probes off",
        &["direct", "compose", "cache"],
        &["place", "trace", "gather", "filter"],
    );

    let mut off = LightingSettings::preset(LightingPreset::High);
    off.world_cache.enabled = false;
    let mut world = room(off);
    timed |= run_case(
        window,
        renderer,
        &mut world,
        &mut camera,
        "world cache off",
        &["direct", "compose", "place", "cache_rays"],
        &["atrous"],
    );

    let mut off = LightingSettings::preset(LightingPreset::High);
    off.denoiser.enabled = false;
    let mut world = room(off);
    timed |= run_case(
        window,
        renderer,
        &mut world,
        &mut camera,
        "denoise off",
        &["direct", "compose", "cache"],
        &["atrous"],
    );

    for (name, edit) in [
        (
            "compose reuse off",
            &(|s: &mut LightingSettings| s.compose_reuse = false) as &dyn Fn(&mut LightingSettings),
        ),
        ("sky off", &(|s: &mut LightingSettings| s.sky = false)),
        (
            "emissive off",
            &(|s: &mut LightingSettings| s.emissive = false),
        ),
        (
            "prefill off",
            &(|s: &mut LightingSettings| s.prefill = false),
        ),
    ] {
        let mut settings = LightingSettings::preset(LightingPreset::High);
        edit(&mut settings);
        let mut world = room(settings.clone());
        timed |= run_case(
            window,
            renderer,
            &mut world,
            &mut camera,
            name,
            &["direct", "compose", "cache"],
            &[],
        );
        assert_eq!(
            renderer.lighting_settings().compose_reuse,
            settings.compose_reuse,
            "{name}"
        );
        assert_eq!(renderer.lighting_settings().sky, settings.sky, "{name}");
        assert_eq!(
            renderer.lighting_settings().emissive,
            settings.emissive,
            "{name}"
        );
        assert_eq!(
            renderer.lighting_settings().prefill,
            settings.prefill,
            "{name}"
        );
    }

    assert!(
        timed,
        "GPU timestamps did not record; GENOS_GPU_TIMES needs a device timestamp period"
    );
}

fn wall(x: f32, z: f32, half_x: f32, half_z: f32) -> Wall {
    Wall {
        position: Vec3::new(x, 0.0, z),
        half_x,
        half_z,
        height: 3.0,
        base: 0.0,
        color: [0.62, 0.62, 0.64],
        absorption: 0.0,
        reflectance: -1.0,
        color_mix: -1.0,
    }
}

fn box_at(x: f32, z: f32, color: [f32; 3]) -> Solid {
    Solid {
        shape: Shape::Square,
        position: Vec3::new(x, 0.0, z),
        size: 1.0,
        height: 1.2,
        yaw: 0.0,
        color,
        absorption: 0.0,
        reflectance: -1.0,
        color_mix: -1.0,
    }
}

/// Closed hangar so the mean is the interior, not the sky. Red and green boxes
/// sit in front of the camera.
fn interior(settings: LightingSettings) -> World {
    present(Scene {
        floor: Floor {
            position: Vec3::ZERO,
            half_x: 4.0,
            half_z: 4.0,
            color: [0.55, 0.55, 0.55],
        },
        walls: vec![
            wall(0.0, -4.0, 4.0, 0.15),
            wall(0.0, 4.0, 4.0, 0.15),
            wall(-4.0, 0.0, 0.15, 4.0),
            wall(4.0, 0.0, 0.15, 4.0),
        ],
        solids: vec![
            box_at(-1.2, -0.4, [0.8, 0.05, 0.05]),
            box_at(1.2, -0.4, [0.05, 0.7, 0.1]),
        ],
        lights: vec![Light {
            position: Vec3::new(0.0, 2.6, 0.4),
            color: [8.0, 7.6, 6.8],
            direction: Vec3::ZERO,
        }],
        ceiling: Some(Ceiling {
            height: 3.0,
            color: [0.7, 0.7, 0.72],
        }),
        sky: Some(Sky {
            color: [0.11, 0.14, 0.19],
        }),
        lighting: settings,
        debug: genos_scene::DebugSettings::default(),
        fps_counter: genos_scene::FpsCounterSettings::default(),
        event_loop: genos_scene::EventLoopSettings::default(),
        reflections: genos_scene::ReflectionSettings::default(),
        hdr: genos_scene::HdrSettings::legacy_bytes(),
        fog: genos_scene::FogSettings::default(),
        menu: genos_scene::MenuSettings::default(),
    })
}

struct Look {
    mean: [f64; 3],
    /// One channel high, the others low. A solid red field scores near 1.
    saturated: f64,
    white: f64,
    /// Largest 16-level colour bucket. A flat blowout scores near 1.
    flat: f64,
    /// Saturated blue, the low-preset tile bug.
    blue: f64,
    alpha0: u32,
}

fn look_at(pixels: &[u8]) -> Look {
    let n = (pixels.len() / 4).max(1) as f64;
    let mut sum = [0u64; 3];
    let mut saturated = 0u32;
    let mut white = 0u32;
    let mut blue = 0u32;
    let mut alpha0 = 0u32;
    let mut buckets = std::collections::HashMap::<u32, u32>::new();
    for px in pixels.chunks_exact(4) {
        let (b, g, r) = (px[0], px[1], px[2]);
        sum[0] += u64::from(r);
        sum[1] += u64::from(g);
        sum[2] += u64::from(b);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        if max > 220 && min < 40 && max - min > 160 {
            saturated += 1;
        }
        if r > 230 && g > 230 && b > 230 {
            white += 1;
        }
        if b > 160 && r < 90 && g < 140 && b > r.saturating_add(40) {
            blue += 1;
        }
        if px[3] == 0 {
            alpha0 += 1;
        }
        let key = u32::from(r / 16) | (u32::from(g / 16) << 8) | (u32::from(b / 16) << 16);
        *buckets.entry(key).or_insert(0) += 1;
    }
    let mode = buckets.values().copied().max().unwrap_or(0) as f64;
    Look {
        mean: [sum[0] as f64 / n, sum[1] as f64 / n, sum[2] as f64 / n],
        saturated: saturated as f64 / n,
        white: white as f64 / n,
        flat: mode / n,
        blue: blue as f64 / n,
        alpha0,
    }
}

fn sample_rgb(pixels: &[u8], width: u32, height: u32, camera: &Camera, at: [f32; 3]) -> [f32; 3] {
    let aspect = width as f32 / height as f32;
    let uv = viewport_uv(camera, aspect, at).unwrap_or_else(|| panic!("off screen {at:?}"));
    let x = (uv[0] * width as f32).round() as i32;
    let y = (uv[1] * height as f32).round() as i32;
    let mut sum = [0.0; 3];
    let mut count = 0.0;
    for dy in -2..=2 {
        for dx in -2..=2 {
            let px = x + dx;
            let py = y + dy;
            if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
                continue;
            }
            let i = ((py as u32 * width + px as u32) * 4) as usize;
            sum[0] += pixels[i + 2] as f32;
            sum[1] += pixels[i + 1] as f32;
            sum[2] += pixels[i] as f32;
            count += 1.0;
        }
    }
    [sum[0] / count, sum[1] / count, sum[2] / count]
}

fn shoot(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &mut Camera,
    frames: u32,
    moving: bool,
) -> Vec<u8> {
    let mut picture = Vec::new();
    for _ in 0..frames {
        if moving {
            camera.position.x += 0.05;
            if camera.position.x > 1.2 {
                camera.position.x = -1.2;
            }
        }
        let _ = window.pump();
        picture = renderer
            .draw(world, camera, true)
            .expect("draw")
            .expect("pixels");
    }
    picture
}

fn assert_sane(name: &str, look: &Look) {
    assert_eq!(look.alpha0, 0, "{name}: unwritten pixels");
    assert!(
        look.flat < 0.85,
        "{name}: flat colour {:.3} mean {:?} (blowout)",
        look.flat,
        look.mean
    );
    assert!(
        look.saturated < 0.45,
        "{name}: saturated {:.3} mean {:?}",
        look.saturated,
        look.mean
    );
    assert!(
        look.white < 0.35,
        "{name}: white {:.3} mean {:?}",
        look.white,
        look.mean
    );
    assert!(
        look.blue < 0.12,
        "{name}: blue blocks {:.3} mean {:?}",
        look.blue,
        look.mean
    );
}

fn assert_boxes(name: &str, pixels: &[u8], width: u32, height: u32, camera: &Camera) {
    let red = sample_rgb(pixels, width, height, camera, [-1.2, 0.7, 0.15]);
    let green = sample_rgb(pixels, width, height, camera, [1.2, 0.7, 0.15]);
    assert!(
        red[0] > red[1] + 8.0 && red[0] > red[2] + 8.0 && red[0] < 230.0,
        "{name}: red box is not red {red:?}"
    );
    assert!(
        green[1] > green[0] + 8.0 && green[1] > green[2] + 8.0 && green[1] < 230.0,
        "{name}: green box is not green {green:?}"
    );
}

/// Each preset and each single feature off. Image stats must match the look:
/// coloured boxes stay coloured, and the frame is not an unwritten field, a
/// saturated field, or a flat-colour blowout. Readback is 8-bit, so a NaN or
/// inf in the shader shows up as one of those.
#[test]
fn presets_and_toggles_match_their_look() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let width = 320u32;
    let height = 180u32;
    let frames = 6u32;

    let mut cases: Vec<(&str, LightingSettings, bool)> = LightingPreset::all()
        .iter()
        .map(|preset| (preset.name(), LightingSettings::preset(*preset), true))
        .collect();
    let mut direct_off = LightingSettings::preset(LightingPreset::High);
    direct_off.direct.enabled = false;
    cases.push(("direct off", direct_off, false));
    let mut shadows_off = LightingSettings::preset(LightingPreset::High);
    shadows_off.shadows.enabled = false;
    cases.push(("shadows off", shadows_off, true));
    let mut gi_off = LightingSettings::preset(LightingPreset::High);
    gi_off.gi.enabled = false;
    cases.push(("gi off", gi_off, true));
    let mut bounces = LightingSettings::preset(LightingPreset::High);
    bounces.bounce.count = 0;
    cases.push(("bounces 0", bounces, true));
    let mut probes_off = LightingSettings::preset(LightingPreset::High);
    probes_off.probes.enabled = false;
    cases.push(("probes off", probes_off, true));
    let mut cache_off = LightingSettings::preset(LightingPreset::High);
    cache_off.world_cache.enabled = false;
    cases.push(("world cache off", cache_off, true));
    let mut denoise_off = LightingSettings::preset(LightingPreset::High);
    denoise_off.denoiser.enabled = false;
    cases.push(("denoise off", denoise_off, true));
    let mut reuse_off = LightingSettings::preset(LightingPreset::High);
    reuse_off.compose_reuse = false;
    cases.push(("compose reuse off", reuse_off, true));
    let mut sky_off = LightingSettings::preset(LightingPreset::High);
    sky_off.sky = false;
    cases.push(("sky off", sky_off, true));
    let mut emissive_off = LightingSettings::preset(LightingPreset::High);
    emissive_off.emissive = false;
    cases.push(("emissive off", emissive_off, true));
    let mut prefill_off = LightingSettings::preset(LightingPreset::High);
    prefill_off.prefill = false;
    cases.push(("prefill off", prefill_off, true));

    for (name, settings, boxes) in cases {
        let mut camera = Camera::new(0.0, 2.4, 0.0);
        // A still camera is the world-cache reuse path. The other cases move
        // so a stale probe field cannot hide a tile bug.
        let moving = name != "world cache off";
        let world = interior(settings);
        let pixels = shoot(
            window,
            renderer,
            &world,
            &mut camera,
            frames,
            moving,
        );
        let look = look_at(&pixels);
        assert_eq!(pixels.len(), (width * height * 4) as usize, "{name}");
        assert_sane(name, &look);
        if boxes {
            assert_boxes(name, &pixels, width, height, &camera);
        }
        if name == "direct off" {
            let red_field = look.mean[0] > 140.0 && look.mean[1] < 80.0 && look.mean[2] < 80.0;
            assert!(!red_field, "direct off is a red field {:?}", look.mean);
        }
    }
}

/// A floor point the lamp cannot see. Bounce 0 is flat ambient, bounce 1 adds
/// the probe bounce, bounce 2 adds cache-to-cache. Each step is brighter, and
/// each lower count records fewer dispatches.
#[test]
fn bounce_counts_brighten_the_shadow_and_skip_work() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let width = 320u32;
    let height = 180u32;
    let sample_at = [1.6, 0.02, 0.0];
    let mut camera = Camera::new(2.8, 2.2, -0.5);
    camera.pitch = -0.55;
    let mut levels = [0.0f32; 3];
    let mut dispatches = [0u32; 3];
    for (i, bounce) in [0u32, 1, 2].into_iter().enumerate() {
        let mut settings = LightingSettings::preset(LightingPreset::High);
        settings.bounce.count = bounce;
        settings.sky = false;
        settings.ambient.mode = AmbientMode::Constant;
        settings.ambient.color = [1.0, 1.0, 1.0];
        settings.ambient.intensity = 0.22;
        let world = shadow_corner(settings);
        let mut saw = [0u32; 12];
        let mut picture = Vec::new();
        for frame in 0..16 {
            let _ = window.pump();
            picture = renderer
                .draw(&world, &camera, true)
                .expect("draw")
                .expect("pixels");
            if frame >= 4 {
                let counts = renderer.lighting_dispatches();
                if counts.iter().any(|count| *count > 0) {
                    for (slot, count) in saw.iter_mut().zip(counts) {
                        *slot = (*slot).max(count);
                    }
                }
            }
        }
        let uv = viewport_uv(&camera, width as f32 / height as f32, sample_at);
        assert!(uv.is_some(), "shadow sample is off screen");
        let rgb = sample_rgb(&picture, width, height, &camera, sample_at);
        levels[i] = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
        dispatches[i] = saw.iter().sum();
    }
    assert!(
        levels[0] + 1.0 < levels[1] && levels[1] + 1.0 < levels[2],
        "shadow luminance bounce 0/1/2 {:?} dispatches {:?}",
        levels,
        dispatches
    );
    assert!(
        dispatches[0] < dispatches[1] && dispatches[1] < dispatches[2],
        "dispatches bounce 0/1/2 {:?} luminance {:?}",
        dispatches,
        levels
    );
}

fn shadow_corner(settings: LightingSettings) -> World {
    present(Scene {
        floor: Floor {
            position: Vec3::ZERO,
            half_x: 4.0,
            half_z: 4.0,
            color: [0.8, 0.8, 0.8],
        },
        walls: vec![
            wall(0.0, -4.0, 4.0, 0.12),
            wall(0.0, 4.0, 4.0, 0.12),
            wall(-4.0, 0.0, 0.12, 4.0),
            wall(4.0, 0.0, 0.12, 4.0),
        ],
        solids: vec![box_at(0.0, 0.0, [0.75, 0.75, 0.75])],
        lights: vec![Light {
            position: Vec3::new(-2.2, 1.5, 0.0),
            color: [10.0, 9.0, 8.0],
            direction: Vec3::ZERO,
        }],
        ceiling: Some(Ceiling {
            height: 3.0,
            color: [0.85, 0.85, 0.85],
        }),
        sky: None,
        lighting: settings,
        debug: genos_scene::DebugSettings::default(),
        fps_counter: genos_scene::FpsCounterSettings::default(),
        event_loop: genos_scene::EventLoopSettings::default(),
        reflections: genos_scene::ReflectionSettings::default(),
        hdr: genos_scene::HdrSettings::legacy_bytes(),
        fog: genos_scene::FogSettings::default(),
        menu: genos_scene::MenuSettings::default(),
    })
}

/// Still probe-only must not cost more than the surface cache. Settle is 32
/// still frames (`LIGHTING_SETTLE_FRAMES`) and the collected counters are one
/// frame behind, so the sample starts after that.
#[test]
fn world_cache_off_still_is_not_slower() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let frames = 48u32;
    let on = still_lighting(
        window,
        renderer,
        &interior(LightingSettings::preset(LightingPreset::High)),
        frames,
    );
    assert!(on.0 > 0.0, "surface cache recorded no lighting time");
    assert!(
        on.1[index("cache_rays")] > 0,
        "surface cache dispatched no cache rays"
    );
    let mut off_settings = LightingSettings::preset(LightingPreset::High);
    off_settings.world_cache.enabled = false;
    let off = still_lighting(window, renderer, &interior(off_settings), frames);
    assert_eq!(off.1[index("cache_rays")], 0, "cache rays still dispatched");
    assert_eq!(off.1[index("hit_light")], 0, "hit light still dispatched");
    assert_eq!(off.1[index("trace")], 0, "probe trace still dispatched");
    assert_eq!(off.1[index("atrous")], 0, "atrous still dispatched");
    assert!(
        off.0 <= on.0,
        "world cache off still lighting {:.3} ms exceeds on {:.3} ms",
        off.0,
        on.0
    );
}

fn still_lighting(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    frames: u32,
) -> (f64, [u32; 12]) {
    let camera = Camera::new(0.0, 2.4, 0.0);
    let mut cost = 0.0f64;
    let mut counts = [0u32; 12];
    let skip = index("shadows");
    for frame in 0..frames {
        let _ = window.pump();
        renderer.draw(world, &camera, false).expect("draw");
        if frame < 40 {
            continue;
        }
        let ms = renderer.lighting_pass_ms();
        if ms.iter().any(|value| *value > 0.0) {
            cost = ms
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != skip)
                .map(|(_, value)| *value)
                .sum();
            counts = renderer.lighting_dispatches();
        }
    }
    (cost, counts)
}
