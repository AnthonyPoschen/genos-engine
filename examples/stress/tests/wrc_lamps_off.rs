#![cfg_attr(
    test,
    expect(
        clippy::as_conversions,
        reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md"
    )
)]
#![cfg_attr(
    test,
    expect(
        clippy::cast_precision_loss,
        reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
    )
)]
#![cfg_attr(
    test,
    expect(clippy::expect_used, reason = "a test aborts when a fixture fails")
)]

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
    lamps_change_run(name, off, true);
}

/// Lit night, then `change`. With `black`, the frames until the picture is
/// under 1 % of lit (gated); otherwise the frames until it stays within 1 % of
/// lit of where it settles (the mean of its last 30 of 600 frames), reported.
/// No pop: the picture moves from the lit level toward where it ends up and
/// never past either by more than noise (1 % of lit).
fn assert_no_pop(name: &str, trail: &[f64], lit: f64, black: bool) {
    let end = if black {
        0.0
    } else {
        let last30 = &trail[trail.len() - 30..];
        last30.iter().sum::<f64>() / last30.len() as f64
    };
    let (lo, hi) = (lit.min(end) - 0.01 * lit, lit.max(end) + 0.01 * lit);
    let pop = trail.iter().position(|m| *m < lo || *m > hi);
    eprintln!("wrc_lamps_off {name}: range [{lo:.2}, {hi:.2}] first out at {pop:?}");
    assert!(
        pop.is_none(),
        "{name}: frame {:?} at {:.2} is outside [{lo:.2}, {hi:.2}] (a pop)",
        pop.map(|i| i + 1),
        pop.map_or(0.0, |i| trail[i])
    );
}

/// Frames until the picture stays within 1 % of lit of its settled level.
fn report_settle(name: &str, trail: &[f64], lit: f64) {
    let last30 = &trail[trail.len() - 30..];
    let level = last30.iter().sum::<f64>() / last30.len() as f64;
    let within = trail
        .iter()
        .rposition(|m| (m - level).abs() > 0.01 * lit)
        .map_or(0, |i| i + 1);
    eprintln!(
        "wrc_lamps_off {name}: lit {lit:.3} settled {level:.3} within 1 % of lit at frame {within} ({:.0} ms at 60 fps)",
        within as f64 * 1000.0 / 60.0
    );
}

/// One lit frame at rest: draining past `CONVERGED_BY` or unsettled past
/// `SETTLE_BY` counts as restless; `rest` spans the picture past `SETTLE_BY`.
fn note_rest(
    f: usize,
    lit: f64,
    settle: genos_render::SettleStats,
    restless: &mut Vec<usize>,
    rest: &mut (f64, f64),
) {
    if (f > CONVERGED_BY && settle.draining) || (f > SETTLE_BY && !settle.settled) {
        restless.push(f);
    }
    if f > SETTLE_BY {
        *rest = (rest.0.min(lit), rest.1.max(lit));
    }
}

fn assert_at_rest(name: &str, restless: &[usize], rest: (f64, f64)) {
    eprintln!(
        "wrc_lamps_off {name}: at rest past frame {SETTLE_BY}: {} draining or unsettled, picture drift {:.4}",
        restless.len(),
        (rest.1 - rest.0).max(0.0)
    );
    assert!(
        restless.is_empty(),
        "{name}: a still lit night is draining past frame {CONVERGED_BY} or not settled past frame {SETTLE_BY} at {} frames (first {:?}): the cache keeps relighting at rest",
        restless.len(),
        restless.first()
    );
}

/// How long the night stays lit before the change. The cache is in a
/// different state at each (still settling from power-on, just settled, long
/// settled), so every case runs at all three and must pass at each;
/// `WRC_LIT_FRAMES` runs just one.
const LIT_FRAMES: [usize; 3] = [250, 300, 420];

/// Lit frames by which a still lit night has settled: the surface cache's 300
/// still frames plus the drain hold and the stats ring's lag.
const SETTLE_BY: usize = 330;

/// Lit frames by which a still lit night has converged: its cache samples
/// match the held light, so the drain flag is down (it may not keep
/// re-arming on a few noisy cells and relight at rest).
const CONVERGED_BY: usize = 200;

