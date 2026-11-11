//! A moving occluder must not leave the direct-sample mean on the old visibility.
//!
//! The floor reprojects and no lamp changes, so a long mean would keep the
//! shadow that was there before the box moved. The bar is the new shadow
//! within 10% of its settled reference by frame 8. History accepts stay
//! high: the mean drops because the sample changed, not because reprojection
//! failed.

#![cfg_attr(test, expect(clippy::as_conversions, reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::cast_precision_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::disallowed_methods, reason = "TODO(lint-2026-10-14): publish intent with the CLI argument, not set_var. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

use genos_render::{DebugView, Renderer, ViewMode, World};
use genos_scene::{AmbientMode, Camera, LightingPreset, LightingSettings, Scene};
use genos_window::Window;

const WARM: usize = 24;
const TAIL: usize = 12;
const WIDTH: u32 = 160;
const HEIGHT: u32 = 90;

struct LampDiag;

impl Drop for LampDiag {
    fn drop(&mut self) {
        std::env::remove_var("GENOS_LIGHTING_LAMP_PICK_DIAG");
    }
}

fn open() -> (
    std::sync::MutexGuard<'static, ()>,
    genos_window::Lifecycle<Renderer>,
) {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = GPU
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut window = Window::open_proof(WIDTH, HEIGHT).expect("proof window");
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height)
            .expect("renderer");
    (
        guard,
        genos_window::Lifecycle::new(window, renderer, permit),
    )
}

fn settings() -> LightingSettings {
    let mut settings = LightingSettings::preset(LightingPreset::High);
    settings.direct.refresh_frames = 1;
    settings.direct.resolution_scale = 1;
    settings.sky = false;
    settings.ambient.mode = AmbientMode::Off;
    settings.shadows.enabled = true;
    settings.gi.enabled = true;
    settings.bounce.count = 1;
    settings.probes.enabled = false;
    // The GI bit, and the last camera the mean reprojects with, exist only
    // when the world cache is allocated. Cache off leaves every frame as one
    // fresh sample, so a shadow would snap for the wrong reason.
    settings.world_cache.enabled = true;
    settings.denoiser.enabled = false;
    settings.denoiser.screen = false;
    settings.compose_reuse = false;
    settings.restir_di = true;
    settings
}

fn solid(x: f32) -> genos_scene::Solid {
    genos_scene::Solid {
        shape: genos_scene::Shape::Square,
        position: genos_scene::Vec3::new(x, 0.0, 0.35),
        size: 0.75,
        height: 1.9,
        yaw: 0.0,
        color: [0.85, 0.12, 0.1],
        absorption: 0.0,
        reflectance: 0.35,
        color_mix: -1.0,
    }
}

fn lamp(x: f32) -> genos_scene::Light {
    genos_scene::Light {
        position: genos_scene::Vec3::new(x, 2.7, 0.35),
        color: [5.0, 4.6, 4.0],
        direction: genos_scene::Vec3::ZERO,
    }
}

fn world(lamp_x: f32, box_x: f32) -> World {
    World::from_scene(Scene {
        floor: genos_scene::Floor {
            position: genos_scene::Vec3::ZERO,
            half_x: 8.0,
            half_z: 8.0,
            color: [0.72, 0.72, 0.72],
        },
        walls: Vec::new(),
        solids: vec![solid(box_x)],
        lights: vec![lamp(lamp_x)],
        ceiling: Some(genos_scene::Ceiling {
            height: 3.4,
            color: [0.45, 0.45, 0.45],
        }),
        sky: None,
        lighting: settings(),
        debug: genos_scene::DebugSettings::default(),
        fps_counter: genos_scene::FpsCounterSettings::default(),
        event_loop: genos_scene::EventLoopSettings::default(),
        reflections: genos_scene::ReflectionSettings::default(),
        hdr: genos_scene::HdrSettings::legacy_bytes(),
        fog: genos_scene::FogSettings::default(),
        menu: genos_scene::MenuSettings::default(),
    })
}

fn camera(yaw: f32) -> Camera {
    let mut camera = Camera::opening();
    camera.set_pose(genos_scene::Vec3::new(0.0, 0.85, 2.7), yaw, -0.42);
    camera
}

fn draw(window: &mut Window, renderer: &mut Renderer, world: &World, camera: &Camera) -> Vec<u8> {
    let _ = window.pump();
    renderer
        .draw(world, camera, true)
        .expect("draw")
        .expect("readback")
}

fn settle(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
    n: usize,
) -> Vec<u8> {
    let mut frame = Vec::new();
    for _ in 0..n {
        frame = draw(window, renderer, world, camera);
    }
    frame
}

fn luma_bgra(px: &[u8]) -> f64 {
    0.2126 * f64::from(px[2]) + 0.7152 * f64::from(px[1]) + 0.0722 * f64::from(px[0])
}

