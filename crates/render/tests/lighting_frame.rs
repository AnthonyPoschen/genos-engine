#![cfg_attr(test, expect(clippy::as_conversions, reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::assert_is_empty, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::cast_lossless, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::cast_possible_truncation, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::cast_precision_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::cast_sign_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::disallowed_methods, reason = "TODO(lint-2026-10-14): publish intent with the CLI argument, not set_var. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::doc_markdown, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(test, expect(clippy::manual_midpoint, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::many_single_char_names, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::redundant_closure_for_method_calls, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]

//! GI frame path (lighting_gpu.rs): its pipelines build quickly, and its direct light
//! matches the current frame's direct light.

use std::time::Duration;

use genos_render::{DebugView, LampPickCounts, LightingAtrousStats, Renderer, ViewMode, World};
use genos_scene::{AmbientMode, Camera, LampPickMode, LightingPreset, LightingSettings, Scene};
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

/// Longest a GI pipeline may take to build. The SDF march inlined into the current
/// shaders took more than four minutes on the 4070's compiler; each GI pass has one
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
fn lighting_pipelines_build_quickly() {
    let (_gpu, _window, renderer) = open();
    let times = renderer.pipeline_times();
    let lighting: Vec<_> = times
        .iter()
        .filter(|(name, _)| name.starts_with("lighting"))
        .collect();
    assert!(
        lighting.iter().all(|(name, _)| *name != "lighting sun compose"),
        "the sun compose is built on the first small step, not at startup: {times:?}"
    );
    // Nine compute names in LIGHTING_PASS_NAMES, plus "lighting gbuffer".
    // "atrous-plain" is timed, but its name does not start with "lighting",
    // so it is outside this count. "lighting sun compose" is built on the
    // first small sun step and is excluded by the assert above.
    assert_eq!(
        lighting.len(),
        10,
        "nine compute pipelines plus the gbuffer; atrous-plain and screen-denoise are not prefixed lighting: {times:?}"
    );
    let screen = times
        .iter()
        .find(|(name, _)| name == "screen-denoise")
        .expect("screen-denoise pipeline");
    let limit = pipeline_limit();
    for (name, took) in &times {
        eprintln!("pipeline {name}: {:.3} s", took.as_secs_f64());
    }
    for (name, took) in lighting {
        assert!(
            *took < limit,
            "{name} took {took:?} to build (limit {limit:?})"
        );
    }
    assert!(
        screen.1 < limit,
        "screen-denoise took {:?} to build (limit {limit:?})",
        screen.1
    );
}

#[test]
fn agx_display_draws_a_frame() {
    let (_gpu, mut window, mut renderer) = open();
    let mut world = shipped_hall();
    world.scene.hdr = genos_scene::HdrSettings::for_preset(LightingPreset::High);
    let camera = Camera::opening();
    let _ = window.pump();
    let pixels = renderer.draw(&world, &camera, true).expect("draw");
    let pixels = pixels.expect("pixels");
    assert!(!pixels.is_empty());
    assert!(renderer.hdr_exposure() > 0.0);
    assert!(!renderer.hdr_legacy_display());
}

fn draw(window: &mut Window, renderer: &mut Renderer, world: &World, camera: &Camera) -> Vec<u8> {
    let _ = window.pump();
    renderer
        .draw(world, camera, true)
        .expect("draw")
        .expect("readback")
}

#[test]
fn the_bounce_adds_light_to_the_direct_view() {
    let (_gpu, mut window, mut renderer) = open();
    let world = shipped_hall();
    let camera = Camera::opening();
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Direct,
        ..DebugView::default()
    });
    let direct = draw(&mut window, &mut renderer, &world, &camera);
    // The full picture has light the direct view does not: the probes' bounce.
    renderer.set_debug_view(DebugView::default());
    let full = draw(&mut window, &mut renderer, &world, &camera);
    let lit = |p: &[u8]| p.iter().map(|&c| u64::from(c)).sum::<u64>();
    assert!(lit(&full) > lit(&direct), "the bounce adds light");
}

/// A dark room but for one emissive glTF cube: GI draws the cube's own light,
/// and rays that hit it carry that light onto the floor in front of it.
#[test]
fn lighting_draws_emitted_light_and_bounces_it() {
    let (_gpu, mut window, mut renderer) = open();
    let gltf = genos_load::load_gltf_file(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../load/fixtures/cube.gltf"
    ))
    .expect("cube fixture");
    let settings = genos_bake::BakeSettings {
        mode: genos_bake::BakeMode::OnLoad,
        cache_dir: std::env::temp_dir().join("genos-lighting-emission-test"),
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
            lighting: genos_scene::LightingSettings::default(),
            debug: genos_scene::DebugSettings::default(),
            fps_counter: genos_scene::FpsCounterSettings::default(),
            event_loop: genos_scene::EventLoopSettings::default(),
            hdr: genos_scene::HdrSettings::legacy_bytes(),
            fog: genos_scene::FogSettings::default(),
            menu: genos_scene::MenuSettings::default(),
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

/// Rays per placed probe GI's screen probes held in the last frame drawn.
fn samples_per_probe(renderer: &mut Renderer) -> f32 {
    let [rays, probes] = renderer.lighting_probe_samples();
    assert!(probes > 0, "the probes are placed");
    rays as f32 / probes as f32
}

/// Draw the still hall until every probe holds all its strata (8 frames of 64
/// rays), well before the frame settles and stops running the probes.
fn fill_strata(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
) -> Vec<u8> {
    let mut picture = Vec::new();
    for _ in 0..10 {
        picture = draw(window, renderer, world, camera);
    }
    let held = samples_per_probe(renderer);
    assert!(
        held > 0.95 * 512.0,
        "a still probe holds its last 8 strata ({held} rays)"
    );
    picture
}

fn changed_pixels(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).filter(|(x, y)| x.abs_diff(**y) > 2).count()
}

/// GI keeps visibility (ray hits), never light: a lamp that moves, dims or turns
/// off, and a sun or sky that changes, show in full on the next frame while every
/// probe keeps all its kept rays, the far ones and the ones the lamp lights alike.
#[test]
fn lighting_probes_keep_their_rays_when_the_light_changes() {
    let (_gpu, mut window, mut renderer) = open();
    let base: Scene = shipped_hall().scene;
    assert!(!base.lights.is_empty());
    let camera = Camera::opening();
    let mut moved_lamp = base.clone();
    moved_lamp.lights[0].position.x += 1.5;
    let mut dimmed = base.clone();
    dimmed.lights[0].color = [
        dimmed.lights[0].color[0] * 0.25,
        dimmed.lights[0].color[1] * 0.25,
        dimmed.lights[0].color[2] * 2.0,
    ];
    let mut off = base.clone();
    off.lights.remove(0);
    let mut sun = base.clone();
    sun.lights.push(genos_scene::Light {
        position: genos_scene::Vec3::new(0.0, 0.0, 0.0),
        color: [2.0, 1.8, 1.5],
        direction: genos_scene::Vec3::new(0.3, -1.0, 0.2),
    });
    sun.sky = Some(genos_scene::Sky {
        color: [0.4, 0.5, 0.7],
    });
    let still_world = World::from_scene(base.clone());
    let changes: [(&str, Scene); 4] = [
        ("a lamp moving", moved_lamp),
        ("a lamp dimming", dimmed),
        ("a lamp going off", off),
        ("the sun and sky changing", sun),
    ];
    for (what, scene) in changes {
        let before = fill_strata(&mut window, &mut renderer, &still_world, &camera);
        let after = draw(
            &mut window,
            &mut renderer,
            &World::from_scene(scene),
            &camera,
        );
        let held = samples_per_probe(&mut renderer);
        assert!(
            held > 0.95 * 512.0,
            "{what}: every probe keeps its rays, near the change or far from it ({held})"
        );
        assert!(
            changed_pixels(&before, &after) > 0,
            "{what}: the first frame after the change shows it"
        );
    }
}

/// An object that moves: the kept rays whose paths cross where it was or is are
/// traced again in the same frame (lighting_trace.comp), so the probes keep almost all
/// 512 rays (about 496 here when they were dropped until their stratum came round,
/// and which rays a cell held changed every frame: flicker around moving objects).
/// Strata traced from inside the moved object's bounds are traced again from the
/// probe's place in the same frame (a fill, GENOS_LIGHTING_MOVED_DROP), which this
/// counter does not count as kept: about 502 here, 512 with it off.
/// The change shows at once.
#[test]
fn lighting_moving_object_retraces_the_rays_it_crosses() {
    let (_gpu, mut window, mut renderer) = open();
    let base: Scene = shipped_hall().scene;
    assert!(!base.solids.is_empty());
    let camera = Camera::opening();
    let mut moved = base.clone();
    moved.solids[0].position.x += 0.5;
    let before = fill_strata(
        &mut window,
        &mut renderer,
        &World::from_scene(base),
        &camera,
    );
    let full = samples_per_probe(&mut renderer);
    let after = draw(
        &mut window,
        &mut renderer,
        &World::from_scene(moved),
        &camera,
    );
    let held = samples_per_probe(&mut renderer);
    assert!(
        held > full - 14.0,
        "the rays that cross the moved object are traced again, not dropped ({full} -> {held})"
    );
    assert!(
        changed_pixels(&before, &after) > 0,
        "the first frame after the move shows it"
    );
}

