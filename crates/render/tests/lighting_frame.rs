//! GI v2's frame path (lighting_gpu.rs): its pipelines build quickly, and its direct light
//! matches the current frame's direct light.

use std::time::Duration;

use genos_render::{DebugView, Renderer, ViewMode, World};
use genos_scene::{Camera, LightingPreset, LightingSettings, Scene};
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
        "nine compute pipelines plus the gbuffer; atrous-plain is not prefixed lighting: {times:?}"
    );
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

/// A dark room but for one emissive glTF cube: GI v2 draws the cube's own light,
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

/// Rays per placed probe GI v2's screen probes held in the last frame drawn.
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

/// GI v2 keeps visibility (ray hits), never light: a lamp that moves, dims or turns
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
    LightingSettings::preset(LightingPreset::High)
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
