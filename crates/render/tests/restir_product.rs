#![cfg_attr(test, expect(clippy::disallowed_methods, reason = "TODO(lint-2026-10-14): publish intent with the CLI argument, not set_var. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::doc_markdown, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! Product wiring for `restir_di`. The picture gates stay in `restir_di.rs`.
//!
//! This file checks the flag, the pipelines, and the shade counters. It does
//! not loosen `REF_ERR` or `FRAME_STEP`.

use genos_render::{DebugView, LampPickCounts, LampShadeGate, Renderer, ViewMode, World};
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
    // Wiring lock for the constants and the counter words. Source text is
    // not SPIR-V, and it does not grade a pixel. `turn_reprojection` grades
    // the CPU mirror of the live policy. The product draw below shows the
    // counters move. Neither is a per-pixel depth or normal threshold.
    let live = include_str!("../shaders/restir_live.comp");
    let direct = include_str!("../shaders/lighting_direct.comp");
    assert!(live.contains("const float RESTIR_DEPTH_REL = 0.02;"));
    assert!(live.contains("const float RESTIR_DEPTH_ABS = 0.05;"));
    assert!(live.contains("const float RESTIR_NORMAL_MIN = 0.9;"));
    assert!(live.contains("const uint RESTIR_TEMPORAL_TRIES = 1u;"));
    assert!(live.contains("const ivec2 RESTIR_TEMPORAL_OFFSETS[1]"));
    assert!(live.contains("const uint RESTIR_DIAG_HIST_ACCEPT = 43u;"));
    assert!(live.contains("const uint RESTIR_DIAG_HIST_REJECT = 44u;"));
    assert!(live.contains("atomicAdd(restir_words[RESTIR_DIAG_HIST_ACCEPT], 1u);"));
    assert!(direct.contains("const uint RESTIR_DIAG_POINT_ADD = 45u;"));
    assert!(direct.contains("atomicAdd(restir_words[RESTIR_DIAG_POINT_ADD], 1u);"));
    assert!(!direct.contains("atomicAdd(gstats.diag[13u], 1u);"));
    assert!(!direct.contains("e = e_exact + e_mean"));
    assert!(!direct.contains("e = e_mean"));
    assert!(direct.contains("sample is the graded colour"));
}

fn assert_quiet(counts: &LampPickCounts, what: &str) {
    assert_eq!(
        counts.exact, 0,
        "{what} fell back to the exact sum ({counts:?})"
    );
    assert_eq!(
        counts.restir_point_adds, 0,
        "{what} added a near point lamp in the direct loop ({counts:?})"
    );
}

fn assert_flag_off(window: &mut Window, renderer: &mut Renderer, camera: &Camera) {
    let lit = hall(4);
    renderer.set_lighting_settings(product(false));
    draw(window, renderer, &lit, camera);
    let turned = pose(0.35);
    draw(window, renderer, &lit, &turned);
    let off = names(renderer);
    assert!(
        off.iter().all(|name| !name.contains("restir")),
        "the exact reference built a ReSTIR pipeline: {off:?}"
    );
    let off_counts = renderer.lighting_lamp_pick_counts();
    assert!(
        off_counts.exact > 0,
        "restir_di off did not shade the exact sum ({off_counts:?})"
    );
    assert_eq!(
        (
            off_counts.restir,
            off_counts.restir_point_adds,
            off_counts.history_accepts,
            off_counts.history_rejects
        ),
        (0, 0, 0, 0),
        "restir_di off counted a ReSTIR shade ({off_counts:?})"
    );
}