/// A camera that moves keeps a probe's strata only where reprojection holds: last
/// frame's probes around the probe's old screen place lend the strata they traced
/// on the probe's plane, within LIGHTING_REACH spacings of it. A slight turn keeps
/// almost all rays; turning round to surfaces last frame never saw keeps few.
#[test]
fn lighting_camera_keeps_rays_only_where_reprojection_holds() {
    let (_gpu, mut window, mut renderer) = open();
    let world = shipped_hall();
    let camera = Camera::opening();
    let p = camera.position;
    let mut turned = Camera::opening();
    turned.set_pose(p, camera.yaw + 0.002, camera.pitch);
    let mut jumped = Camera::opening();
    jumped.set_pose(p, camera.yaw + std::f32::consts::PI, camera.pitch);
    fill_strata(&mut window, &mut renderer, &world, &camera);
    let _ = draw(&mut window, &mut renderer, &world, &turned);
    let held = samples_per_probe(&mut renderer);
    assert!(
        held > 0.9 * 512.0,
        "a slight turn keeps almost all kept rays ({held})"
    );
    fill_strata(&mut window, &mut renderer, &world, &camera);
    let _ = draw(&mut window, &mut renderer, &world, &jumped);
    let held = samples_per_probe(&mut renderer);
    assert!(held < 0.25 * 512.0, "turning round keeps few ({held})");
}

/// The traced world changing as a whole (here the floor's colour, which kept hits
/// hold) drops every kept ray: each probe is back to this frame's 64 rays, then
/// refills one stratum a frame.
#[test]
fn lighting_floor_change_drops_all_kept_rays() {
    let (_gpu, mut window, mut renderer) = open();
    let base: Scene = shipped_hall().scene;
    let camera = Camera::opening();
    let mut floor = base.clone();
    floor.floor.color = [0.2, 0.6, 0.3];
    fill_strata(
        &mut window,
        &mut renderer,
        &World::from_scene(base),
        &camera,
    );
    let changed = World::from_scene(floor);
    let _ = draw(&mut window, &mut renderer, &changed, &camera);
    let held = samples_per_probe(&mut renderer);
    assert!(held <= 64.5, "only this frame's rays are left ({held})");
    let _ = draw(&mut window, &mut renderer, &changed, &camera);
    let held = samples_per_probe(&mut renderer);
    assert!(
        held > 64.5 && held <= 128.5,
        "the next frame keeps the first and adds its own ({held})"
    );
}

/// Mean of a picture's RGB bytes.
fn mean_byte(p: &[u8]) -> f64 {
    p.chunks(4)
        .map(|c| (c[0] as f64 + c[1] as f64 + c[2] as f64) / 3.0)
        .sum::<f64>()
        / (p.len() / 4).max(1) as f64
}

/// A moving camera draws the bounce light as bright as the settled picture. Three
/// 12-frame clips end on the settled pose: a 1e-5 rad wiggle (nothing moves on
/// screen, but each frame's camera differs), a 0.55 m slide and a 19 degree turn;
/// the last frame of each must match the settled picture's mean. Until 2026-10-10
/// the gather projected a cell's mean light along its rays' mean direction, which
/// bends toward the normal once a probe holds strata traced from different world
/// cells: 1.05, 1.22 and 1.31 here.
#[test]
fn lighting_moving_camera_keeps_its_brightness() {
    let (_gpu, mut window, mut renderer) = open();
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Bounce,
        ..DebugView::default()
    });
    let world = shipped_hall();
    let camera = Camera::opening();
    let at = |dx: f32, yaw: f32| {
        let mut c = Camera::opening();
        c.set_pose(
            camera.position + genos_scene::Vec3::new(dx, 0.0, 0.0),
            camera.yaw + yaw,
            camera.pitch,
        );
        c
    };
    for clip in ["wiggle", "slide", "turn"] {
        let mut settled = Vec::new();
        for _ in 0..30 {
            settled = draw(&mut window, &mut renderer, &world, &camera);
        }
        let mut last = Vec::new();
        for i in 0..12 {
            let k = (11 - i) as f32;
            let c = match clip {
                "wiggle" if k > 0.0 => at(0.0, if i % 2 == 0 { 1.0e-5 } else { -1.0e-5 }),
                "slide" => at(-0.05 * k, 0.0),
                "turn" => at(0.0, -0.03 * k),
                _ => at(0.0, 0.0),
            };
            last = draw(&mut window, &mut renderer, &world, &c);
        }
        let ratio = mean_byte(&last) / mean_byte(&settled).max(1.0e-6);
        eprintln!("{clip}: moving / settled {ratio:.4}");
        assert!(
            (ratio - 1.0).abs() < 0.03,
            "{clip}: the moving picture's bounce light is {ratio:.4} of the settled one"
        );
    }
}

/// Mean absolute difference of two pictures' RGB bytes (0..255).
fn mean_abs(a: &[u8], b: &[u8]) -> f64 {
    let n = (a.len().min(b.len()) / 4).max(1);
    let mut sum = 0u64;
    for i in 0..n {
        let p = i * 4;
        sum += u64::from(a[p].abs_diff(b[p]));
        sum += u64::from(a[p + 1].abs_diff(b[p + 1]));
        sum += u64::from(a[p + 2].abs_diff(b[p + 2]));
    }
    sum as f64 / (3 * n) as f64
}

/// S78X dense fallback: a still hall stays on the work lists at threshold 1 and
/// switches to the dense sweep at threshold 0. The two pictures match, and the
/// stats report the fraction, the mode and the threshold.
#[test]
fn lighting_atrous_switches_to_dense_above_the_dirty_fraction() {
    let (_gpu, mut window, mut renderer) = open();
    let world = shipped_hall();
    let camera = Camera::opening();
    // The screen wavelet reports mode 3. This check is the hash-slot lists.
    let mut hash = LightingSettings::preset(LightingPreset::High);
    hash.denoiser.screen = false;
    renderer.set_lighting_settings(hash);
    renderer.set_lighting_atrous_dirty(1.0);
    let mut previous = Vec::new();
    let mut listed = Vec::new();
    for _ in 0..6 {
        previous = listed;
        listed = draw(&mut window, &mut renderer, &world, &camera);
    }
    let list_stats = renderer.lighting_atrous_stats();
    let temporal = mean_abs(&previous, &listed);
    eprintln!(
        "list dirty {:.4} mode {} threshold {:.4} filtered {} ran {} temporal {temporal:.3}",
        list_stats.dirty_fraction,
        list_stats.mode,
        list_stats.threshold,
        list_stats.filtered,
        list_stats.ran
    );
    assert!(list_stats.ran, "the a-trous levels ran");
    assert_eq!(list_stats.mode, 1, "threshold 1 keeps the work lists");
    assert!(
        (list_stats.threshold - 1.0).abs() < 1.0e-4,
        "threshold {}",
        list_stats.threshold
    );
    assert!(
        (0.0..=1.0).contains(&list_stats.dirty_fraction),
        "dirty fraction {}",
        list_stats.dirty_fraction
    );
    assert!(
        list_stats.filtered > 0,
        "filtered texels are the denominator"
    );

    renderer.set_lighting_atrous_dirty(0.0);
    let dense = draw(&mut window, &mut renderer, &world, &camera);
    let dense_stats = renderer.lighting_atrous_stats();
    let cross = mean_abs(&listed, &dense);
    eprintln!(
        "dense dirty {:.4} mode {} threshold {:.4} filtered {} cross {cross:.3}",
        dense_stats.dirty_fraction, dense_stats.mode, dense_stats.threshold, dense_stats.filtered
    );
    assert!(dense_stats.ran, "the dense frame ran the levels");
    assert_eq!(dense_stats.mode, 0, "threshold 0 is the dense sweep");
    assert!(
        dense_stats.threshold.abs() < 1.0e-4,
        "threshold {}",
        dense_stats.threshold
    );
    assert!(
        (0.0..=1.0).contains(&dense_stats.dirty_fraction),
        "dirty fraction {}",
        dense_stats.dirty_fraction
    );
    assert!(
        (list_stats.dirty_fraction - dense_stats.dirty_fraction).abs() < 0.35,
        "dirty fraction list {} dense {}",
        list_stats.dirty_fraction,
        dense_stats.dirty_fraction
    );
    assert!(
        cross <= temporal + 2.0,
        "list and dense pictures diverged: cross {cross:.3} same-mode step {temporal:.3}"
    );

    renderer.set_lighting_atrous_dirty(1.0);
    let _ = draw(&mut window, &mut renderer, &world, &camera);
    let back = renderer.lighting_atrous_stats();
    assert_eq!(
        back.mode, 1,
        "raising the threshold returns to the work lists"
    );
    assert!(back.ran, "the levels ran after the switch back");
}

fn denoise_settings() -> LightingSettings {
    let mut settings = LightingSettings::preset(LightingPreset::High);
    // The adaptive reference is the hash wavelet, not the screen filter.
    settings.denoiser.screen = false;
    settings
}

fn view_at(renderer: &mut Renderer, mode: ViewMode) {
    renderer.set_debug_view(DebugView { mode, lamp: 0 });
}

/// Per-pixel mean absolute channel error, 0..255.
fn pixel_errors(reference: &[u8], image: &[u8]) -> Vec<f64> {
    let n = reference.len().min(image.len()) / 4;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let p = i * 4;
        let e = (u32::from(reference[p].abs_diff(image[p]))
            + u32::from(reference[p + 1].abs_diff(image[p + 1]))
            + u32::from(reference[p + 2].abs_diff(image[p + 2]))) as f64
            / 3.0;
        out.push(e);
    }
    out
}

fn mean_of(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.iter().sum::<f64>() / v.len() as f64
}

fn percentile(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let i = ((v.len() - 1) as f64 * p).round() as usize;
    v[i.min(v.len() - 1)]
}

fn luma_bgra(px: &[u8]) -> f64 {
    0.2126 * f64::from(px[2]) + 0.7152 * f64::from(px[1]) + 0.0722 * f64::from(px[0])
}

fn mean_luma(frame: &[u8]) -> f64 {
    let n = frame.len() / 4;
    if n == 0 {
        return 0.0;
    }
    let mut sum = 0.0;
    for i in 0..n {
        sum += luma_bgra(&frame[i * 4..]);
    }
    sum / n as f64
}

