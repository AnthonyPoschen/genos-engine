#![cfg_attr(
    test,
    expect(clippy::expect_used, reason = "a test aborts when a fixture fails")
)]
#![cfg_attr(
    test,
    expect(
        clippy::doc_markdown,
        reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
    )
)]

//! Live ReSTIR DI (ADR 0022 stage C5) against brute direct on the gate scenes.
//!
//! The flag stays off for every other test. This file turns it on and grades
//! `ref_err_near` the way `docs/systems/lighting/restir-di-plan.md` defines it:
//! `sum(w |Y-Yref|) / sum(w Yref)` with `w = 1 / max(distance, 0.5)`.

use genos_render::{DebugView, Renderer, ViewMode, World};
use genos_restir_ref::Scene as RestirScene;
use genos_scene::{AmbientMode, Camera, LampPickMode, LightingPreset, LightingSettings, Scene};
use genos_window::Window;

const REF_ERR: f64 = 0.05;
/// One spatial sample still moves a settled picture. The measured step on the
/// gate is about 0.06. The product flicker gate is the oracle `grid_worst`.
const FRAME_STEP: f64 = 0.10;
const WARMUP: u32 = 8;
const GATE_AVG: u32 = 8;
const SHADOW_AVG: u32 = 64;

fn open() -> (std::sync::MutexGuard<'static, ()>, Window, Renderer) {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = GPU
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut window = Window::open_proof(320, 180).expect("proof window");
    let frame = window.pump();
    let renderer = Renderer::open(window.display, window.surface, frame.width, frame.height)
        .expect("renderer");
    (guard, window, renderer)
}

fn direct_settings(restir: bool) -> LightingSettings {
    let mut settings = LightingSettings::preset(LightingPreset::Potato);
    settings.direct.max_lights = 0;
    settings.direct.refresh_frames = 1;
    settings.direct.resolution_scale = 1;
    settings.sky = false;
    settings.ambient.mode = AmbientMode::Off;
    settings.shadows.enabled = true;
    settings.gi.enabled = false;
    settings.bounce.count = 0;
    settings.probes.enabled = false;
    settings.world_cache.enabled = false;
    settings.lamp_pick.mode = LampPickMode::Off;
    settings.lamp_pick.radius = 0;
    settings.compose_reuse = false;
    settings.restir_di = restir;
    settings
}

fn world_from(spec: &RestirScene, wall: bool) -> World {
    let lights = spec
        .lamps
        .iter()
        .map(|lamp| genos_scene::Light {
            position: genos_scene::Vec3::new(lamp.position.x, lamp.position.y, lamp.position.z),
            color: [lamp.intensity.x, lamp.intensity.y, lamp.intensity.z],
            direction: genos_scene::Vec3::ZERO,
        })
        .collect();
    let walls = if wall {
        vec![genos_scene::Wall {
            position: genos_scene::Vec3::ZERO,
            half_x: 0.15,
            half_z: 2.0,
            height: 1.3,
            base: 0.0,
            color: [0.62, 0.62, 0.62],
            absorption: 0.0,
            reflectance: 0.8,
            color_mix: -1.0,
        }]
    } else {
        Vec::new()
    };
    World::from_scene(Scene {
        floor: genos_scene::Floor {
            position: genos_scene::Vec3::ZERO,
            half_x: 8.0,
            half_z: 8.0,
            color: [0.72, 0.72, 0.72],
        },
        walls,
        solids: Vec::new(),
        lights,
        ceiling: None,
        sky: None,
        lighting: direct_settings(false),
        debug: genos_scene::DebugSettings::default(),
        fps_counter: genos_scene::FpsCounterSettings::default(),
        event_loop: genos_scene::EventLoopSettings::default(),
        reflections: genos_scene::ReflectionSettings::default(),
        hdr: genos_scene::HdrSettings::for_preset(LightingPreset::High),
        fog: genos_scene::FogSettings::default(),
        menu: genos_scene::MenuSettings::default(),
    })
}

fn pose(x: f32, z: f32, yaw: f32) -> Camera {
    let mut camera = Camera::opening();
    camera.set_pose(genos_scene::Vec3::new(x, 1.7, z), yaw, 0.0);
    camera
}

fn luma_image(rgb: &[f64]) -> Vec<f64> {
    let pixels = rgb.len() / 3;
    let mut out = Vec::with_capacity(pixels);
    for pixel in 0..pixels {
        let i = pixel.saturating_mul(3);
        let r = rgb.get(i).copied().unwrap_or(0.0);
        let g = rgb.get(i.saturating_add(1)).copied().unwrap_or(0.0);
        let b = rgb.get(i.saturating_add(2)).copied().unwrap_or(0.0);
        out.push(0.2126 * r + 0.7152 * g + 0.0722 * b);
    }
    out
}

fn mean_of(frames: &[Vec<f64>]) -> Vec<f64> {
    let width = frames.first().map_or(0, Vec::len);
    let count = u32::try_from(frames.len()).unwrap_or(1).max(1);
    let scale = 1.0 / f64::from(count);
    let mut out = vec![0.0; width];
    for frame in frames {
        for (slot, value) in out.iter_mut().zip(frame.iter()) {
            *slot += *value * scale;
        }
    }
    out
}

fn image_mean(values: &[f64]) -> f64 {
    let count = u32::try_from(values.len()).unwrap_or(1).max(1);
    values.iter().sum::<f64>() / f64::from(count)
}