fn assert_lamp_counts(window: &mut Window, renderer: &mut Renderer, camera: &Camera) {
    renderer.set_lighting_settings(product(true));
    for count in [0usize, 1, 8, 32] {
        let world = hall(count);
        draw(window, renderer, &world, camera);
        draw(window, renderer, &world, camera);
        let counts = renderer.lighting_lamp_pick_counts();
        assert_quiet(&counts, &format!("{count} lamps"));
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
}

fn assert_still(
    window: &mut Window,
    renderer: &mut Renderer,
    lit: &World,
    still: &Camera,
) -> LampPickCounts {
    // The lamp list just changed, so this frame resets history.
    draw(window, renderer, lit, still);
    draw(window, renderer, lit, still);
    let held = renderer.lighting_lamp_pick_counts();
    eprintln!(
        "stable accept {} reject {} exact {} restir {} point_adds {}",
        held.history_accepts, held.history_rejects, held.exact, held.restir, held.restir_point_adds
    );
    assert_quiet(&held, "a still view");
    assert!(held.restir > 0, "a still view dropped the reservoir ({held:?})");
    assert!(
        held.history_accepts > 0,
        "a still view kept no live history ({held:?})"
    );
    assert_eq!(
        held.history_rejects, 0,
        "a still view rejected live history ({held:?})"
    );
    held
}

fn assert_yaw(
    window: &mut Window,
    renderer: &mut Renderer,
    lit: &World,
    held: &LampPickCounts,
) {
    let turned = pose(0.7);
    draw(window, renderer, lit, &turned);
    let turn = renderer.lighting_lamp_pick_counts();
    eprintln!(
        "turn accept {} reject {} exact {} restir {} point_adds {}",
        turn.history_accepts, turn.history_rejects, turn.exact, turn.restir, turn.restir_point_adds
    );
    assert_quiet(&turn, "a yaw");
    assert!(turn.restir > 0, "a yaw dropped the reservoir ({turn:?})");
    assert!(turn.history_accepts > 0, "a yaw kept no live history ({turn:?})");
    assert!(
        turn.history_rejects > 0,
        "a yaw rejected no live history ({turn:?})"
    );
    assert!(
        turn.history_accepts < held.history_accepts,
        "a yaw kept as much history as the still view ({turn:?} vs {held:?})"
    );
}

fn assert_lift(
    window: &mut Window,
    renderer: &mut Renderer,
    lit: &World,
    still: &Camera,
    held: &LampPickCounts,
) -> LampPickCounts {
    // Same floor, camera 2 m higher. The assert is accepts > rejects.
    // The reject count is not identified as depth, off-screen, or empty.
    draw(window, renderer, lit, still);
    draw(window, renderer, lit, still);
    let lifted = pose_at(0.0, 3.6, 3.0, 0.0);
    draw(window, renderer, lit, &lifted);
    let mismatch = renderer.lighting_lamp_pick_counts();
    eprintln!(
        "lift accept {} reject {} exact {} restir {} point_adds {}",
        mismatch.history_accepts,
        mismatch.history_rejects,
        mismatch.exact,
        mismatch.restir,
        mismatch.restir_point_adds
    );
    assert_quiet(&mismatch, "a coplanar lift");
    assert!(
        mismatch.history_accepts > mismatch.history_rejects,
        "a coplanar lift kept no more history than it rejected ({mismatch:?})"
    );
    assert!(
        mismatch.history_rejects < held.history_accepts,
        "a coplanar lift rejected more pixels than the still view kept ({mismatch:?})"
    );
    mismatch
}

fn assert_wall(window: &mut Window, renderer: &mut Renderer, mismatch: &LampPickCounts) {
    // Depth and normal change together. The lamps are behind the face, so
    // the reservoir sample count is 0. That is the fixture.
    let lifted = pose_at(0.0, 3.6, 3.0, 0.0);
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
    assert_quiet(&surface, "a wall mismatch");
    assert!(
        surface.history_rejects > surface.history_accepts,
        "a facing wall kept at least as much history as it rejected ({surface:?})"
    );
    assert_eq!(
        surface.restir, 0,
        "a facing wall shaded a reservoir sample; the lamps sit behind that face ({surface:?})"
    );
    assert!(
        surface.history_rejects > mismatch.history_rejects,
        "a facing wall rejected no more than the coplanar lift ({surface:?})"
    );
}

fn assert_exact_gate(window: &mut Window, renderer: &mut Renderer, lit: &World, still: &Camera) {
    renderer.set_lamp_shade_gate(LampShadeGate::Exact);
    draw(window, renderer, lit, still);
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

/// `restir_di` off is the labelled exact sum and builds no ReSTIR pipeline.
/// Flag on, product shade, every lamp
/// count leaves `exact` at 0 and adds no near point lamp in the direct loop.
/// One, eight (the exact-sum count), and thirty-two lamps count a reservoir
/// sample. The labelled exact gate still sums and does not take that loop.
/// A still view keeps live history. A yaw keeps some and rejects some. A
/// coplanar lift keeps more surfaced pixels than it rejects. A facing wall
/// rejects more than it keeps, and the lamps sit behind that face so the
/// reservoir sample count is 0. Those rows are counter movement. They are
/// not a depth-only or normal-only threshold.
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
    assert_flag_off(window, renderer, &camera);
    assert_lamp_counts(window, renderer, &camera);
    let lit = hall(8);
    let still = pose(0.0);
    let held = assert_still(window, renderer, &lit, &still);
    assert_yaw(window, renderer, &lit, &held);
    let mismatch = assert_lift(window, renderer, &lit, &still, &held);
    assert_wall(window, renderer, &mismatch);
    assert_exact_gate(window, renderer, &lit, &still);
}

/// GI on and probes on, so the quiet-view hold is armed. Rays stay small.
fn quiet_restir(mode: LampPickMode) -> LightingSettings {
    let mut settings = LightingSettings::preset(LightingPreset::Gi1Bounce);
    settings.lamp_pick.mode = mode;
    settings.probes.rays = 4;
    settings.sky = false;
    settings.ambient.mode = AmbientMode::Off;
    settings.denoiser.enabled = false;
    settings.prefill = false;
    settings.restir_di = true;
    settings.direct.max_lights = 4;
    settings
}

/// `gi-1bounce` is mode Hits. A quiet frame used to return before the reservoir.
#[test]
fn gi_1bounce_quiet_frames_read_the_reservoir() {
    let _diag = LampDiag;
    std::env::set_var("GENOS_LIGHTING_LAMP_PICK_DIAG", "1");
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Direct,
        ..DebugView::default()
    });
    let world = hall(8);
    let camera = pose(0.0);
    renderer.set_lighting_settings(quiet_restir(LampPickMode::Hits));
    draw(window, renderer, &world, &camera);
    let first = renderer.lighting_lamp_pick_counts();
    assert!(
        first.restir > 0,
        "the first gi-1bounce frame did not read the reservoir ({first:?})"
    );
    draw(window, renderer, &world, &camera);
    draw(window, renderer, &world, &camera);
    let quiet = renderer.lighting_lamp_pick_counts();
    eprintln!(
        "gi-1bounce quiet exact {} restir {}",
        quiet.exact, quiet.restir
    );
    assert_eq!(
        quiet.exact, 0,
        "a quiet gi-1bounce frame shaded the exact sum ({quiet:?})"
    );
    assert!(
        quiet.restir > 0,
        "a quiet gi-1bounce frame skipped the reservoir ({quiet:?})"
    );
}