/// Mean per-pixel luminance variance across still frames.
fn flicker_variance(frames: &[Vec<u8>]) -> f64 {
    let n = frames.iter().map(|f| f.len() / 4).min().unwrap_or(0);
    let k = frames.len() as f64;
    if n == 0 || k < 2.0 {
        return 0.0;
    }
    let mut acc = 0.0;
    for p in 0..n {
        let mut m = 0.0;
        let mut m2 = 0.0;
        for frame in frames {
            let y = luma_bgra(&frame[p * 4..]);
            m += y;
            m2 += y * y;
        }
        m /= k;
        acc += (m2 / k - m * m).max(0.0);
    }
    acc / n as f64
}

fn decode_normal(px: &[u8]) -> [f64; 3] {
    [
        f64::from(px[2]) / 255.0 * 2.0 - 1.0,
        f64::from(px[1]) / 255.0 * 2.0 - 1.0,
        f64::from(px[0]) / 255.0 * 2.0 - 1.0,
    ]
}

/// A pixel on a depth or normal break. The adaptive skip is not allowed to
/// smear light across that break.
fn depth_edge(depth: &[u8], normal: &[u8], index: usize, width: usize) -> bool {
    let x = (index / 4) % width;
    let y = (index / 4) / width;
    let height = depth.len() / 4 / width;
    if x + 1 >= width || y + 1 >= height {
        return false;
    }
    let right = index + 4;
    let down = index + width * 4;
    if depth[index].abs_diff(depth[right]) > 8 || depth[index].abs_diff(depth[down]) > 8 {
        return true;
    }
    let a = decode_normal(&normal[index..]);
    let b = decode_normal(&normal[right..]);
    let dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    dot < 0.85
}

fn adaptive_settings(variance: &str) -> LightingSettings {
    let mut settings = LightingSettings::preset(LightingPreset::High);
    settings.denoiser.screen = false;
    settings
        .set_key("denoise_adaptive", "on")
        .expect("denoise_adaptive");
    settings
        .set_key("denoise_variance", variance)
        .expect("denoise_variance");
    settings
}

fn assert_bounce_close(
    name: &str,
    reference: &[u8],
    image: &[u8],
    depth: &[u8],
    normal: &[u8],
    frames: &[Vec<u8>],
    full_flicker: f64,
) {
    let errors = pixel_errors(reference, image);
    let mean = mean_of(&errors);
    let mut sorted = errors.clone();
    let p99 = percentile(&mut sorted, 0.99);
    let width = 320usize;
    let mut edge = Vec::new();
    let mut flat = Vec::new();
    for (i, e) in errors.iter().copied().enumerate() {
        let index = i * 4;
        if index + 4 >= depth.len() || index + width * 4 >= depth.len() {
            continue;
        }
        if depth_edge(depth, normal, index, width) {
            edge.push(e);
        } else {
            flat.push(e);
        }
    }
    let edge_mean = mean_of(&edge);
    let flat_mean = mean_of(&flat);
    let flicker = flicker_variance(frames);
    eprintln!(
        "{name} mean {mean:.3} p99 {p99:.3} edge {edge_mean:.3} ({}) flat {flat_mean:.3} flicker {flicker:.3} full_flicker {full_flicker:.3}",
        edge.len()
    );
    assert!(mean < 3.0, "{name} mean error {mean:.3}");
    assert!(p99 < 12.0, "{name} p99 error {p99:.3}");
    assert!(
        edge_mean < flat_mean * 6.0 + 2.0,
        "{name} edge error {edge_mean:.3} smears past flat {flat_mean:.3}"
    );
    assert!(
        flicker < full_flicker * 4.0 + 1.0,
        "{name} flicker {flicker:.3} full {full_flicker:.3}"
    );
}

/// Adaptive iteration count against the full five-iteration picture.
///
/// `0.02` is the preset threshold. `0.1` forces the variance band down to one
/// iteration so the skip is actually taken on a young scene.
#[test]
fn adaptive_atrous_matches_the_full_reference() {
    let (_gpu, mut window, mut renderer) = open();
    let world = shipped_hall();
    let camera = Camera::opening();
    renderer.set_lighting_settings(denoise_settings());
    view_at(&mut renderer, ViewMode::Bounce);
    let mut full_frames = Vec::new();
    for _ in 0..6 {
        full_frames.push(draw(&mut window, &mut renderer, &world, &camera));
    }
    let reference = full_frames.last().expect("reference").clone();
    let full_flicker = flicker_variance(&full_frames[4..]);
    view_at(&mut renderer, ViewMode::Depth);
    let depth = draw(&mut window, &mut renderer, &world, &camera);
    view_at(&mut renderer, ViewMode::Normal);
    let normal = draw(&mut window, &mut renderer, &world, &camera);
    view_at(&mut renderer, ViewMode::Direct);
    let direct_full = draw(&mut window, &mut renderer, &world, &camera);

    for (name, settings) in [
        ("adaptive 0.02", adaptive_settings("0.02")),
        ("adaptive 0.10", adaptive_settings("0.1")),
    ] {
        renderer.set_lighting_settings(settings);
        view_at(&mut renderer, ViewMode::Bounce);
        let mut frames = Vec::new();
        for _ in 0..4 {
            frames.push(draw(&mut window, &mut renderer, &world, &camera));
        }
        assert_bounce_close(
            name,
            &reference,
            &frames[2],
            &depth,
            &normal,
            &frames,
            full_flicker,
        );
        view_at(&mut renderer, ViewMode::Direct);
        let direct = draw(&mut window, &mut renderer, &world, &camera);
        let gap = mean_abs(&direct_full, &direct);
        eprintln!("{name} direct {gap:.3}");
        assert!(gap < 1.0, "{name} direct moved: {gap:.3}");
    }
}

/// The settled-cache skip, from a cold cache, against one real wavelet.
///
/// The 4070 adaptive picture was darker and blotchy (still mean 15.9, p99 41)
/// while lavapipe's old check reported ~0.04. That check warmed up with the
/// full five-iteration filter, so buffer A already held the reference, and a
/// young texel's variance sat above 0.02 so the skip never ran. A threshold of
/// 0.1 forces one iteration. This run starts there, before any full filter has
/// published A. One configured iteration publishes A from the wavelet it
/// actually runs. The skip has to match that picture, not the 25 cm WRC the
/// push blends in when A is empty.
#[test]
fn adaptive_skip_keeps_the_bounce_bright() {
    let world = shipped_hall();
    let camera = Camera::opening();
    let adaptive = {
        let (_gpu, mut window, mut renderer) = open();
        renderer.set_lighting_settings(adaptive_settings("0.1"));
        view_at(&mut renderer, ViewMode::Bounce);
        let mut frames = Vec::new();
        for _ in 0..4 {
            frames.push(draw(&mut window, &mut renderer, &world, &camera));
        }
        frames.pop().expect("adaptive frame")
    };
    let one = {
        let (_gpu, mut window, mut renderer) = open();
        let mut settings = LightingSettings::preset(LightingPreset::High);
        settings.denoiser.screen = false;
        settings
            .set_key("denoise_iters", "1")
            .expect("denoise_iters");
        renderer.set_lighting_settings(settings);
        view_at(&mut renderer, ViewMode::Bounce);
        let mut frames = Vec::new();
        for _ in 0..4 {
            frames.push(draw(&mut window, &mut renderer, &world, &camera));
        }
        frames.pop().expect("one-iter frame")
    };
    let errors = pixel_errors(&one, &adaptive);
    let mean = mean_of(&errors);
    let mut sorted = errors.clone();
    let p99 = percentile(&mut sorted, 0.99);
    let y_skip = mean_luma(&adaptive);
    let y_one = mean_luma(&one);
    eprintln!(
        "adaptive skip vs 1 iter mean {mean:.3} p99 {p99:.3} luma {y_skip:.2} one {y_one:.2}"
    );
    // The bounce view on this hall is dim (luma about 2.6). A black frame is
    // not a comparison. Leaving A unpublished measured mean 0.133 p99 1.0
    // here; the 4070's unpublished A was mean 15.9 p99 41. Both fail below.
    assert!(y_one > 1.0, "one-iteration bounce is empty ({y_one:.2})");
    assert!(
        mean < 0.05,
        "adaptive skip mean error {mean:.3} against one wavelet"
    );
    assert!(
        p99 < 2.0,
        "adaptive skip p99 {p99:.3} (the 4070 p99 was 41)"
    );
    assert!(
        y_skip + 4.0 >= y_one,
        "adaptive skip darkened the bounce: luma {y_skip:.2} one-iter {y_one:.2}"
    );
}

fn high_bounce(
    screen: bool,
    frames: usize,
    shift: f32,
) -> (Vec<Vec<u8>>, LightingAtrousStats) {
    let (_gpu, mut window, mut renderer) = open();
    let mut settings = LightingSettings::preset(LightingPreset::High);
    settings.denoiser.screen = screen;
    renderer.set_lighting_settings(settings);
    view_at(&mut renderer, ViewMode::Bounce);
    let mut world = shipped_hall();
    let camera = Camera::opening();
    let x0 = world.scene.solids[0].position.x;
    let mut out = Vec::with_capacity(frames);
    for i in 0..frames {
        if shift != 0.0 {
            world.scene.solids[0].position.x = x0 + shift * (i as f32 + 1.0);
        }
        out.push(draw(&mut window, &mut renderer, &world, &camera));
    }
    let stats = renderer.lighting_atrous_stats();
    (out, stats)
}

