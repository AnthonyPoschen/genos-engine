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
    hall_walls(n, Vec::new())
}

fn hall_walls(n: usize, walls: Vec<genos_scene::Wall>) -> World {
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
        walls,
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
    pose_at(0.0, 1.6, 3.0, yaw)
}

fn pose_at(x: f32, y: f32, z: f32, yaw: f32) -> Camera {
    let mut camera = Camera::opening();
    camera.set_pose(genos_scene::Vec3::new(x, y, z), yaw, 0.0);
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
    // Wiring lock for the constants and the diag owners. The numbers are
    // graded by `turn_reprojection` (live policy) and by the product draw
    // below. This source check does not accept the shader.
    let live = include_str!("../shaders/restir_live.comp");
    let direct = include_str!("../shaders/lighting_direct.comp");
    assert!(live.contains("const float RESTIR_DEPTH_REL = 0.02;"));
    assert!(live.contains("const float RESTIR_DEPTH_ABS = 0.05;"));
    assert!(live.contains("const float RESTIR_NORMAL_MIN = 0.9;"));
    assert!(live.contains("const uint RESTIR_TEMPORAL_TRIES = 1u;"));
    assert!(live.contains("const ivec2 RESTIR_TEMPORAL_OFFSETS[1]"));
    assert!(live.contains("const uint RESTIR_DIAG_HIST_ACCEPT = 5u;"));
    assert!(live.contains("const uint RESTIR_DIAG_HIST_REJECT = 6u;"));
    assert!(direct.contains("atomicAdd(gstats.diag[13u], 1u);"));
    assert!(!direct.contains("e = e_exact + e_mean"));
    assert!(!direct.contains("e = e_mean"));
    assert!(direct.contains("sample is the graded colour"));
}