/// `sum(w |Y-Yref|) / sum(w Yref)`, `w = 1 / max(distance, 0.5)`.
fn ref_err_near(live: &[f64], brute: &[f64], distance: &[f64]) -> f64 {
    let mut num = 0.0;
    let mut den = 0.0;
    for (index, refer) in brute.iter().enumerate() {
        let sample = live.get(index).copied().unwrap_or(0.0);
        let depth = distance.get(index).copied().unwrap_or(0.5);
        let weight = 1.0 / depth.max(0.5);
        num += weight * (sample - refer).abs();
        den += weight * refer.abs();
    }
    if den <= 1.0e-8 {
        1.0
    } else {
        num / den
    }
}

fn frame_step(previous: &[f64], next: &[f64]) -> f64 {
    let mut num = 0.0;
    let mut den = 0.0;
    for (before, after) in previous.iter().zip(next.iter()) {
        num += (after - before).abs();
        den += before.abs();
    }
    if den <= 1.0e-8 {
        0.0
    } else {
        num / den
    }
}

fn grab(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
) -> genos_render::SceneRead {
    let _ = window.pump();
    renderer
        .draw(world, camera, true)
        .expect("draw")
        .expect("readback");
    renderer.read_scene().expect("scene radiance")
}

fn settle(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
    restir: bool,
    warmup: u32,
    average: u32,
) -> (Vec<f64>, Vec<f64>) {
    renderer.set_lighting_settings(direct_settings(restir));
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Direct,
        lamp: 0,
    });
    for _ in 0..warmup {
        let _ = grab(window, renderer, world, camera);
    }
    let mut frames = Vec::new();
    let mut distance = Vec::new();
    let shots = average.max(1);
    for _ in 0..shots {
        let read = grab(window, renderer, world, camera);
        if distance.is_empty() {
            distance.clone_from(&read.distance_m);
        }
        frames.push(luma_image(&read.rgb));
    }
    (mean_of(&frames), distance)
}

fn assert_flag_off(window: &mut Window, renderer: &mut Renderer, world: &World, camera: &Camera) {
    let _ = grab(window, renderer, world, camera);
    let names: Vec<_> = renderer
        .pipeline_times()
        .iter()
        .map(|(name, _)| name.clone())
        .collect();
    assert!(
        names.iter().all(|name| !name.contains("restir")),
        "the flag-off frame must not build a ReSTIR pipeline: {names:?}"
    );
}

fn assert_gate(window: &mut Window, renderer: &mut Renderer, world: &World, camera: &Camera) {
    let (brute, distance) = settle(window, renderer, world, camera, false, 2, 1);
    assert!(
        image_mean(&brute) > 1.0e-3,
        "the gate floor is lit, mean {}",
        image_mean(&brute)
    );
    let (live, _) = settle(window, renderer, world, camera, true, WARMUP, GATE_AVG);
    let gate_err = ref_err_near(&live, &brute, &distance);
    assert!(
        gate_err <= REF_ERR,
        "gate ref_err_near {gate_err:.4} above {REF_ERR}"
    );
    assert!(
        names_have_restir(renderer),
        "the flag-on frame builds restir-di and restir-direct"
    );
    let still_a = grab(window, renderer, world, camera);
    let still_b = grab(window, renderer, world, camera);
    let flicker = frame_step(&luma_image(&still_a.rgb), &luma_image(&still_b.rgb));
    assert!(
        flicker <= FRAME_STEP,
        "a still camera flickers by {flicker:.4}"
    );
}

fn assert_shadow(window: &mut Window, renderer: &mut Renderer, world: &World, camera: &Camera) {
    let (shadow_brute, shadow_distance) = settle(window, renderer, world, camera, false, 2, 1);
    let (shadow_live, _) = settle(window, renderer, world, camera, true, WARMUP, SHADOW_AVG);
    let shadow_err = ref_err_near(&shadow_live, &shadow_brute, &shadow_distance);
    assert!(
        shadow_err <= REF_ERR,
        "shadow ref_err_near {shadow_err:.4} above {REF_ERR}"
    );
}

fn assert_turn(window: &mut Window, renderer: &mut Renderer, world: &World, camera: &Camera) {
    let turned = pose(2.2, 2.4, 0.55);
    let (turned_brute, turned_distance) = settle(window, renderer, world, &turned, false, 2, 1);
    renderer.set_lighting_settings(direct_settings(true));
    let before = luma_image(&grab(window, renderer, world, camera).rgb);
    for _ in 0..WARMUP {
        let _ = grab(window, renderer, world, camera);
    }
    let first = luma_image(&grab(window, renderer, world, &turned).rgb);
    let stuck = ref_err_near(&before, &turned_brute, &turned_distance);
    let immediate = ref_err_near(&first, &turned_brute, &turned_distance);
    assert!(
        immediate < stuck * 0.75,
        "the turn kept the old picture: first {immediate:.4}, old-versus-new {stuck:.4}"
    );
    let (settled, _) = settle(window, renderer, world, &turned, true, WARMUP, GATE_AVG);
    let turned_err = ref_err_near(&settled, &turned_brute, &turned_distance);
    assert!(
        turned_err <= REF_ERR,
        "turned ref_err_near {turned_err:.4} above {REF_ERR}"
    );
}

#[test]
fn restir_di_matches_brute_direct_on_the_gate_scenes() {
    let (_gpu, mut window, mut renderer) = open();
    let gate = world_from(&RestirScene::gate(), false);
    let camera = pose(0.0, 4.0, 0.0);
    assert_flag_off(&mut window, &mut renderer, &gate, &camera);
    assert_gate(&mut window, &mut renderer, &gate, &camera);
    let shadow = world_from(&RestirScene::shadow(), true);
    assert_shadow(&mut window, &mut renderer, &shadow, &camera);
    assert_turn(&mut window, &mut renderer, &shadow, &camera);
}

fn names_have_restir(renderer: &Renderer) -> bool {
    let times = renderer.pipeline_times();
    times.iter().any(|(name, _)| name == "restir-di")
        && times.iter().any(|(name, _)| name == "restir-direct")
}