/// Screen wavelet against the hash wavelet, still and with a moving solid.
/// Lavapipe run-to-run noise is about zero, so the bar is the 4070 picture
/// bar directly: mean 1.5, p99 10 on 0..255.
#[test]
fn screen_denoise_matches_the_hash_wavelet() {
    let (hash_still, hash_stats) = high_bounce(false, 6, 0.0);
    let (screen_still, screen_stats) = high_bounce(true, 6, 0.0);
    assert_picture("still", &hash_still[5], &screen_still[5]);
    assert_picture_cost("still", &screen_stats, 320 * 180);
    assert_eq!(
        hash_stats.screen_pixels, 0,
        "the hash wavelet does not count screen pixels"
    );

    let (hash_move, _) = high_bounce(false, 4, 0.35);
    let (screen_move, move_stats) = high_bounce(true, 4, 0.35);
    for i in 0..4 {
        assert_picture(&format!("move {i}"), &hash_move[i], &screen_move[i]);
    }
    assert_picture_cost("moving", &move_stats, 320 * 180);
}

fn assert_picture(name: &str, hash: &[u8], screen: &[u8]) {
    let errors = pixel_errors(hash, screen);
    let mean = mean_of(&errors);
    let mut sorted = errors.clone();
    let p99 = percentile(&mut sorted, 0.99);
    eprintln!("screen vs hash {name} mean {mean:.3} p99 {p99:.3}");
    assert!(
        mean <= 1.5,
        "{name} mean {mean:.3} is past the picture bar 1.5"
    );
    assert!(
        p99 <= 10.0,
        "{name} p99 {p99:.3} is past the picture bar 10"
    );
}

fn assert_picture_cost(name: &str, stats: &LightingAtrousStats, pixels: u32) {
    eprintln!(
        "screen {name} pixels {} coarse {} changed {} filtered {} ran {}",
        stats.screen_pixels, stats.coarse_pixels, stats.changed_pixels, stats.filtered, stats.ran
    );
    assert!(stats.ran, "{name} screen wavelet did not publish");
    assert!(stats.screen_pixels > 0, "{name} filtered no pixels");
    assert!(
        stats.screen_pixels <= pixels,
        "{name} fine count {} exceeds the picture {pixels}",
        stats.screen_pixels
    );
    assert!(
        stats.filtered <= pixels,
        "{name} filtered {} is the slot table, not the picture",
        stats.filtered
    );
    assert_ne!(stats.filtered, 1_048_576);
}

/// The curtains shot. The central banner is a thin two-sided cloth and the
/// lamps sit on the far side of it. Bright axis-aligned squares on the side
/// the camera sees are those lamps' 12.5 cm cells, when the front and the back
/// share a key. The test fills the lamp side, then cuts to the other side:
/// that side has to stay dark, and it has to stay free of those squares while
/// the camera moves. A shared key keeps the lamp's squares on the cut.
#[test]
fn sponza_banner_cells_stay_still_under_the_screen_wavelet() {
    let (hash_lit, hash_far) = banner_cut(false);
    let (screen_lit, screen_far) = banner_cut(true);
    eprintln!(
        "banner lamp side hash p95 {:.1} peaks {:.1} screen p95 {:.1} peaks {:.1}",
        hash_lit.p95, hash_lit.peaks, screen_lit.p95, screen_lit.peaks
    );
    eprintln!(
        "banner far side hash p95 {:.1} peaks {:.1} edge {:.3} flicker {:.3} screen p95 {:.1} peaks {:.1} edge {:.3} flicker {:.3}",
        hash_far.p95, hash_far.peaks, hash_far.edge, hash_far.flicker,
        screen_far.p95, screen_far.peaks, screen_far.edge, screen_far.flicker
    );
    assert!(
        hash_lit.p95 >= 12.0,
        "the lamp did not light the side that faces it ({:.1})",
        hash_lit.p95
    );
    assert!(
        screen_lit.p95 >= 12.0,
        "the lamp did not light the side that faces it ({:.1})",
        screen_lit.p95
    );
    // The far side reads a different axis. The lamp side's cells, including
    // their bright squares, stay on that axis.
    assert!(
        hash_far.p95 <= hash_lit.p95 * 0.45,
        "hash shows the far lamp on the camera side: p95 {:.1} lamp side {:.1}",
        hash_far.p95, hash_lit.p95
    );
    assert!(
        screen_far.p95 <= screen_lit.p95 * 0.45,
        "screen shows the far lamp on the camera side: p95 {:.1} lamp side {:.1}",
        screen_far.p95, screen_lit.p95
    );
    assert!(
        hash_far.peaks < 8.0,
        "hash bright squares on the camera side {:.1}",
        hash_far.peaks
    );
    assert!(
        screen_far.peaks < 8.0,
        "screen bright squares on the camera side {:.1}",
        screen_far.peaks
    );
    assert!(
        screen_far.edge <= hash_far.edge * 0.85 + 0.5,
        "screen edge {screen_edge:.3} did not beat hash {hash_edge:.3}",
        screen_edge = screen_far.edge,
        hash_edge = hash_far.edge
    );
    assert!(
        screen_far.flicker <= hash_far.flicker * 0.85 + 0.35,
        "screen flicker {:.3} did not beat hash {:.3}",
        screen_far.flicker, hash_far.flicker
    );
    if sponza_gltf_is_real() {
        let (hash_edge, hash_flicker) = sponza_curtain_scores(false);
        let (screen_edge, screen_flicker) = sponza_curtain_scores(true);
        eprintln!(
            "sponza curtains hash edge {hash_edge:.3} flicker {hash_flicker:.3} screen edge {screen_edge:.3} flicker {screen_flicker:.3}"
        );
        assert!(screen_edge <= hash_edge * 0.85 + 0.25);
        assert!(screen_flicker <= hash_flicker * 0.85 + 0.25);
    }
}

/// Sun-shadowed floor on high. Direct sun is blocked, so the picture there is
/// bounce. A small emitter lights one cell. The hash wavelet keeps that cell
/// brighter than its neighbours. The screen wavelet walks the same cells, so
/// the peak stays (erasing it was the pixel-step blur) and the floor points
/// stay inside the picture bar. The same points must not flicker harder than
/// the hash wavelet.
#[test]
fn shadowed_floor_bounce_stays_smooth_on_high() {
    let hash = floor_shadow_shot(false);
    let screen = floor_shadow_shot(true);
    let (err_mean, err_p99) = paired_luma(&hash.lumas, &screen.lumas);
    eprintln!(
        "shadow floor hash mean {:.1} peaks {:.1}..{:.1} flicker {:.3} screen mean {:.1} peaks {:.1}..{:.1} flicker {:.3} luma mean {err_mean:.3} p99 {err_p99:.3}",
        hash.mean, hash.peaks_min, hash.peaks_max, hash.flicker,
        screen.mean, screen.peaks_min, screen.peaks_max, screen.flicker
    );
    assert!(
        hash.mean >= 8.0,
        "the shadowed floor has no bounce to score ({:.1})",
        hash.mean
    );
    assert!(
        screen.mean >= 8.0,
        "the shadowed floor has no bounce to score ({:.1})",
        screen.mean
    );
    assert!(
        hash.peaks_min >= 20.0,
        "the hash wavelet did not keep a cell blotch ({:.1})",
        hash.peaks_min
    );
    assert!(
        screen.peaks_max <= hash.peaks_max + 8.0,
        "screen cell excess {:.1} is above the hash {:.1}",
        screen.peaks_max, hash.peaks_max
    );
    assert!(
        err_mean <= 1.5,
        "shadow floor mean {err_mean:.3} is past the picture bar 1.5"
    );
    assert!(
        err_p99 <= 10.0,
        "shadow floor p99 {err_p99:.3} is past the picture bar 10"
    );
    assert!(
        screen.flicker <= hash.flicker + 0.2,
        "screen flicker {:.3} is above hash {:.3}",
        screen.flicker, hash.flicker
    );
    assert!(
        screen.flicker <= 1.5,
        "screen flicker {:.3} on the shadowed floor",
        screen.flicker
    );
}

struct FloorShadow {
    mean: f64,
    peaks_min: f64,
    peaks_max: f64,
    flicker: f64,
    lumas: Vec<f64>,
}

fn paired_luma(hash: &[f64], screen: &[f64]) -> (f64, f64) {
    let n = hash.len().min(screen.len());
    let mut err: Vec<f64> = (0..n).map(|i| (hash[i] - screen[i]).abs()).collect();
    let mean = mean_of(&err);
    let p99 = percentile(&mut err, 0.99);
    (mean, p99)
}

#[derive(Clone, Copy)]
struct FloorCam {
    x: f32,
    y: f32,
    z: f32,
    yaw: f32,
    pitch: f32,
}

fn floor_shadow_shot(screen: bool) -> FloorShadow {
    let (_gpu, mut window, mut renderer) = open();
    let mut settings = LightingSettings::preset(LightingPreset::High);
    settings.denoiser.screen = screen;
    renderer.set_lighting_settings(settings);
    view_at(&mut renderer, ViewMode::Bounce);
    let world = shadow_floor_world();
    let still = FloorCam {
        x: 0.0,
        y: 1.45,
        z: -0.4,
        yaw: 0.0,
        pitch: -0.85,
    };
    let shots = [
        still,
        FloorCam { x: 0.05, ..still },
        FloorCam {
            x: 0.1,
            yaw: 0.012,
            ..still
        },
        FloorCam {
            x: 0.04,
            z: -0.46,
            pitch: -0.87,
            ..still
        },
        FloorCam {
            x: -0.05,
            yaw: -0.01,
            ..still
        },
    ];
    for _ in 0..10 {
        let camera = floor_camera(&shots[0]);
        let _ = draw(&mut window, &mut renderer, &world, &camera);
    }
    let mut frames = Vec::new();
    for cam in &shots {
        frames.push(draw(
            &mut window,
            &mut renderer,
            &world,
            &floor_camera(cam),
        ));
    }
    let still_lumas = floor_lumas(&frames[0], &shots[0]);
    let covered = still_lumas.len();
    assert!(
        covered >= 40,
        "the shadow patch left the frame ({covered} samples)"
    );
    let mut peaks_min = f64::MAX;
    let mut peaks_max = 0.0f64;
    for (frame, cam) in frames.iter().zip(shots.iter()) {
        let peaks = floor_peaks(frame, cam);
        peaks_min = peaks_min.min(peaks);
        peaks_max = peaks_max.max(peaks);
    }
    FloorShadow {
        mean: mean_of(&still_lumas),
        peaks_min,
        peaks_max,
        flicker: floor_flicker(&frames, &shots),
        lumas: still_lumas,
    }
}