/// A forced prepare failure is the labelled exact sum. The next reservoir
/// frame does not keep that hold.
#[test]
fn a_failed_restir_prepare_is_labelled_and_does_not_hold() {
    let _diag = LampDiag;
    std::env::set_var("GENOS_LIGHTING_LAMP_PICK_DIAG", "1");
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Direct,
        ..DebugView::default()
    });
    let world = hall(8);
    let camera = pose(0.0);
    // Mode reuse writes the 32-frame exact record. GI and probes arm the hold.
    renderer.set_lighting_settings(quiet_restir(LampPickMode::Reuse));
    draw(window, renderer, &world, &camera);
    draw(window, renderer, &world, &camera);
    let live = renderer.lighting_lamp_pick_counts();
    assert!(live.restir > 0, "the reservoir did not shade ({live:?})");
    assert_eq!(
        renderer.lighting_restir_prepare_failures(),
        0,
        "a live frame counted a prepare failure"
    );
    renderer.force_restir_prepare_failure(true);
    draw(window, renderer, &world, &camera);
    let failed = renderer.lighting_lamp_pick_counts();
    eprintln!(
        "restir prepare fail exact {} restir {} failures {}",
        failed.exact,
        failed.restir,
        renderer.lighting_restir_prepare_failures()
    );
    assert!(
        renderer.lighting_restir_prepare_failures() >= 1,
        "the forced failure was not counted"
    );
    assert!(
        failed.exact > 0,
        "the failed frame did not shade the labelled exact sum ({failed:?})"
    );
    assert_eq!(
        failed.restir, 0,
        "the failed frame still counted a reservoir sample ({failed:?})"
    );
    renderer.force_restir_prepare_failure(false);
    draw(window, renderer, &world, &camera);
    let again = renderer.lighting_lamp_pick_counts();
    eprintln!(
        "restir after fail exact {} restir {}",
        again.exact, again.restir
    );
    assert_eq!(
        again.exact, 0,
        "the exact hold survived into a restir frame ({again:?})"
    );
    assert!(
        again.restir > 0,
        "the frame after the failure did not read the reservoir ({again:?})"
    );
}