fn chroma(px: &[u8]) -> u8 {
    let mx = px[0].max(px[1]).max(px[2]);
    let mn = px[0].min(px[1]).min(px[2]);
    mx - mn
}

/// Floor pixels the move actually relit. The box itself is saturated and is
/// left out: those pixels fail reprojection and would hide a stale shadow.
fn shadow_ids(open: &[u8], closed: &[u8]) -> Vec<usize> {
    let n = open.len().min(closed.len()) / 4;
    let mut ids = Vec::new();
    for i in 0..n {
        let a = &open[i * 4..i * 4 + 4];
        let b = &closed[i * 4..i * 4 + 4];
        if chroma(a) > 40 || chroma(b) > 40 {
            continue;
        }
        if (luma_bgra(a) - luma_bgra(b)).abs() >= 12.0 {
            ids.push(i);
        }
    }
    ids
}

fn region_mean(frame: &[u8], ids: &[usize]) -> f64 {
    if ids.is_empty() {
        return 0.0;
    }
    let sum: f64 = ids
        .iter()
        .map(|&i| luma_bgra(&frame[i * 4..i * 4 + 4]))
        .sum();
    sum / ids.len() as f64
}

fn channel_mean(frame: &[u8], ch: usize) -> f64 {
    let n = frame.len() / 4;
    if n == 0 {
        return 0.0;
    }
    let sum: u64 = (0..n).map(|i| u64::from(frame[i * 4 + ch])).sum();
    sum as f64 / n as f64
}

fn whole_mean(frame: &[u8]) -> f64 {
    let n = frame.len() / 4;
    if n == 0 {
        return 0.0;
    }
    let sum: f64 = (0..n)
        .map(|i| luma_bgra(&frame[i * 4..i * 4 + 4]))
        .sum();
    sum / n as f64
}

/// Share of the open-to-closed step still left. 0 is the closed reference.
fn step_left(live: f64, open: f64, closed: f64) -> f64 {
    let span = (open - closed).abs().max(1.0);
    (live - closed).abs() / span
}

fn curve(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &mut World,
    camera: &Camera,
    ids: &[usize],
    open_mean: f64,
    closed_mean: f64,
) -> Vec<f64> {
    let mut out = Vec::with_capacity(TAIL);
    for _ in 0..TAIL {
        let frame = draw(window, renderer, world, camera);
        out.push(step_left(
            region_mean(&frame, ids),
            open_mean,
            closed_mean,
        ));
    }
    out
}

/// The shader's 12-bit log luma. One code is `2^(1/64)`.
fn log_code(luma: f32) -> u32 {
    if luma.partial_cmp(&1.0e-8) != Some(core::cmp::Ordering::Greater) {
        return 0;
    }
    let q = (luma.log2() + 24.0) * 64.0;
    let rounded = (q + 0.5).floor().clamp(1.0, 4095.0);
    let mut code = 0_u32;
    let mut step = 0.0_f32;
    while step < rounded && code < 4095 {
        code += 1;
        step += 1.0;
    }
    code
}

#[test]
fn a_log_step_is_a_different_integrand() {
    let center = 2.0_f32.powf(2000.0 / 64.0 - 24.0);
    assert_eq!(log_code(center), 2000);
    let step = 2.0_f32.powf(1.0 / 64.0);
    assert!(step < 1.02, "one code is about 1.1%, not a ten percent band");
    assert_eq!(log_code(center * step), 2001);
    assert_ne!(log_code(center * 1.10), 2000);
    assert_eq!(log_code(0.0), 0);
}

fn frames_until(errs: &[f64]) -> String {
    errs.iter()
        .position(|e| *e <= 0.10)
        .map_or_else(|| format!(">{TAIL}"), |i| format!("{}", i + 1))
}

fn lamp_step(
    window: &mut Window,
    renderer: &mut Renderer,
    home: &Camera,
    aside: &Camera,
) -> (Vec<f64>, usize) {
    let at_b = world(1.6, 4.0);
    let ref_b = settle(window, renderer, &at_b, home, WARM);
    let _ = draw(window, renderer, &at_b, aside);
    let mut at_a = world(0.0, 4.0);
    let ref_a = settle(window, renderer, &at_a, home, WARM);
    let ids = shadow_ids(&ref_a, &ref_b);
    let a_mean = region_mean(&ref_a, &ids);
    let b_mean = region_mean(&ref_b, &ids);
    at_a.scene.lights[0].position.x = 1.6;
    let errs = curve(window, renderer, &mut at_a, home, &ids, a_mean, b_mean);
    report("lamp", &errs, a_mean, b_mean, ids.len());
    (errs, ids.len())
}