fn floor_camera(cam: &FloorCam) -> Camera {
    let mut camera = Camera::opening();
    camera.set_pose(
        genos_scene::Vec3::new(cam.x, cam.y, cam.z),
        cam.yaw,
        cam.pitch,
    );
    camera
}

/// A wall blocks the sun. The floor beyond it is bounce only: sky, the lit
/// wall, and a small emitter beside the patch.
fn shadow_floor_world() -> World {
    let dir = std::env::temp_dir().join("genos-shadow-floor-test");
    std::fs::create_dir_all(&dir).expect("shadow dir");
    let glow = genos_load::load_gltf_file(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../load/fixtures/cube.gltf"
    ))
    .expect("cube");
    let mut glowing = glow;
    for material in &mut glowing.materials {
        material.emissive = [240.0, 96.0, 28.0];
        material.emissive_strength = 1.0;
    }
    let bake = genos_bake::BakeSettings {
        mode: genos_bake::BakeMode::OnLoad,
        cache_dir: dir.join("bake"),
        ..genos_bake::BakeSettings::default()
    };
    let mut world = World::from_scene(Scene {
        floor: genos_scene::Floor {
            position: genos_scene::Vec3::ZERO,
            half_x: 6.0,
            half_z: 6.0,
            color: [0.55, 0.42, 0.30],
        },
        walls: vec![
            genos_scene::Wall {
                position: genos_scene::Vec3::new(0.0, 0.0, -0.15),
                half_x: 3.0,
                half_z: 0.12,
                height: 2.8,
                base: 0.0,
                color: [0.4, 0.38, 0.34],
                absorption: 0.0,
                reflectance: 0.4,
                color_mix: -1.0,
            },
            genos_scene::Wall {
                position: genos_scene::Vec3::new(0.0, 0.0, -3.5),
                half_x: 3.2,
                half_z: 0.12,
                height: 3.0,
                base: 0.0,
                color: [0.86, 0.78, 0.64],
                absorption: 0.0,
                reflectance: 0.85,
                color_mix: -1.0,
            },
        ],
        solids: Vec::new(),
        lights: vec![genos_scene::Light {
            position: genos_scene::Vec3::ZERO,
            color: [4.0, 3.4, 2.6],
            direction: genos_scene::Vec3::new(0.08, -0.85, -0.55),
        }],
        ceiling: None,
        sky: Some(genos_scene::Sky {
            color: [0.02, 0.02, 0.025],
        }),
        lighting: LightingSettings::preset(LightingPreset::High),
        debug: genos_scene::DebugSettings::default(),
        fps_counter: genos_scene::FpsCounterSettings::default(),
        event_loop: genos_scene::EventLoopSettings::default(),
        hdr: genos_scene::HdrSettings::legacy_bytes(),
        fog: genos_scene::FogSettings::default(),
        menu: genos_scene::MenuSettings::default(),
    });
    // Beside the scored patch, low, so a few cells see it and the rest do not.
    // A few centimetres off the floor, just beside the scored patch, so the
    // nearest 12.5 cm cells see it and the cells one step away do not.
    let s = 0.035f32;
    let mut place = genos_render::identity_pose();
    place[0] = s;
    place[5] = s;
    place[10] = s;
    place[13] = 0.12;
    place[12] = 1.15 - 10.0 * s;
    place[14] = -1.5;
    world
        .add_gltf_traced(&glowing, &place, &bake)
        .expect("emitter");
    world
}

fn floor_points() -> Vec<[f32; 3]> {
    let mut pts = Vec::new();
    let mut x = -1.0f32;
    while x <= 1.001 {
        let mut z = -2.2f32;
        while z <= -0.999 {
            pts.push([x, 0.0, z]);
            z += 0.125;
        }
        x += 0.125;
    }
    pts
}

fn floor_lumas(frame: &[u8], cam: &FloorCam) -> Vec<f64> {
    floor_points()
        .into_iter()
        .filter_map(|p| floor_luma(frame, cam, p))
        .collect()
}

fn floor_luma(frame: &[u8], cam: &FloorCam, world: [f32; 3]) -> Option<f64> {
    let (x, y) = floor_project(cam, world)?;
    Some(luma_at(frame, 320, x, y))
}

/// A bright blotch is a floor point brighter than the four points one cache
/// cell away. The sun-shadow shot is these patches.
fn floor_peak_excess(frame: &[u8], cam: &FloorCam, p: [f32; 3]) -> f64 {
    let mut nb = [p; 4];
    nb[0][0] -= 0.125;
    nb[1][0] += 0.125;
    nb[2][2] -= 0.125;
    nb[3][2] += 0.125;
    let Some(c) = floor_luma(frame, cam, p) else {
        return 0.0;
    };
    let mut around = Vec::new();
    for q in nb {
        if let Some(v) = floor_luma(frame, cam, q) {
            around.push(v);
        }
    }
    if around.len() < 4 || around.iter().any(|v| c <= *v + 6.0) {
        return 0.0;
    }
    c - mean_of(&around)
}

fn floor_peaks(frame: &[u8], cam: &FloorCam) -> f64 {
    let peaks: Vec<f64> = floor_points()
        .into_iter()
        .map(|p| floor_peak_excess(frame, cam, p))
        .filter(|v| *v > 0.0)
        .collect();
    mean_of(&peaks)
}

fn floor_flicker(frames: &[Vec<u8>], cams: &[FloorCam]) -> f64 {
    let pts = floor_points();
    let mut acc = 0.0;
    let mut n = 0.0;
    for i in 0..frames.len().saturating_sub(1) {
        for p in &pts {
            let Some(a) = floor_luma(&frames[i], &cams[i], *p) else {
                continue;
            };
            let Some(b) = floor_luma(&frames[i + 1], &cams[i + 1], *p) else {
                continue;
            };
            acc += (a - b).abs();
            n += 1.0;
        }
    }
    if n == 0.0 { 0.0 } else { acc / n }
}

fn floor_project(cam: &FloorCam, world: [f32; 3]) -> Option<(usize, usize)> {
    let (sy, cy) = cam.yaw.sin_cos();
    let (sp, cp) = cam.pitch.sin_cos();
    let forward = [sy * cp, sp, -cy * cp];
    let right = [cy, 0.0, sy];
    let up = [
        right[1] * forward[2] - right[2] * forward[1],
        right[2] * forward[0] - right[0] * forward[2],
        right[0] * forward[1] - right[1] * forward[0],
    ];
    let rel = [
        world[0] - cam.x,
        world[1] - cam.y,
        world[2] - cam.z,
    ];
    let zv = rel[0] * forward[0] + rel[1] * forward[1] + rel[2] * forward[2];
    if zv < 1.0e-3 {
        return None;
    }
    let t = (30.0f32.to_radians()).tan();
    let aspect = 320.0f32 / 180.0;
    let ndc_x = (rel[0] * right[0] + rel[1] * right[1] + rel[2] * right[2]) / (zv * t * aspect);
    let ndc_y = (rel[0] * up[0] + rel[1] * up[1] + rel[2] * up[2]) / (zv * t);
    let px = (ndc_x + 1.0) * 0.5 * 320.0 - 0.5;
    let py = (1.0 - ndc_y) * 0.5 * 180.0 - 0.5;
    let x = (px + 0.5).floor() as i32;
    let y = (py + 0.5).floor() as i32;
    if x < 2 || y < 2 || x >= 318 || y >= 178 {
        return None;
    }
    Some((x as usize, y as usize))
}

struct BannerView {
    p95: f64,
    peaks: f64,
    edge: f64,
    flicker: f64,
}

/// (eye x, yaw, eye z). Yaw 0 looks along -Z. The lamp side is +Z; the far
/// side is -Z, yaw π, so the cloth sits between the camera and the lamp.
fn banner_camera(cam: (f32, f32, f32)) -> Camera {
    let mut camera = Camera::opening();
    camera.set_pose(genos_scene::Vec3::new(cam.0, 1.7, cam.2), cam.1, 0.0);
    camera
}

/// Fill the lamp side, then cut to the far side and score that side's first
/// frames. The far side is measured in the same cache: a shared 12.5 cm cell
/// still holds the lamp.
fn banner_cut(screen: bool) -> (BannerView, BannerView) {
    let (_gpu, mut window, mut renderer) = open();
    let mut settings = LightingSettings::preset(LightingPreset::High);
    settings.denoiser.screen = screen;
    renderer.set_lighting_settings(settings);
    view_at(&mut renderer, ViewMode::Bounce);
    // Beside the cloth, on the +Z side, so neither camera's view of the scored
    // region passes through the emitter.
    let world = banner_world(2.2, 0.45);
    let lamp_side = (0.0f32, 0.0f32, 3.2f32);
    for _ in 0..20 {
        let camera = banner_camera(lamp_side);
        let _ = draw(&mut window, &mut renderer, &world, &camera);
    }
    let lamp_frame = draw(
        &mut window,
        &mut renderer,
        &world,
        &banner_camera(lamp_side),
    );
    let lit = BannerView {
        p95: cloth_p95(&lamp_frame, lamp_side),
        peaks: cloth_square_peaks(&lamp_frame, lamp_side),
        edge: cell_edge_excess(&lamp_frame, 320, 6),
        flicker: 0.0,
    };
    let yaw = std::f32::consts::PI;
    let far_shots = [
        (0.0f32, yaw, -2.8f32),
        (0.04, yaw, -2.8),
        (0.08, yaw + 0.008, -2.8),
        (0.12, yaw, -2.8),
        (0.06, yaw - 0.008, -2.8),
    ];
    let mut frames = Vec::new();
    for cam in far_shots {
        frames.push(draw(
            &mut window,
            &mut renderer,
            &world,
            &banner_camera(cam),
        ));
    }
    let edge = frames
        .iter()
        .map(|f| cell_edge_excess(f, 320, 6))
        .sum::<f64>()
        / frames.len() as f64;
    let far = BannerView {
        p95: cloth_p95(&frames[0], far_shots[0]),
        peaks: cloth_square_peaks(&frames[0], far_shots[0]),
        edge,
        flicker: cloth_flicker(&frames, &far_shots),
    };
    (lit, far)
}