fn lamps_change_run(name: &str, off: fn(&mut Stage), black: bool) {
    #[allow(clippy::disallowed_methods, reason = "a harness reads the environment")]
    let only: Option<usize> = std::env::var("WRC_LIT_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok());
    for lit_frames in only.map_or(LIT_FRAMES.to_vec(), |n| vec![n]) {
        lamps_change_once(&format!("{name}@{lit_frames}"), off, black, lit_frames);
    }
}

fn lamps_change_once(name: &str, off: fn(&mut Stage), black: bool, lit_frames: usize) {
    let mut window = Window::open_proof(WIDTH, HEIGHT).expect("proof window");
    let size = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, size.width, size.height).expect("renderer");
    let mut owner = genos_window::Lifecycle::new(window, renderer, permit);
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    // The stress default: preset high (surface cache, world radiance cache).
    renderer.set_lighting_settings(LightingSettings::preset(LightingPreset::High));
    let mix = LightMix {
        count: 32,
        dynamic_pct: 0,
        layout: Layout::Spread,
    };
    let mut stage = Stage::new(Scale::SMALL, 1, mix, 0.0, NIGHT, 120.0);
    stage.sun_frozen = true;
    stage.boxes_still = true;
    stage.lamps_still = true;
    stage.apply();
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(14.5, 1.7, 23.5), -0.3554, 0.08);
    let exposure = std::cell::Cell::new(0.0f64);
    let settle = std::cell::Cell::new(genos_render::SettleStats::default());
    let mut draw = |stage: &Stage| {
        let _ = window.pump();
        let m = mean(
            &renderer
                .draw(&stage.world, &camera, true)
                .expect("draw")
                .expect("pixels"),
        );
        exposure.set(renderer.hdr_exposure());
        settle.set(renderer.settle_stats());
        m
    };
    let mut never = 0.0;
    for _ in 0..120 {
        never = draw(&stage);
    }
    stage.power = 1.0;
    stage.apply();
    let mut lit = 0.0;
    // At rest: once the lit night has held still past the settle window the
    // cache must settle (stop relighting) and stay settled, as on master.
    let mut restless = Vec::new();
    let mut rest = (f64::MAX, f64::MIN);
    for f in 1..=lit_frames {
        lit = draw(&stage);
        note_rest(f, lit, settle.get(), &mut restless, &mut rest);
        if f % 100 == 0 {
            eprintln!(
                "wrc_lamps_off {name}: lit frame {f} mean {lit:.2} exposure {:.4}",
                exposure.get()
            );
        }
    }
    assert!(lit > 4.0, "the lit reference is lit: mean {lit:.2}");
    assert_at_rest(name, &restless, rest);
    off(&mut stage);
    let mut trail = Vec::new();
    let mut black_at = None;
    for f in 1..=600 {
        let m = draw(&stage);
        trail.push(m);
        if f % 100 == 0 {
            eprintln!("wrc_lamps_off {name}: off frame {f} mean {m:.2}");
        }
        if black && m < 0.01 * lit {
            black_at = Some(f);
            break;
        }
    }
    eprintln!(
        "wrc_lamps_off {name}: never-lit {never:.3} lit {lit:.3} off first frames {:?} black at {black_at:?}",
        trail
            .iter()
            .take(16)
            .map(|m| format!("{m:.2}"))
            .collect::<Vec<_>>()
    );
    assert_no_pop(name, &trail, lit, black);
    if !black {
        report_settle(name, &trail, lit);
        return;
    }
    assert!(
        never < 0.01 * lit,
        "never-lit night {never:.3} is not below 1 % of lit {lit:.3}"
    );
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

/// Lamp power halved, then doubled from lit: frames to settle, reported
/// (not gated).
#[test]
fn lamps_half_power_settles() {
    lamps_change_run(
        "half",
        |stage| {
            stage.power = 0.5;
            stage.apply();
        },
        false,
    );
}

#[test]
fn lamps_double_power_settles() {
    lamps_change_run(
        "double",
        |stage| {
            stage.power = 2.0;
            stage.apply();
        },
        false,
    );
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
    let mix = LightMix {
        count: 32,
        dynamic_pct: 0,
        layout: Layout::Spread,
    };
    let mut stage = Stage::new(Scale::SMALL, 1, mix, 0.0, NIGHT, 120.0);
    stage.sun_frozen = true;
    stage.boxes_still = true;
    stage.lamps_still = true;
    stage.power = 1.0;
    stage.apply();
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(14.5, 1.7, 23.5), -0.3554, 0.08);
    let mut setup = genos_debug::reference::RefSetup::from_camera(
        &stage.world.scene,
        &camera,
        WIDTH,
        HEIGHT,
        64,
    );
    setup.noise_target = 0.01;
    setup.max_spp = 16384;
    setup.seconds = 900.0;
    let r = genos_debug::reference::render(&setup);
    let sum: u64 = r.image.rgb.iter().map(|&v| u64::from(v)).sum();
    let level = sum as f64 / r.image.rgb.len().max(1) as f64;
    eprintln!(
        "wrc_lamps_off reference: lit {level:.3} spp {} noise {:.4} {:.0}s",
        r.spp, r.noise, r.seconds
    );
}
