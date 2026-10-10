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

/// Rays per placed probe GI v2's screen probes held in the last frame drawn.
fn samples_per_probe(renderer: &mut Renderer) -> f32 {
    let [rays, probes] = renderer.gi_v2_probe_samples();
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
fn gi_v2_probes_keep_their_rays_when_the_light_changes() {
    let (_gpu, mut window, mut renderer) = open();
    renderer.set_gi_v2(true).expect("GI v2");
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
/// traced again in the same frame (gi2_trace.comp), so the probes keep almost all
/// 512 rays (about 496 here when they were dropped until their stratum came round,
/// and which rays a cell held changed every frame: flicker around moving objects).
/// Strata traced from inside the moved object's bounds are traced again from the
/// probe's place in the same frame (a fill, GENOS_GI2_MOVED_DROP), which this
/// counter does not count as kept: about 502 here, 512 with it off.
/// The change shows at once.
#[test]
fn gi_v2_moving_object_retraces_the_rays_it_crosses() {
    let (_gpu, mut window, mut renderer) = open();
    renderer.set_gi_v2(true).expect("GI v2");
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
/// on the probe's plane, within GI2_REACH spacings of it. A slight turn keeps
/// almost all rays; turning round to surfaces last frame never saw keeps few.
#[test]
fn gi_v2_camera_keeps_rays_only_where_reprojection_holds() {
    let (_gpu, mut window, mut renderer) = open();
    renderer.set_gi_v2(true).expect("GI v2");
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
fn gi_v2_floor_change_drops_all_kept_rays() {
    let (_gpu, mut window, mut renderer) = open();
    renderer.set_gi_v2(true).expect("GI v2");
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
fn gi_v2_moving_camera_keeps_its_brightness() {
    let (_gpu, mut window, mut renderer) = open();
    renderer.set_gi_v2(true).expect("GI v2");
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