fn banner_world(lamp_x: f32, lamp_z: f32) -> World {
    let dir = std::env::temp_dir().join("genos-banner-cell-test");
    std::fs::create_dir_all(&dir).expect("banner dir");
    let gltf_path = dir.join("banner.gltf");
    std::fs::write(&gltf_path, banner_gltf()).expect("banner gltf");
    let banner = genos_load::load_gltf_file(&gltf_path).expect("banner");
    let glow = genos_load::load_gltf_file(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../load/fixtures/cube.gltf"
    ))
    .expect("cube");
    let mut glowing = glow;
    for material in &mut glowing.materials {
        material.emissive = [80.0, 36.0, 8.0];
        material.emissive_strength = 1.0;
    }
    let settings = genos_bake::BakeSettings {
        mode: genos_bake::BakeMode::OnLoad,
        cache_dir: dir.join(format!("bake-{lamp_x}-{lamp_z}")),
        ..genos_bake::BakeSettings::default()
    };
    let mut world = World::from_scene(Scene {
        floor: genos_scene::Floor {
            position: genos_scene::Vec3::ZERO,
            half_x: 8.0,
            half_z: 8.0,
            color: [0.02, 0.02, 0.02],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights: Vec::new(),
        ceiling: None,
        sky: None,
        lighting: LightingSettings::preset(LightingPreset::High),
        debug: genos_scene::DebugSettings::default(),
        fps_counter: genos_scene::FpsCounterSettings::default(),
        event_loop: genos_scene::EventLoopSettings::default(),
        hdr: genos_scene::HdrSettings::legacy_bytes(),
        fog: genos_scene::FogSettings::default(),
        menu: genos_scene::MenuSettings::default(),
    });
    world
        .add_gltf_traced(&banner, &genos_render::identity_pose(), &settings)
        .expect("banner mesh");
    // cube.gltf's parent sits at x = 10 and the child scale is 2. Scale 0.1
    // maps that parent to x = 1 and the cube to about 0.4 m, just off the cloth.
    let s = 0.1;
    let mut place = genos_render::identity_pose();
    place[0] = s;
    place[5] = s;
    place[10] = s;
    place[12] = lamp_x - 10.0 * s;
    place[13] = 1.7;
    place[14] = lamp_z;
    world
        .add_gltf_traced(&glowing, &place, &settings)
        .expect("emitter");
    world
}

fn banner_gltf() -> String {
    let mut bytes = Vec::new();
    for v in [
        -1.5f32, 0.2, 0.0, 1.5, 0.2, 0.0, 1.5, 3.2, 0.0, -1.5, 3.2, 0.0,
    ] {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    for _ in 0..4 {
        for v in [0.0f32, 0.0, 1.0] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    }
    for v in [0u16, 1, 2, 0, 2, 3] {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    let b64 = base64_encode(&bytes);
    format!(
        r#"{{
  "asset": {{"version": "2.0"}},
  "scene": 0,
  "scenes": [{{"nodes": [0]}}],
  "nodes": [{{"mesh": 0}}],
  "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0, "NORMAL": 1}}, "indices": 2, "material": 0}}]}}],
  "materials": [{{"doubleSided": true, "pbrMetallicRoughness": {{"baseColorFactor": [0.85, 0.82, 0.75, 1], "metallicFactor": 0, "roughnessFactor": 1}}}}],
  "buffers": [{{"byteLength": {len}, "uri": "data:application/octet-stream;base64,{b64}"}}],
  "bufferViews": [
    {{"buffer": 0, "byteOffset": 0, "byteLength": 48, "target": 34962}},
    {{"buffer": 0, "byteOffset": 48, "byteLength": 48, "target": 34962}},
    {{"buffer": 0, "byteOffset": 96, "byteLength": 12, "target": 34963}}
  ],
  "accessors": [
    {{"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3", "min": [-1.5, 0.2, 0], "max": [1.5, 3.2, 0]}},
    {{"bufferView": 1, "componentType": 5126, "count": 4, "type": "VEC3"}},
    {{"bufferView": 2, "componentType": 5123, "count": 6, "type": "SCALAR"}}
  ]
}}"#,
        len = bytes.len(),
    )
}

