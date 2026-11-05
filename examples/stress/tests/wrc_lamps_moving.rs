#![cfg_attr(
    test,
    expect(clippy::expect_used, reason = "a test aborts when a fixture fails")
)]

//! Lamps that never stop moving in the default surface cache: the cache's
//! per-round work. A lamp change settles in a burst (more relights a round);
//! a scene that keeps changing must not pay that burst forever. Counts the
//! cache round's batch entries (16 rays each) and fails when the mean past the
//! burst exceeds the plain relight rate by more than `MARGIN`.

use genos_render::Renderer;
use genos_scene::{Camera, LightingPreset, LightingSettings, Vec3};
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};
use genos_window::Window;

const WIDTH: u32 = 160;
const HEIGHT: u32 = 90;
/// 22:00 on the stage clock (0 is 06:00).
const NIGHT: f32 = 16.0 / 24.0;
/// Frames the lamps move; the count starts after `SKIP` (the burst and the
/// stats ring's lag).
const MOVING_FRAMES: usize = 300;
const SKIP: usize = 60;
/// The plain (not settling) round of this scene: master's rate, measured with
/// settling off (`LIGHTING_SETTLE_ROUNDS` 0) on lavapipe, in batch entries.
/// Uncapped settling measured 65 793.
const PLAIN_ENTRIES: f64 = 19_896.0;
/// Allowed share over the plain rate.
const MARGIN: f64 = 0.10;

#[test]
fn moving_lamps_cost_the_plain_rate() {
    let mut window = Window::open_proof(WIDTH, HEIGHT).expect("proof window");
    let size = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, size.width, size.height).expect("renderer");
    let mut owner = genos_window::Lifecycle::new(window, renderer, permit);
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
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
    stage.power = 1.0;
    stage.apply();
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(14.5, 1.7, 23.5), -0.3554, 0.08);
    let mut entries = Vec::new();
    for f in 0..MOVING_FRAMES {
        // A small sway every frame: each frame is a lamp change.
        let dx = 0.02 * if f % 40 < 20 { 1.0 } else { -1.0 };
        for light in &mut stage.world.scene.lights {
            light.position.x += dx;
        }
        let _ = window.pump();
        renderer
            .draw(&stage.world, &camera, true)
            .expect("draw")
            .expect("pixels");
        if f >= SKIP {
            entries.push(f64::from(renderer.settle_stats().round_entries));
        }
    }
    let mean =
        entries.iter().sum::<f64>() / f64::from(u32::try_from(entries.len()).expect("count"));
    let max = entries.iter().copied().fold(0.0, f64::max);
    eprintln!(
        "wrc_lamps_moving: entries a round past frame {SKIP}: mean {mean:.0} max {max:.0} ({:.2} M rays mean), plain {PLAIN_ENTRIES:.0}",
        mean * 16.0 / 1.0e6
    );
    assert!(
        mean <= PLAIN_ENTRIES * (1.0 + MARGIN),
        "moving lamps cost {mean:.0} entries a round, over the plain {PLAIN_ENTRIES:.0} by more than {:.0} %",
        MARGIN * 100.0
    );
}