fn report(label: &str, errs: &[f64], open_mean: f64, closed_mean: f64, pixels: usize) {
    let list = errs
        .iter()
        .enumerate()
        .map(|(i, e)| format!("{}:{e:.3}", i + 1))
        .collect::<Vec<_>>()
        .join(" ");
    eprintln!(
        "{label} px={pixels} open={open_mean:.2} closed={closed_mean:.2} until10={until} {list}",
        until = frames_until(errs)
    );
}

fn lock_the_drop_before_the_cap() {
    let direct = include_str!("../shaders/lighting_direct.comp");
    let pick = include_str!("../shaders/lighting_lamp_pick.glsl");
    assert!(
        !pick.contains("RESTIR_MEAN_TARGET_REL"),
        "a ratio band keeps a different integrand in the mean"
    );
    assert!(pick.contains("return restir_mean_quant(luma) != stored_q;"));
    assert!(direct.contains("lamp_pick_reproject_at"));
    assert!(direct.contains("mean_dropped && !restir_sampled"));
    assert!(direct.contains("restir_mean_target_moved"));
    let drop_at = direct
        .find("restir_mean_target_moved")
        .expect("stored-sample drop");
    let cap_at = direct
        .find("mean_prev >= RESTIR_MEAN_CAP")
        .expect("mean cap");
    assert!(
        drop_at < cap_at,
        "a frozen mean still has to drop when the stored sample changes"
    );
}

#[test]
fn a_moved_box_shadow_is_within_ten_percent_by_frame_eight() {
    lock_the_drop_before_the_cap();
    // Header words 43 and 44 stay zero unless this bit is on, so the
    // reprojection bar would pass a run that never counted an accept.
    let _diag = LampDiag;
    std::env::set_var("GENOS_LIGHTING_LAMP_PICK_DIAG", "1");
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    renderer.set_lighting_settings(settings());
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Direct,
        lamp: 0,
    });
    let home = camera(0.0);
    let aside = camera(1.4);

    let blocked = world(0.0, 0.0);
    let ref_blocked = settle(window, renderer, &blocked, &home, WARM);
    let _ = draw(window, renderer, &blocked, &aside);

    let mut open = world(0.0, 4.0);
    let ref_open = settle(window, renderer, &open, &home, WARM);
    let accepts = renderer.lighting_lamp_pick_counts().history_accepts;
    eprintln!(
        "warmup accepts={accepts} restir={} open_luma={:.2} blocked_luma={:.2} open_g={:.1} blocked_g={:.1}",
        renderer.lighting_lamp_pick_counts().restir,
        whole_mean(&ref_open),
        whole_mean(&ref_blocked),
        channel_mean(&ref_open, 1),
        channel_mean(&ref_blocked, 1)
    );
    assert!(
        accepts > 64,
        "reprojection never kept history, so a passing shadow would be one fresh sample: accepts={accepts}"
    );

    let ids = shadow_ids(&ref_open, &ref_blocked);
    let open_mean = region_mean(&ref_open, &ids);
    let blocked_mean = region_mean(&ref_blocked, &ids);
    // `open` still has the box aside. Move that same scene so the camera,
    // the lamp, and the floor match the frames that built the open mean.
    open.scene.solids[0].position.x = 0.0;
    let box_errs = curve(
        window,
        renderer,
        &mut open,
        &home,
        &ids,
        open_mean,
        blocked_mean,
    );
    let accepts_after = renderer.lighting_lamp_pick_counts().history_accepts;
    report("box", &box_errs, open_mean, blocked_mean, ids.len());
    eprintln!("box move still reprojects accepts={accepts_after}");

    // Lamp position only. The box stays aside so the change is the lamp.
    let (lamp_errs, lamp_pixels) = lamp_step(window, renderer, &home, &aside);

    assert!(
        ids.len() >= 40,
        "the box did not move a floor shadow ({} pixels, open {:.1} blocked {:.1})",
        ids.len(),
        whole_mean(&ref_open),
        whole_mean(&ref_blocked)
    );
    // Unfixed, frame 8 was still 0.794 of the open picture and frame 12 was
    // 0.791, with reprojection still accepting. The cap would have kept that.
    assert!(
        box_errs.len() >= 8 && box_errs[7] <= 0.10,
        "frame 8 of the moved shadow is still {:.3} of the step",
        box_errs.get(7).copied().unwrap_or(1.0)
    );
    assert!(
        accepts_after > 64,
        "the shadow snap killed reprojection (accepts={accepts_after})"
    );
    assert!(
        lamp_pixels >= 40,
        "moving the lamp did not relight the floor ({lamp_pixels} pixels)"
    );
    assert!(
        lamp_errs.len() >= 8 && lamp_errs[7] <= 0.10,
        "frame 8 after the lamp moved is still {:.3} of the step",
        lamp_errs.get(7).copied().unwrap_or(1.0)
    );
}