fn base64_encode(bytes: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let n = (u32::from(bytes[i]) << 16) | (u32::from(bytes[i + 1]) << 8) | u32::from(bytes[i + 2]);
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(T[((n >> 6) & 63) as usize] as char);
        out.push(T[(n & 63) as usize] as char);
        i += 3;
    }
    if i < bytes.len() {
        let a = bytes[i];
        let b = if i + 1 < bytes.len() { bytes[i + 1] } else { 0 };
        let n = (u32::from(a) << 16) | (u32::from(b) << 8);
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        if i + 1 < bytes.len() {
            out.push(T[((n >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        out.push('=');
    }
    out
}

fn roi_of(frame: &[u8], width: usize) -> impl Iterator<Item = (usize, usize)> {
    let height = frame.len() / 4 / width;
    let x0 = width / 4;
    let x1 = width * 3 / 4;
    let y0 = height / 4;
    let y1 = height * 3 / 4;
    (y0..y1).flat_map(move |y| (x0..x1).map(move |x| (x, y)))
}

fn luma_at(frame: &[u8], width: usize, x: usize, y: usize) -> f64 {
    luma_bgra(&frame[(y * width + x) * 4..])
}

/// Extra luma on the strongest cell-period phase, in 0..255. A smooth gradient
/// and dither score near 0. A piecewise-constant 12.5 cm grid scores the step
/// between cells. The projected cell is about `stride` pixels and is not aligned
/// to x = 0, so every phase of stride-1, stride, and stride+1 is scored.
fn cell_edge_excess(frame: &[u8], width: usize, stride: usize) -> f64 {
    let mut best = 0.0f64;
    let lo = stride.saturating_sub(1).max(1);
    for s in lo..=stride + 1 {
        for phase in 0..s {
            let mut boundary = 0.0;
            let mut boundary_n = 0.0;
            let mut interior = 0.0;
            let mut interior_n = 0.0;
            for (x, y) in roi_of(frame, width) {
                if x + 1 >= width * 3 / 4 {
                    continue;
                }
                let left = luma_at(frame, width, x, y);
                let right = luma_at(frame, width, x + 1, y);
                // The quad's silhouette is one step of the whole gradient.
                // The cell grid, if it is there, is in the lit interior.
                if left < 12.0 || right < 12.0 {
                    continue;
                }
                let d = (right - left).abs();
                if x % s == phase {
                    boundary += d;
                    boundary_n += 1.0;
                } else {
                    interior += d;
                    interior_n += 1.0;
                }
            }
            let b = if boundary_n > 0.0 { boundary / boundary_n } else { 0.0 };
            let i = if interior_n > 0.0 { interior / interior_n } else { 0.0 };
            best = best.max(b - i);
        }
    }
    best
}

/// Strongest period of a cache-cell grid. The projected cell is about `stride`
/// pixels, and the grid phase is not aligned to x = 0, so every phase of
/// stride-1, stride, and stride+1 is scored. A smooth gradient scores near 1.
fn cell_edge_ratio(frame: &[u8], width: usize, stride: usize) -> f64 {
    let mut best = 0.0f64;
    let lo = stride.saturating_sub(1).max(1);
    for s in lo..=stride + 1 {
        for phase in 0..s {
            let mut boundary = 0.0;
            let mut boundary_n = 0.0;
            let mut interior = 0.0;
            let mut interior_n = 0.0;
            for (x, y) in roi_of(frame, width) {
                if x + 1 >= width * 3 / 4 {
                    continue;
                }
                let d = (luma_at(frame, width, x + 1, y) - luma_at(frame, width, x, y)).abs();
                if x % s == phase {
                    boundary += d;
                    boundary_n += 1.0;
                } else {
                    interior += d;
                    interior_n += 1.0;
                }
            }
            let b = if boundary_n > 0.0 { boundary / boundary_n } else { 0.0 };
            let i = if interior_n > 0.0 { interior / interior_n } else { 0.0 };
            best = best.max(b / (i + 0.05));
        }
    }
    best
}

fn cloth_lumas(frame: &[u8], cam: (f32, f32, f32)) -> Vec<f64> {
    let mut v = Vec::new();
    let mut x = -1.0f32;
    while x <= 1.001 {
        let mut y = 0.8f32;
        while y <= 2.601 {
            if let Some((px, py)) = banner_project(cam, [x, y, 0.0]) {
                v.push(luma_at(frame, 320, px, py));
            }
            y += 0.1;
        }
        x += 0.1;
    }
    v
}

fn cloth_p95(frame: &[u8], cam: (f32, f32, f32)) -> f64 {
    percentile(&mut cloth_lumas(frame, cam), 0.95)
}

/// A bright square is a cloth point brighter than the four neighbours one
/// cache cell away. A smooth falloff is brighter on only one side, so it
/// scores 0. The curtains shot is these isolated squares.
fn cloth_square_peaks(frame: &[u8], cam: (f32, f32, f32)) -> f64 {
    let mut peaks = Vec::new();
    let mut x = -0.75f32;
    while x <= 0.751 {
        let mut y = 1.0f32;
        while y <= 2.401 {
            let here = [x, y, 0.0];
            let left = [x - 0.125, y, 0.0];
            let right = [x + 0.125, y, 0.0];
            let down = [x, y - 0.125, 0.0];
            let up = [x, y + 0.125, 0.0];
            if let (Some(c), Some(l), Some(r), Some(d), Some(u)) = (
                banner_project(cam, here),
                banner_project(cam, left),
                banner_project(cam, right),
                banner_project(cam, down),
                banner_project(cam, up),
            ) {
                let c = luma_at(frame, 320, c.0, c.1);
                let l = luma_at(frame, 320, l.0, l.1);
                let r = luma_at(frame, 320, r.0, r.1);
                let d = luma_at(frame, 320, d.0, d.1);
                let u = luma_at(frame, 320, u.0, u.1);
                if c > l + 6.0 && c > r + 6.0 && c > d + 6.0 && c > u + 6.0 {
                    peaks.push(c - 0.25 * (l + r + d + u));
                }
            }
            y += 0.125;
        }
        x += 0.125;
    }
    mean_of(&peaks)
}

/// Same points on the cloth (z = 0) across camera moves. A screen-space diff
/// also counts the picture sliding, which a stable field still does.
fn cloth_flicker(frames: &[Vec<u8>], cams: &[(f32, f32, f32)]) -> f64 {
    let mut pts = Vec::new();
    let mut x = -1.0f32;
    while x <= 1.001 {
        let mut y = 0.8f32;
        while y <= 2.601 {
            pts.push([x, y, 0.0]);
            y += 0.15;
        }
        x += 0.1;
    }
    let mut acc = 0.0;
    let mut n = 0.0;
    for i in 0..frames.len().saturating_sub(1) {
        for p in &pts {
            let Some((x0, y0)) = banner_project(cams[i], *p) else {
                continue;
            };
            let Some((x1, y1)) = banner_project(cams[i + 1], *p) else {
                continue;
            };
            acc += (luma_at(&frames[i], 320, x0, y0) - luma_at(&frames[i + 1], 320, x1, y1)).abs();
            n += 1.0;
        }
    }
    if n == 0.0 { 0.0 } else { acc / n }
}

fn banner_project(cam: (f32, f32, f32), world: [f32; 3]) -> Option<(usize, usize)> {
    let (sy, cy) = cam.1.sin_cos();
    let forward = [sy, 0.0, -cy];
    let right = [cy, 0.0, sy];
    let eye = [cam.0, 1.7, cam.2];
    let rel = [world[0] - eye[0], world[1] - eye[1], world[2] - eye[2]];
    let zv = rel[0] * forward[0] + rel[1] * forward[1] + rel[2] * forward[2];
    if zv < 1.0e-3 {
        return None;
    }
    let t = (30.0f32.to_radians()).tan();
    let aspect = 320.0f32 / 180.0;
    let ndc_x = (rel[0] * right[0] + rel[2] * right[2]) / (zv * t * aspect);
    let ndc_y = rel[1] / (zv * t);
    let px = (ndc_x + 1.0) * 0.5 * 320.0 - 0.5;
    let py = (1.0 - ndc_y) * 0.5 * 180.0 - 0.5;
    let x = (px + 0.5).floor() as i32;
    let y = (py + 0.5).floor() as i32;
    if x < 0 || y < 0 || x >= 320 || y >= 180 {
        return None;
    }
    Some((x as usize, y as usize))
}

fn roi_flicker(frames: &[Vec<u8>], width: usize) -> f64 {
    if frames.len() < 2 {
        return 0.0;
    }
    let mut acc = 0.0;
    let mut n = 0.0;
    for pair in frames.windows(2) {
        for (x, y) in roi_of(&pair[0], width) {
            acc += (luma_at(&pair[0], width, x, y) - luma_at(&pair[1], width, x, y)).abs();
            n += 1.0;
        }
    }
    if n == 0.0 { 0.0 } else { acc / n }
}

fn sponza_gltf_is_real() -> bool {
    let path = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/built/sponza/sponza.gltf"
    ));
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    !text.starts_with("version https://git-lfs.github.com/spec/v1")
}

fn sponza_curtain_scores(screen: bool) -> (f64, f64) {
    let (_gpu, mut window, mut renderer) = open();
    let mut settings = LightingSettings::preset(LightingPreset::High);
    settings.denoiser.screen = screen;
    renderer.set_lighting_settings(settings);
    view_at(&mut renderer, ViewMode::Bounce);
    let path = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/built/sponza/sponza.gltf"
    ));
    let scene = genos_load::load_gltf_file(path).expect("sponza");
    let settings_bake = genos_bake::BakeSettings {
        mode: genos_bake::BakeMode::OnLoad,
        cache_dir: std::env::temp_dir().join("genos-sponza-banner-test"),
        ..genos_bake::BakeSettings::default()
    };
    let mut world = World::from_scene(Scene {
        floor: genos_scene::Floor {
            position: genos_scene::Vec3::ZERO,
            half_x: 1.0,
            half_z: 1.0,
            color: [0.2, 0.2, 0.2],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights: vec![
            genos_scene::Light {
                position: genos_scene::Vec3::new(0.0, 6.0, 0.0),
                color: [6.0, 4.5, 3.0],
                direction: genos_scene::Vec3::ZERO,
            },
            genos_scene::Light {
                position: genos_scene::Vec3::ZERO,
                color: [2.0, 1.9, 1.7],
                direction: genos_scene::Vec3::new(0.25, -1.0, 0.15),
            },
        ],
        ceiling: None,
        sky: None,
        lighting: LightingSettings::preset(LightingPreset::High),
        debug: genos_scene::DebugSettings::default(),
        fps_counter: genos_scene::FpsCounterSettings::default(),
        event_loop: genos_scene::EventLoopSettings::default(),
        hdr: genos_scene::HdrSettings::legacy_bytes(),
        fog: genos_scene::FogSettings::default(),
        menu: genos_scene::MenuSettings::default(),
    });
    world
        .add_gltf_traced(&scene, &genos_render::identity_pose(), &settings_bake)
        .expect("sponza mesh");
    let mut frames = Vec::new();
    for x in [0.0f32, 0.05, 0.1] {
        let mut camera = Camera::opening();
        camera.set_pose(
            genos_scene::Vec3::new(x, 1.6, -4.5),
            std::f32::consts::PI,
            0.0,
        );
        frames.push(draw(&mut window, &mut renderer, &world, &camera));
    }
    let edge = frames.iter().map(|f| cell_edge_ratio(f, 320, 6)).sum::<f64>() / frames.len() as f64;
    (edge, roi_flicker(&frames, 320))
}

struct LampDiag;

impl Drop for LampDiag {
    fn drop(&mut self) {
        std::env::remove_var("GENOS_LIGHTING_LAMP_PICK_DIAG");
    }
}

fn lamp_camera() -> Camera {
    let mut camera = Camera::opening();
    camera.set_pose(genos_scene::Vec3::new(0.0, 1.7, 4.0), 0.0, 0.0);
    camera
}

fn white_lamps(n: usize) -> World {
    let mut lights = Vec::new();
    for i in 0..n {
        let x = (i as f32 - (n as f32 - 1.0) * 0.5) * 1.2;
        lights.push(genos_scene::Light {
            position: genos_scene::Vec3::new(x, 2.4, -1.0),
            color: [1.0, 1.0, 1.0],
            direction: genos_scene::Vec3::ZERO,
        });
    }
    World::from_scene(Scene {
        floor: genos_scene::Floor {
            position: genos_scene::Vec3::ZERO,
            half_x: 20.0,
            half_z: 20.0,
            color: [0.8, 0.8, 0.8],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights,
        ceiling: None,
        sky: None,
        lighting: LightingSettings::preset(LightingPreset::Potato),
        debug: genos_scene::DebugSettings::default(),
        fps_counter: genos_scene::FpsCounterSettings::default(),
        event_loop: genos_scene::EventLoopSettings::default(),
        hdr: genos_scene::HdrSettings::legacy_bytes(),
        fog: genos_scene::FogSettings::default(),
        menu: genos_scene::MenuSettings::default(),
    })
}

fn direct_pick(mode: LampPickMode, rays: u32) -> LightingSettings {
    let mut settings = LightingSettings::preset(LightingPreset::Potato);
    settings.direct.max_lights = 0;
    settings.sky = false;
    settings.ambient.mode = AmbientMode::Off;
    settings.shadows.enabled = true;
    settings.gi.enabled = false;
    settings.bounce.count = 0;
    settings.probes.enabled = false;
    settings.world_cache.enabled = false;
    settings.lamp_pick.mode = mode;
    settings.lamp_pick.rays = rays;
    settings.lamp_pick.radius = 0;
    settings.lamp_pick.unbiased = true;
    settings
}

fn max_channel(a: &[u8], b: &[u8]) -> u8 {
    let n = a.len().min(b.len()) / 4;
    let mut worst = 0u8;
    for i in 0..n {
        let p = i * 4;
        worst = worst
            .max(a[p].abs_diff(b[p]))
            .max(a[p + 1].abs_diff(b[p + 1]))
            .max(a[p + 2].abs_diff(b[p + 2]));
    }
    worst
}

fn traced_rays(counts: LampPickCounts) -> u32 {
    counts
        .pixels
        .saturating_add(counts.kept)
        .saturating_add(counts.fills)
        .saturating_add(counts.cache)
}

/// Direct view, white lamps, no sun. Covering every lamp matches the full sum.
/// One sample per pixel reconstructs that sum and does not flicker.
#[test]
fn lamp_pick_pixels_match_the_full_sum() {
    let (_gpu, mut window, mut renderer) = open();
    let world = white_lamps(3);
    let camera = lamp_camera();
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Direct,
        ..DebugView::default()
    });
    renderer.set_lighting_settings(direct_pick(LampPickMode::Off, 1));
    let off = draw(&mut window, &mut renderer, &world, &camera);
    assert!(
        mean_byte(&off) > 1.0,
        "the full sum is black ({})",
        mean_byte(&off)
    );
    renderer.set_lighting_settings(direct_pick(LampPickMode::Hits, 1));
    let hits = draw(&mut window, &mut renderer, &world, &camera);
    assert!(
        max_channel(&off, &hits) <= 1,
        "hits on bounce 0 changed a pixel by {}",
        max_channel(&off, &hits)
    );
    renderer.set_lighting_settings(direct_pick(LampPickMode::Pixels, 4));
    let covered = draw(&mut window, &mut renderer, &world, &camera);
    assert!(
        max_channel(&off, &covered) <= 1,
        "rays that cover the lamps differ by {}",
        max_channel(&off, &covered)
    );
    renderer.set_lighting_settings(direct_pick(LampPickMode::Pixels, 1));
    let once = draw(&mut window, &mut renderer, &world, &camera);
    let again = draw(&mut window, &mut renderer, &world, &camera);
    eprintln!(
        "lamp pick rays=1 mean_abs {:.4} stable {:.4}",
        mean_abs(&off, &once),
        mean_abs(&once, &again)
    );
    assert!(
        mean_abs(&once, &again) < 0.05,
        "a still pixel sample flickered"
    );
    assert!(
        mean_abs(&off, &once) < 1.5,
        "one visible sample left the full sum"
    );
}

