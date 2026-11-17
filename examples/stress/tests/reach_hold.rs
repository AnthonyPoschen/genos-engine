#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! A mover in one part of the hall unholds the tiles it can reach.
//! A lamp still unholds the whole picture, the same frame.

use std::sync::Mutex;

use genos_render::Renderer;
use genos_scene::{Camera, LightingPreset, LightingSettings, Vec3};
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};
use genos_window::Window;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const TILE: u32 = 32;

fn open() -> (MutexGuard, genos_window::Lifecycle<Renderer>) {
    static GPU: Mutex<()> = Mutex::new(());
    let guard = GPU.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut window = Window::open_proof(WIDTH, HEIGHT).expect("proof window");
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height).expect("renderer");
    (guard, genos_window::Lifecycle::new(window, renderer, permit))
}

type MutexGuard = std::sync::MutexGuard<'static, ()>;

fn hall() -> (Stage, Camera) {
    let mut stage = Stage::new(
        Scale::SMALL,
        1,
        LightMix {
            count: 5,
            dynamic_pct: 0,
            layout: Layout::Spread,
        },
        1.0,
        0.12,
        120.0,
    );
    stage.sun_frozen = true;
    stage.boxes_still = true;
    stage.lamps_still = true;
    stage.apply();
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(14.5, 1.7, 23.5), -0.3554, 0.08);
    (stage, camera)
}

fn place_far(stage: &mut Stage, x: f32) {
    let solid = stage
        .world
        .scene
        .solids
        .iter_mut()
        .find(|solid| solid.color[0] > 0.8 && solid.color[1] < 0.2)
        .expect("red box");
    solid.position = Vec3::new(x, 0.4, 4.0);
    solid.yaw = 0.0;
}

#[derive(Clone, Copy, Debug)]
struct Sample {
    heavy: u32,
    walks: u32,
    rebuilt: u32,
    tiles: u32,
    hot: u32,
    corners: u32,
}

fn draw_sample(window: &mut genos_window::Window, renderer: &mut Renderer, stage: &Stage, camera: &Camera) -> Sample {
    let _ = window.pump();
    renderer.draw(&stage.world, camera, true).expect("draw").expect("pixels");
    let stats = renderer.lighting_atrous_stats();
    Sample {
        heavy: stats.filter_heavy,
        walks: stats.cache_walks,
        rebuilt: stats.list_rebuilt,
        tiles: stats.reach_tiles,
        hot: stats.reach_hot,
        corners: stats.reach_corners,
    }
}

#[test]
fn a_far_mover_leaves_the_other_corners_held() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    renderer.set_lighting_settings(LightingSettings::preset(LightingPreset::High));
    let (mut stage, camera) = hall();
    place_far(&mut stage, 3.0);
    let mut still = Sample {
        heavy: 0,
        walks: 0,
        rebuilt: 1,
        tiles: 1,
        hot: 1,
        corners: 0,
    };
    for i in 0..40 {
        let sample = draw_sample(window, renderer, &stage, &camera);
        if i == 39 {
            still = sample;
        }
    }
    let screen = WIDTH * HEIGHT;
    let tile_count = WIDTH.div_ceil(TILE) * HEIGHT.div_ceil(TILE);
    eprintln!(
        "reach still heavy {} walks {} rebuilt {} tiles {} hot {} corners {}",
        still.heavy, still.walks, still.rebuilt, still.tiles, still.hot, still.corners
    );
    assert_eq!(still.rebuilt, 0, "a held camera rebuilt the screen list");
    assert!(still.heavy * 20 < screen, "still heavy {} of {screen}", still.heavy);
    assert_eq!(still.tiles, 0, "a still picture marked mover tiles");

    let nudges = [0.12_f32, 0.24, 0.36, 0.48, 0.60, 0.72, 0.84, 0.96];
    let mut moving = Vec::new();
    for x in nudges {
        place_far(&mut stage, 3.0 + x);
        moving.push(draw_sample(window, renderer, &stage, &camera));
    }
    eprintln!("reach moving {moving:?}");
    let late = &moving[4..];
    assert!(
        late.iter().all(|s| s.rebuilt == 0),
        "a mover rebuilt the screen list"
    );
    assert!(
        late.iter().all(|s| s.tiles > 0 && s.tiles * 2 < tile_count),
        "mover tiles were none or the whole picture (tile count {tile_count})"
    );
    assert!(
        late.iter().all(|s| s.corners == 0),
        "a corner tile was unheld"
    );
    // The wavelet also runs the taps around those tiles. That ring is still
    // a fraction of the list: a picture-wide rerun is about one heavy slot
    // per pixel.
    assert!(
        late.iter().all(|s| s.heavy * 3 < screen),
        "the wavelet reran the picture"
    );

    stage.power = 0.0;
    stage.apply();
    let mut lamp = Vec::new();
    for _ in 0..4 {
        lamp.push(draw_sample(window, renderer, &stage, &camera));
    }
    eprintln!("reach lamp {lamp:?}");
    let lamp_walks = lamp.iter().map(|s| s.walks).max().unwrap_or(0);
    assert!(
        lamp_walks > 1000,
        "a lamp toggle left the bounce held (walks {lamp_walks})"
    );
}
