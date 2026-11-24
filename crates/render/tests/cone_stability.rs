#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! The cone-traced cascades are stable: nothing is dropped, the presented
//! frame comes back bit for bit after the camera turns away and back, and a
//! probe's seat does not move with a turn of the head
//! (`docs/systems/lighting/cone-traced-cascades-plan.md`, section 18).
//! `scripts/ci-gate-knockout.sh` knocks out `cone_table_cap` and
//! `cone_field_seat` and these go red.

mod common;

use common::PresentedFrame;
use genos_render::{Renderer, World, CONE_FIELD_SEAT, CONE_TABLE_CAP};
use genos_scene::{Camera, Floor, LightingPreset, LightingSettings, Light, Scene, Vec3, Wall};
use genos_window::Window;

/// Frames drawn at a pose before it is read.
const SETTLE: u32 = 3;
/// A seat that moves more than this has moved (metres).
const SEAT_HELD: f32 = 0.001;

fn open() -> (std::sync::MutexGuard<'static, ()>, genos_window::Lifecycle<Renderer>) {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = GPU.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut window = Window::open_proof(320, 180).expect("proof window");
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height).expect("renderer");
    (guard, genos_window::Lifecycle::new(window, renderer, permit))
}

/// A floor, a wall and one lamp, in cone mode, with exposure fixed so a turn
/// does not move it.
fn scene() -> Scene {
    let mut scene = Scene::from_lists(
        Floor { position: Vec3::ZERO, half_x: 4.0, half_z: 4.0, color: [0.8, 0.8, 0.8] },
        vec![Wall {
            position: Vec3::new(0.0, 0.0, -1.0),
            half_x: 2.0,
            half_z: 0.1,
            height: 2.0,
            base: 0.0,
            color: [0.8, 0.7, 0.6],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }],
        Vec::new(),
        vec![Light { position: Vec3::new(0.5, 1.5, 0.5), color: [1.0, 0.9, 0.8], direction: Vec3::ZERO }],
    );
    scene.ceiling = None;
    scene.sky = None;
    let mut lighting = LightingSettings::preset(LightingPreset::Fallback);
    lighting.cone.enabled = true;
    scene.lighting = lighting;
    scene.hdr = genos_scene::HdrSettings::legacy_bytes();
    scene
}

fn camera(yaw: f32) -> Camera {
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(0.0, 1.4, 3.0), yaw, -0.25);
    camera
}

fn draw(window: &mut Window, renderer: &mut Renderer, world: &World, camera: &Camera) -> PresentedFrame {
    let mut frame = None;
    for _ in 0..SETTLE {
        frame = Some(PresentedFrame::from_draw(window, renderer, world, camera));
    }
    frame.expect("a frame")
}

/// Draw until the field is built and uploaded and the cascades have run.
fn resident(window: &mut Window, renderer: &mut Renderer, world: &World, camera: &Camera) {
    for _ in 0..600 {
        let _ = PresentedFrame::from_draw(window, renderer, world, camera);
        let stats = renderer.cone_stats().expect("cone stats");
        if stats.first().is_some_and(|s| s.probes > 0) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("the cone field never became resident");
}

/// Nothing is dropped, and a turn away and back gives the same presented
/// frame. Fails when `cone_table_cap` is knocked out.
#[test]
fn the_cone_cascades_drop_nothing_and_turn_back_to_the_same_frame() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let world = World::from_scene(scene());
    resident(window, renderer, &world, &camera(0.0));
    let before = draw(window, renderer, &world, &camera(0.0));
    let stats = renderer.cone_stats().expect("cone stats");
    let dropped: u32 = stats.iter().map(|s| s.dropped_compact + s.dropped_insert).sum();
    eprintln!("cone gate: probes {:?}, dropped {dropped}", stats.iter().map(|s| s.probes).collect::<Vec<_>>());
    assert_eq!(dropped, 0, "the cone cascades dropped {dropped} probes (guard {CONE_TABLE_CAP})");
    let _ = draw(window, renderer, &world, &camera(0.5));
    let after = draw(window, renderer, &world, &camera(0.0));
    let changed = before.pixels.chunks(4).zip(after.pixels.chunks(4)).filter(|(a, b)| a != b).count();
    eprintln!("cone gate: {changed} pixels changed after turning away and back");
    assert_eq!(changed, 0, "{changed} presented pixels changed after a turn away and back (guard {CONE_TABLE_CAP})");
}

/// Every cascade-0 probe seen before and after a 5 degree yaw keeps its seat.
/// Fails when `cone_field_seat` is knocked out.
#[test]
fn a_turn_of_the_head_does_not_move_a_cone_probe() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let world = World::from_scene(scene());
    resident(window, renderer, &world, &camera(0.0));
    let _ = draw(window, renderer, &world, &camera(0.0));
    let before = renderer.cone_probes(0).expect("probes");
    let _ = draw(window, renderer, &world, &camera(5.0_f32.to_radians()));
    let after = renderer.cone_probes(0).expect("probes");
    let mut shared = 0usize;
    let mut moved = 0usize;
    for a in &before {
        if let Some(b) = after.iter().find(|b| b.lod == a.lod && b.cell == a.cell) {
            shared += 1;
            let d = (0..3).map(|k| (a.seat[k] - b.seat[k]).powi(2)).sum::<f32>().sqrt();
            if d > SEAT_HELD {
                moved += 1;
            }
        }
    }
    eprintln!("cone gate: {moved} of {shared} shared probes moved their seat");
    assert!(shared > 0, "no probe was seen at both poses");
    assert_eq!(moved, 0, "{moved} of {shared} probes moved their seat with a turn of the head (guard {CONE_FIELD_SEAT})");
}