/// Flag off builds no ReSTIR pipeline. Flag on, product shade, every lamp
/// count leaves `exact` at 0 and adds no near point lamp in the direct loop.
/// One, eight (the exact-sum count), and thirty-two lamps count a reservoir
/// sample. The labelled exact gate still sums and does not take that loop.
/// A still view keeps live history. A yaw keeps some and rejects some. A
/// coplanar lift keeps the floor. A facing wall rejects it. A still view and
/// a yaw both stay off the sum.
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
    let off_counts = renderer.lighting_lamp_pick_counts();
    assert_eq!(
        (off_counts.exact, off_counts.restir_point_adds, off_counts.history_accepts, off_counts.history_rejects),
        (0, 0, 0, 0),
        "flag off counted a ReSTIR shade ({off_counts:?})"
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
        assert_eq!(
            counts.restir_point_adds, 0,
            "{count} lamps added a near point lamp in the direct loop ({counts:?})"
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
    // The lamp list just changed, so this frame resets history.
    draw(window, renderer, &lit, &still);
    draw(window, renderer, &lit, &still);
    let held = renderer.lighting_lamp_pick_counts();
    eprintln!(
        "stable accept {} reject {} exact {} restir {} point_adds {}",
        held.history_accepts, held.history_rejects, held.exact, held.restir, held.restir_point_adds
    );
    assert_eq!(
        held.exact, 0,
        "a still view fell back to the exact sum ({held:?})"
    );
    assert_eq!(
        held.restir_point_adds, 0,
        "a still view added a near point lamp in the direct loop ({held:?})"
    );
    assert!(
        held.restir > 0,
        "a still view dropped the reservoir ({held:?})"
    );
    assert!(
        held.history_accepts > 0,
        "a still view kept no live history ({held:?})"
    );
    assert_eq!(
        held.history_rejects, 0,
        "a still view rejected live history ({held:?})"
    );

    let turned = pose(0.7);
    draw(window, renderer, &lit, &turned);
    let turn = renderer.lighting_lamp_pick_counts();
    eprintln!(
        "turn accept {} reject {} exact {} restir {} point_adds {}",
        turn.history_accepts, turn.history_rejects, turn.exact, turn.restir, turn.restir_point_adds
    );
    assert_eq!(
        turn.exact, 0,
        "a yaw fell back to the exact sum ({turn:?})"
    );
    assert_eq!(
        turn.restir_point_adds, 0,
        "a yaw added a near point lamp in the direct loop ({turn:?})"
    );
    assert!(
        turn.restir > 0,
        "a yaw dropped the reservoir ({turn:?})"
    );
    assert!(
        turn.history_accepts > 0,
        "a yaw kept no live history ({turn:?})"
    );
    assert!(
        turn.history_rejects > 0,
        "a yaw rejected no live history ({turn:?})"
    );
    assert!(
        turn.history_accepts < held.history_accepts,
        "a yaw kept as much history as the still view ({turn:?} vs {held:?})"
    );

    // Back to the still pose, then a 2 m lift. The lift is the only change
    // from the frame before it, so the rejection is the depth test.
    draw(window, renderer, &lit, &still);
    draw(window, renderer, &lit, &still);
    let lifted = pose_at(0.0, 3.6, 3.0, 0.0);
    draw(window, renderer, &lit, &lifted);
    let mismatch = renderer.lighting_lamp_pick_counts();
    eprintln!(
        "lift accept {} reject {} exact {} restir {} point_adds {}",
        mismatch.history_accepts,
        mismatch.history_rejects,
        mismatch.exact,
        mismatch.restir,
        mismatch.restir_point_adds
    );
    assert_eq!(
        mismatch.exact, 0,
        "a coplanar lift fell back to the exact sum ({mismatch:?})"
    );
    assert_eq!(
        mismatch.restir_point_adds, 0,
        "a coplanar lift added a near point lamp in the direct loop ({mismatch:?})"
    );
    // The live test compares the candidate ray to the stored depth. A lift that
    // stays on the floor plane keeps history. View-depth would reject it.
    assert!(
        mismatch.history_accepts > mismatch.history_rejects,
        "a coplanar lift rejected the floor plane ({mismatch:?})"
    );
    assert!(
        mismatch.history_rejects < held.history_accepts,
        "a coplanar lift rejected more pixels than the still view kept ({mismatch:?})"
    );

    // Same camera as the lift. The new wall faces the camera, so its normal
    // misses the floor history. That is the depth/normal rejection.
    let blocked = hall_walls(
        8,
        vec![genos_scene::Wall {
            position: genos_scene::Vec3::new(0.0, 0.0, 1.0),
            half_x: 4.0,
            half_z: 0.15,
            height: 3.0,
            base: 0.0,
            color: [0.62, 0.62, 0.62],
            absorption: 0.0,
            reflectance: 0.8,
            color_mix: -1.0,
        }],
    );
    draw(window, renderer, &blocked, &lifted);
    let surface = renderer.lighting_lamp_pick_counts();
    eprintln!(
        "wall accept {} reject {} exact {} restir {} point_adds {}",
        surface.history_accepts,
        surface.history_rejects,
        surface.exact,
        surface.restir,
        surface.restir_point_adds
    );
    assert_eq!(
        surface.exact, 0,
        "a wall mismatch fell back to the exact sum ({surface:?})"
    );
    assert_eq!(
        surface.restir_point_adds, 0,
        "a wall mismatch added a near point lamp in the direct loop ({surface:?})"
    );
    assert!(
        surface.history_rejects > surface.history_accepts,
        "a facing wall kept the floor history ({surface:?})"
    );
    assert!(
        surface.history_rejects > mismatch.history_rejects,
        "a facing wall rejected no more than the coplanar lift ({surface:?})"
    );

    renderer.set_lamp_shade_gate(LampShadeGate::Exact);
    draw(window, renderer, &lit, &still);
    let reference = renderer.lighting_lamp_pick_counts();
    eprintln!(
        "exact-gate exact {} restir {} point_adds {} accept {} reject {}",
        reference.exact,
        reference.restir,
        reference.restir_point_adds,
        reference.history_accepts,
        reference.history_rejects
    );
    assert!(
        reference.exact > 0,
        "the labelled exact gate did not sum ({reference:?})"
    );
    assert_eq!(
        reference.restir_point_adds, 0,
        "the labelled exact gate counted a ReSTIR point-lamp add ({reference:?})"
    );
    assert_eq!(
        reference.restir, 0,
        "the labelled exact gate shaded a reservoir sample ({reference:?})"
    );
    assert_eq!(
        (reference.history_accepts, reference.history_rejects),
        (0, 0),
        "the labelled exact gate ran live history ({reference:?})"
    );
}