/// Off traces a ray per lamp. One sample per pixel does not.
#[test]
fn lamp_pick_shadow_rays_do_not_grow_with_the_lamp_count() {
    let (_gpu, mut window, mut renderer) = open();
    let _diag = LampDiag;
    std::env::set_var("GENOS_LIGHTING_LAMP_PICK_DIAG", "1");
    let camera = lamp_camera();
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Direct,
        ..DebugView::default()
    });
    let count_of = |renderer: &mut Renderer,
                    window: &mut Window,
                    lights: usize,
                    mode: LampPickMode,
                    rays: u32| {
        let world = white_lamps(lights);
        renderer.set_lighting_settings(direct_pick(mode, rays));
        let _ = draw(window, renderer, &world, &camera);
        traced_rays(renderer.lighting_lamp_pick_counts())
    };
    let off3 = count_of(&mut renderer, &mut window, 3, LampPickMode::Off, 1);
    let off9 = count_of(&mut renderer, &mut window, 9, LampPickMode::Off, 1);
    eprintln!("lamp pick off rays 3={off3} 9={off9}");
    assert!(off3 > 0, "off traced nothing");
    assert!(
        off9 > off3 * 2,
        "off did not grow with the lamps ({off3} -> {off9})"
    );
    let one3 = count_of(&mut renderer, &mut window, 3, LampPickMode::Pixels, 1);
    let one9 = count_of(&mut renderer, &mut window, 9, LampPickMode::Pixels, 1);
    eprintln!("lamp pick pixels rays 3={one3} 9={one9}");
    assert!(one3 > 0 && one9 > 0, "pixels traced nothing");
    assert!(
        one9 < one3 + one3 / 2,
        "one sample per pixel grew with the lamps ({one3} -> {one9})"
    );
}

/// Reuse reconnects a moved lamp. Radius 0 merges no neighbour. Radius 8 does.
#[test]
fn lamp_pick_reuse_follows_a_moved_lamp() {
    let (_gpu, mut window, mut renderer) = open();
    let _diag = LampDiag;
    std::env::set_var("GENOS_LIGHTING_LAMP_PICK_DIAG", "1");
    let mut world = white_lamps(3);
    let camera = lamp_camera();
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Direct,
        ..DebugView::default()
    });
    let mut settings = LightingSettings::preset(LightingPreset::High);
    settings.direct.max_lights = 0;
    settings.sky = false;
    settings.ambient.mode = AmbientMode::Off;
    settings.shadows.enabled = true;
    settings.gi.enabled = true;
    settings.bounce.count = 1;
    settings.probes.enabled = false;
    settings.world_cache.enabled = true;
    settings.denoiser.enabled = false;
    settings.prefill = false;
    settings.compose_reuse = false;
    settings.lamp_pick.mode = LampPickMode::Reuse;
    settings.lamp_pick.rays = 1;
    settings.lamp_pick.radius = 0;
    settings.lamp_pick.unbiased = true;
    renderer.set_lighting_settings(settings.clone());
    let _ = draw(&mut window, &mut renderer, &world, &camera);
    world.scene.lights[0].position.x += 3.0;
    let moved = draw(&mut window, &mut renderer, &world, &camera);
    let spatial = renderer.lighting_lamp_pick_counts().spatial;
    assert_eq!(spatial, 0, "radius 0 merged a neighbour");
    settings.lamp_pick.radius = 8;
    renderer.set_lighting_settings(settings.clone());
    let _ = draw(&mut window, &mut renderer, &world, &camera);
    let spatial = renderer.lighting_lamp_pick_counts().spatial;
    assert!(spatial > 0, "radius 8 merged no neighbour");
    settings.lamp_pick.mode = LampPickMode::Off;
    renderer.set_lighting_settings(settings);
    let full = draw(&mut window, &mut renderer, &world, &camera);
    eprintln!("lamp pick reuse mean_abs {:.4}", mean_abs(&moved, &full));
    assert!(mean_byte(&full) > 1.0, "the moved lamps are black");
    assert!(
        mean_abs(&moved, &full) < 1.5,
        "reuse left the old lamp shadow"
    );
}

/// A box around the centre lamp. The side lamps still reach the floor. The
/// centre lamp's unshadowed weight stays in the prefix, and its shadow ray does not.
fn shadowed_lamps() -> World {
    let mut world = white_lamps(3);
    world.scene.solids.push(genos_scene::Solid {
        shape: genos_scene::Shape::Square,
        position: genos_scene::Vec3::new(0.0, 0.0, -1.0),
        size: 0.8,
        height: 4.0,
        yaw: 0.0,
        color: [0.35, 0.35, 0.35],
        absorption: 0.0,
        reflectance: -1.0,
        color_mix: -1.0,
    });
    // from_scene is what registers the solid with the tracer.
    World::from_scene(world.scene)
}

/// GI on so a still pixel can accept history. Bounce 1 and the world-cache bit
/// keep the quiet-frame pixel skip off, and the direct view drops the bounce.
fn history_pick(mode: LampPickMode, rays: u32) -> LightingSettings {
    let mut settings = LightingSettings::preset(LightingPreset::High);
    settings.direct.max_lights = 0;
    settings.sky = false;
    settings.ambient.mode = AmbientMode::Off;
    settings.shadows.enabled = true;
    settings.gi.enabled = true;
    settings.bounce.count = 1;
    settings.probes.enabled = false;
    settings.world_cache.enabled = true;
    settings.denoiser.enabled = false;
    settings.prefill = false;
    settings.compose_reuse = false;
    settings.lamp_pick.mode = mode;
    settings.lamp_pick.rays = rays;
    settings.lamp_pick.radius = 0;
    settings.lamp_pick.unbiased = true;
    settings
}

/// Each mode against off, direct view, one lamp shadowed.
///
/// Hits, pixels, and both match off when the shadow-ray count covers the lamps,
/// because pixels stay on the full sum. Reuse does not. Radius 0, so the miss
/// is one visibility bit times the unshadowed sum, not a neighbour reservoir.
#[test]
fn lamp_pick_each_mode_against_off() {
    let (_gpu, mut window, mut renderer) = open();
    let _diag = LampDiag;
    std::env::set_var("GENOS_LIGHTING_LAMP_PICK_DIAG", "1");
    let world = shadowed_lamps();
    let camera = lamp_camera();
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Direct,
        ..DebugView::default()
    });
    renderer.set_lighting_settings(history_pick(LampPickMode::Off, 1));
    let open = draw(&mut window, &mut renderer, &white_lamps(3), &camera);
    let off = draw(&mut window, &mut renderer, &world, &camera);
    let boxed = mean_abs(&open, &off);
    let off_rays = renderer.lighting_lamp_pick_counts().pixels;
    assert!(
        boxed > 2.0,
        "the centre lamp is not shadowed ({boxed:.4})"
    );
    assert!(
        mean_byte(&off) > 1.0,
        "the shadowed full sum is black ({})",
        mean_byte(&off)
    );
    let matches = |renderer: &mut Renderer, window: &mut Window, mode, rays| {
        renderer.set_lighting_settings(history_pick(mode, rays));
        let picture = draw(window, renderer, &world, &camera);
        max_channel(&off, &picture)
    };
    let hits = matches(&mut renderer, &mut window, LampPickMode::Hits, 1);
    let pixels = matches(&mut renderer, &mut window, LampPickMode::Pixels, 4);
    let both = matches(&mut renderer, &mut window, LampPickMode::Both, 4);
    assert!(hits <= 1, "hits changed a pixel by {hits}");
    assert!(pixels <= 1, "covering pixel samples differ by {pixels}");
    assert!(both <= 1, "covering both samples differ by {both}");
    renderer.set_lighting_settings(history_pick(LampPickMode::Reuse, 1));
    let _ = draw(&mut window, &mut renderer, &world, &camera);
    let reused = draw(&mut window, &mut renderer, &world, &camera);
    let counts = renderer.lighting_lamp_pick_counts();
    let err = mean_abs(&off, &reused);
    eprintln!(
        "lamp pick occluded box {boxed:.4} max hits={hits} pixels={pixels} both={both} reuse {err:.4} rays {} / {off_rays} spatial {}",
        counts.pixels, counts.spatial
    );
    assert_eq!(counts.spatial, 0, "radius 0 merged a neighbour");
    assert!(
        counts.pixels * 2 < off_rays,
        "reuse did not drop to one shadow ray ({} vs {off_rays})",
        counts.pixels
    );
    assert!(
        err > 10.0,
        "reuse matched off ({err:.4}); the one-sample visibility error did not show"
    );
}
