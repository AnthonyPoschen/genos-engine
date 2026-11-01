#![cfg_attr(test, expect(clippy::as_conversions, reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::cast_precision_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! Lamps-off night in the default surface cache (ADR 0037).
//!
//! A night scene whose lamps never lit must be black: below 1 % of the same
//! view lit. Switching the lamps off must reach that within 200 ms, 12 frames
//! at 60 fps. Multi-bounce light from the lit state is real but decays by the
//! albedo each bounce; a cache that holds it (gain per round near 1) fails.

use genos_render::Renderer;
use genos_scene::{Camera, LightingPreset, LightingSettings, Vec3};
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};
use genos_window::Window;

const WIDTH: u32 = 160;
const HEIGHT: u32 = 90;
/// 22:00 on the stage clock (0 is 06:00).
const NIGHT: f32 = 16.0 / 24.0;
const STEP_FRAMES: usize = 12;

fn mean(frame: &[u8]) -> f64 {
    let mut sum = 0u64;
    for px in frame.as_chunks::<4>().0 {
        sum += u64::from(px[0]) + u64::from(px[1]) + u64::from(px[2]);
    }
    sum as f64 / (frame.len() / 4 * 3).max(1) as f64
}

fn lamps_off_run(name: &str, off: fn(&mut Stage)) {
    let mut window = Window::open_proof(WIDTH, HEIGHT).expect("proof window");
    let size = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, size.width, size.height).expect("renderer");
    let mut owner = genos_window::Lifecycle::new(window, renderer, permit);
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    // The stress default: preset high (surface cache, world radiance cache).
    renderer.set_lighting_settings(LightingSettings::preset(LightingPreset::High));
    let mix = LightMix { count: 32, dynamic_pct: 0, layout: Layout::Spread };
    let mut stage = Stage::new(Scale::SMALL, 1, mix, 0.0, NIGHT, 120.0);
    stage.sun_frozen = true;
    stage.boxes_still = true;
    stage.lamps_still = true;
    stage.apply();
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(14.5, 1.7, 23.5), -0.3554, 0.08);
    let exposure = std::cell::Cell::new(0.0f64);
    let mut draw = |stage: &Stage| {
        let _ = window.pump();
        let m = mean(&renderer.draw(&stage.world, &camera, true).expect("draw").expect("pixels"));
        exposure.set(renderer.hdr_exposure());
        m
    };
    let mut never = 0.0;
    for _ in 0..120 {
        never = draw(&stage);
    }
    stage.power = 1.0;
    stage.apply();
    let mut lit = 0.0;
    #[allow(clippy::disallowed_methods, reason = "a harness reads the environment")]
    let lit_frames: usize = std::env::var("WRC_LIT_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(300);
    for f in 1..=lit_frames {
        lit = draw(&stage);
        if f % 100 == 0 {
            eprintln!("wrc_lamps_off {name}: lit frame {f} mean {lit:.2} exposure {:.4}", exposure.get());
        }
    }
    assert!(lit > 4.0, "the lit reference is lit: mean {lit:.2}");
    off(&mut stage);
    let mut trail = Vec::new();
    let mut black_at = None;
    for f in 1..=600 {
        let m = draw(&stage);
        trail.push(m);
        if f % 100 == 0 {
            eprintln!("wrc_lamps_off {name}: off frame {f} mean {m:.2}");
        }
        if m < 0.01 * lit {
            black_at = Some(f);
            break;
        }
    }
    eprintln!(
        "wrc_lamps_off {name}: never-lit {never:.3} lit {lit:.3} off first frames {:?} black at {black_at:?}",
        trail.iter().take(16).map(|m| format!("{m:.2}")).collect::<Vec<_>>()
    );
    assert!(never < 0.01 * lit, "never-lit night {never:.3} is not below 1 % of lit {lit:.3}");
    let frames = black_at.unwrap_or(usize::MAX);
    assert!(
        frames <= STEP_FRAMES,
        "lamps off took {frames} frames to reach 1 % of lit (window {STEP_FRAMES}), last {:.3}",
        trail.last().copied().unwrap_or(0.0)
    );
}

#[test]
fn lamps_off_night_goes_black() {
    lamps_off_run("power", |stage| {
        stage.power = 0.0;
        stage.apply();
    });
}

/// The same light leaving without a lamp-set change: every lamp moves far
/// below the stage, so the cache's lamp-set reset does not fire and only
/// the cache's own drain can take the bounce light out.
#[test]
fn lamps_moved_away_drain_without_a_reset() {
    lamps_off_run("moved", |stage| {
        for light in &mut stage.world.scene.lights {
            light.position.y -= 1000.0;
        }
    });
}

/// Ground truth for the lit level `lamps_off_run` reaches: the CPU path-traced
/// reference (unlimited bounces) of the same lit night, same pose and size,
/// toned like the picture. Minutes on a CPU, so ignored by default:
/// `--ignored --nocapture lit_reference_level`.
#[test]
#[ignore = "a CPU path trace, minutes"]
fn lit_reference_level() {
    let mix = LightMix { count: 32, dynamic_pct: 0, layout: Layout::Spread };
    let mut stage = Stage::new(Scale::SMALL, 1, mix, 0.0, NIGHT, 120.0);
    stage.sun_frozen = true;
    stage.boxes_still = true;
    stage.lamps_still = true;
    stage.power = 1.0;
    stage.apply();
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(14.5, 1.7, 23.5), -0.3554, 0.08);
    let mut setup = genos_debug::reference::RefSetup::from_camera(&stage.world.scene, &camera, WIDTH, HEIGHT, 64);
    setup.noise_target = 0.01;
    setup.max_spp = 16384;
    setup.seconds = 900.0;
    let r = genos_debug::reference::render(&setup);
    let sum: u64 = r.image.rgb.iter().map(|&v| u64::from(v)).sum();
    let level = sum as f64 / r.image.rgb.len().max(1) as f64;
    eprintln!("wrc_lamps_off reference: lit {level:.3} spp {} noise {:.4} {:.0}s", r.spp, r.noise, r.seconds);
}
