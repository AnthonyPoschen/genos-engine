#![cfg_attr(test, expect(clippy::disallowed_methods, reason = "TODO(lint-2026-10-14): publish intent with the CLI argument, not set_var. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::doc_markdown, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! Product wiring for `restir_di`. The picture gates stay in `restir_di.rs`.
//!
//! This file checks the flag, the pipelines, and the shade counters. It does
//! not loosen `REF_ERR` or `FRAME_STEP`.

use genos_render::{DebugView, LampShadeGate, Renderer, ViewMode, World};
use genos_scene::{AmbientMode, Camera, LampPickMode, LightingPreset, LightingSettings, Scene};
use genos_window::Window;

fn open() -> (
    std::sync::MutexGuard<'static, ()>,
    genos_window::Lifecycle<Renderer>,
) {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = GPU
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut window = Window::open_proof(64, 36).expect("proof window");
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height)
            .expect("renderer");
    (
        guard,
        genos_window::Lifecycle::new(window, renderer, permit),
    )
}

struct LampDiag;

impl Drop for LampDiag {
    fn drop(&mut self) {
        std::env::remove_var("GENOS_LIGHTING_LAMP_PICK_DIAG");
    }
}

fn product(restir: bool) -> LightingSettings {
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
    settings.lamp_pick.mode = LampPickMode::Reuse;
    settings.lamp_pick.radius = 0;
    settings.lamp_pick.unbiased = true;
    settings.compose_reuse = false;
    settings.restir_di = restir;
    settings
}

fn hall(n: usize) -> World {
    let mut lights = Vec::new();
    for i in 0..n {
        let i_f = f32::from(u16::try_from(i).unwrap_or(0));
        let n_f = f32::from(u16::try_from(n).unwrap_or(0));
        let x = (i_f - (n_f - 1.0) * 0.5) * 0.8;
        lights.push(genos_scene::Light {
            position: genos_scene::Vec3::new(x, 2.2, -1.0),
            color: [4.0, 4.0, 4.0],
            direction: genos_scene::Vec3::ZERO,
        });
    }
    World::from_scene(Scene {
        floor: genos_scene::Floor {
            position: genos_scene::Vec3::ZERO,
            half_x: 8.0,
            half_z: 8.0,
            color: [0.7, 0.7, 0.7],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights,
        ceiling: None,
        sky: None,
        lighting: product(false),
        debug: genos_scene::DebugSettings::default(),
        fps_counter: genos_scene::FpsCounterSettings::default(),
        event_loop: genos_scene::EventLoopSettings::default(),
        reflections: genos_scene::ReflectionSettings::default(),
        hdr: genos_scene::HdrSettings::for_preset(LightingPreset::High),
        fog: genos_scene::FogSettings::default(),
        menu: genos_scene::MenuSettings::default(),
    })
}

fn pose(yaw: f32) -> Camera {
    let mut camera = Camera::opening();
    camera.set_pose(genos_scene::Vec3::new(0.0, 1.6, 3.0), yaw, 0.0);
    camera
}

fn draw(window: &mut Window, renderer: &mut Renderer, world: &World, camera: &Camera) {
    let _ = window.pump();
    renderer
        .draw(world, camera, true)
        .expect("draw")
        .expect("readback");
}

fn names(renderer: &Renderer) -> Vec<String> {
    renderer
        .pipeline_times()
        .iter()
        .map(|(name, _)| name.clone())
        .collect()
}

#[test]
fn restir_live_keeps_the_product_history_test_and_the_raw_sample() {
    let live = include_str!("../shaders/restir_live.comp");
    let direct = include_str!("../shaders/lighting_direct.comp");
    assert!(live.contains("const float RESTIR_DEPTH_REL = 0.02;"));
    assert!(live.contains("const float RESTIR_DEPTH_ABS = 0.05;"));
    assert!(live.contains("const float RESTIR_NORMAL_MIN = 0.9;"));
    assert!(live.contains("const uint RESTIR_TEMPORAL_TRIES = 1u;"));
    assert!(live.contains("const ivec2 RESTIR_TEMPORAL_OFFSETS[1]"));
    assert!(!direct.contains("e = e_exact + e_mean"));
    assert!(!direct.contains("e = e_mean"));
    assert!(direct.contains("sample is the graded colour"));
}

/// Flag off builds no ReSTIR pipeline. Flag on, product shade, every lamp
/// count leaves `exact` at 0. One, eight, and thirty-two lamps count a
/// reservoir sample. The labelled exact gate still sums. A still view and a
/// hard turn both stay off that sum.
#[test]
fn restir_di_product_shades_every_lamp_count_without_the_exact_sum() {
    let _diag = LampDiag;
    std::env::set_var("GENOS_LIGHTING_LAMP_PICK_DIAG", "1");
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Direct,
        ..DebugView::default()
    });
    let camera = pose(0.0);
    let dark = hall(0);
    renderer.set_lighting_settings(product(false));
    draw(window, renderer, &dark, &camera);
    let off = names(renderer);
    assert!(
        off.iter().all(|name| !name.contains("restir")),
        "flag off built a ReSTIR pipeline: {off:?}"
    );

    renderer.set_lighting_settings(product(true));
    for count in [0usize, 1, 8, 32] {
        let world = hall(count);
        draw(window, renderer, &world, &camera);
        draw(window, renderer, &world, &camera);
        let counts = renderer.lighting_lamp_pick_counts();
        assert_eq!(
            counts.exact, 0,
            "{count} lamps took the exact sum ({counts:?})"
        );
        if count > 0 {
            assert!(
                counts.restir > 0,
                "{count} lamps did not shade a reservoir sample ({counts:?})"
            );
        }
    }
    let built = names(renderer);
    assert!(
        built.iter().any(|name| name == "restir-di")
            && built.iter().any(|name| name == "restir-direct"),
        "flag on did not build restir-di and restir-direct: {built:?}"
    );

    let lit = hall(8);
    let still = pose(0.0);
    draw(window, renderer, &lit, &still);
    let held = renderer.lighting_lamp_pick_counts();
    assert_eq!(
        held.exact, 0,
        "a still view fell back to the exact sum ({held:?})"
    );
    assert!(
        held.restir > 0,
        "a still view dropped the reservoir ({held:?})"
    );
    let turned = pose(1.4);
    draw(window, renderer, &lit, &turned);
    let rejected = renderer.lighting_lamp_pick_counts();
    assert_eq!(
        rejected.exact, 0,
        "a hard turn fell back to the exact sum ({rejected:?})"
    );
    assert!(
        rejected.restir > 0,
        "a hard turn dropped the reservoir ({rejected:?})"
    );

    renderer.set_lamp_shade_gate(LampShadeGate::Exact);
    draw(window, renderer, &lit, &still);
    let reference = renderer.lighting_lamp_pick_counts();
    assert!(
        reference.exact > 0,
        "the labelled exact gate did not sum ({reference:?})"
    );
}
