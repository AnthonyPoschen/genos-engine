#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! A mover in one part of the hall unholds the tiles it can reach.
//! A lamp still unholds the whole picture, the same frame.

use std::sync::Mutex;

use genos_render::Renderer;
use genos_scene::{Camera, LightingPreset, LightingSettings, Shape, Solid, Vec3};
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

fn held_settings() -> LightingSettings {
    let mut settings = LightingSettings::preset(LightingPreset::High);
    // Screen-tile fill keeps spending rays on a held camera. A settled hold and
    // a picture-wide unhold then differ by the rays those frames just added.
    // This test measures the mover mask, so that budget is off.
    settings.surface_cache.fill.budget = 0;
    settings
}

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
    renderer.set_lighting_settings(held_settings());
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

struct Shot {
    px: Vec<u8>,
    mask: Vec<u32>,
    tiles: u32,
    walks: u32,
    rebuilt: u32,
    wide: bool,
}

fn tile_on(mask: &[u32], tile: u32) -> bool {
    let i = tile as usize;
    mask.get(i / 32)
        .is_some_and(|word| word & (1 << (i % 32)) != 0)
}

fn tile_mean_abs(a: &[u8], b: &[u8], tx: u32, ty: u32) -> f64 {
    let mut sum = 0.0f64;
    let mut n = 0u32;
    let y0 = ty * TILE;
    let x0 = tx * TILE;
    for y in y0..(y0 + TILE).min(HEIGHT) {
        for x in x0..(x0 + TILE).min(WIDTH) {
            let i = ((y * WIDTH + x) * 4) as usize;
            if i + 2 >= a.len() || i + 2 >= b.len() {
                continue;
            }
            sum += (f64::from(a[i]) - f64::from(b[i])).abs();
            sum += (f64::from(a[i + 1]) - f64::from(b[i + 1])).abs();
            sum += (f64::from(a[i + 2]) - f64::from(b[i + 2])).abs();
            n += 3;
        }
    }
    if n == 0 { 0.0 } else { sum / f64::from(n) }
}

/// Largest per-tile mean absolute code difference. `outside` uses tiles whose
/// mask bit is clear. A picture-wide frame compares every tile.
fn max_tile_diff(local: &Shot, control: &Shot, outside: bool) -> (f64, u32) {
    let tw = WIDTH.div_ceil(TILE);
    let th = HEIGHT.div_ceil(TILE);
    let mut worst = 0.0f64;
    let mut count = 0u32;
    for ty in 0..th {
        for tx in 0..tw {
            let tile = ty * tw + tx;
            if outside && !local.wide && tile_on(&local.mask, tile) {
                continue;
            }
            count += 1;
            worst = worst.max(tile_mean_abs(&local.px, &control.px, tx, ty));
        }
    }
    (worst, count)
}

fn box_at(stage: &mut Stage, x: f32, y: f32, z: f32, size: f32, height: f32) {
    let solid = stage
        .world
        .scene
        .solids
        .iter_mut()
        .find(|solid| solid.color[0] > 0.8 && solid.color[1] < 0.2)
        .expect("red box");
    solid.position = Vec3::new(x, y, z);
    solid.size = size;
    solid.height = height;
    solid.yaw = 0.0;
}

fn push_spread(stage: &mut Stage) {
    let spots = [
        (8.0, 8.0),
        (12.0, 10.0),
        (16.0, 12.0),
        (10.0, 16.0),
        (14.0, 18.0),
        (18.0, 20.0),
    ];
    for (i, (x, z)) in spots.into_iter().enumerate() {
        stage.world.scene.solids.push(Solid {
            shape: Shape::Square,
            position: Vec3::new(x, 0.0, z),
            size: 2.5,
            height: 2.0,
            yaw: 0.0,
            color: [0.2, 0.2, 0.2 + i as f32 * 0.05],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        });
    }
}

fn apply_frame(stage: &mut Stage, camera: &mut Camera, frame: u32, spread: &mut bool) {
    let base = Vec3::new(14.5, 1.7, 23.5);
    camera.set_pose(base, -0.3554, 0.08);
    if frame < 36 {
        box_at(stage, 3.0, 0.4, 4.0, 1.0, 1.0);
        return;
    }
    if frame < 60 {
        let step = if frame < 44 { (frame - 35) as f32 } else { 8.0 };
        let x = 3.0 + 0.12 * step;
        box_at(stage, x, 0.4, 4.0, 1.0, 1.0);
        return;
    }
    if frame < 66 {
        box_at(stage, 14.5, 0.4, 32.0, 1.0, 1.0);
        return;
    }
    if frame < 72 {
        let x = 12.0 + 0.2 * (frame - 66) as f32;
        box_at(stage, x, 0.0, 14.0, 8.0, 6.0);
        return;
    }
    if frame < 78 {
        if !*spread {
            push_spread(stage);
            *spread = true;
        }
        let n = stage.world.scene.solids.len();
        let shift = 0.15 * (frame - 71) as f32;
        for solid in stage
            .world
            .scene
            .solids
            .iter_mut()
            .skip(n.saturating_sub(6))
        {
            solid.position.x += shift;
        }
        return;
    }
    box_at(stage, 3.0 + 0.2 * (frame - 77) as f32, 0.4, 4.0, 1.0, 1.0);
    camera.set_pose(Vec3::new(14.5 + 0.35, 1.7, 23.5), -0.3554, 0.08);
}

fn capture(unhold: bool) -> Vec<Shot> {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    renderer.set_lighting_settings(held_settings());
    renderer.lighting_picture_unhold(unhold);
    let (mut stage, mut camera) = hall();
    let mut spread = false;
    let mut shots = Vec::new();
    for frame in 0..82u32 {
        apply_frame(&mut stage, &mut camera, frame, &mut spread);
        let _ = window.pump();
        let px = renderer
            .draw(&stage.world, &camera, true)
            .expect("draw")
            .expect("pixels");
        let stats = renderer.lighting_atrous_stats();
        let mask = renderer.lighting_reach_mask();
        shots.push(Shot {
            px,
            mask,
            tiles: stats.reach_tiles,
            walks: stats.cache_walks,
            rebuilt: stats.list_rebuilt,
            wide: stats.reach_wide,
        });
    }
    shots
}

/// Far shadow and bounce outside a corner mover's disc, then the cases where
/// the disc is not a proof: an off-screen mover, a large mover, many movers,
/// and a camera move with a mover. Each frame's tiles outside the mask match
/// a second run that unholds the whole picture. The stop tail fails if the
/// mask clears while that difference is still there, and if the mask is still
/// marked once the box has been still.
#[test]
fn outside_the_mask_matches_a_picture_wide_unhold() {
    let control = capture(true);
    let local = capture(false);
    let tile_count = WIDTH.div_ceil(TILE) * HEIGHT.div_ceil(TILE);
    let still = max_tile_diff(&local[35], &control[35], false);
    eprintln!(
        "reach control still max tile diff {:.3} over {} tiles",
        still.0, still.1
    );
    // Two lavapipe runs of the same held picture differ by a couple of codes
    // in a tile. A stale shadow is much larger.
    assert!(
        still.0 < 6.0,
        "two settled runs already differ by {:.3} codes",
        still.0
    );

    let mut local_disc = false;
    for frame in 40..44 {
        let (diff, n) = max_tile_diff(&local[frame], &control[frame], true);
        eprintln!(
            "reach corner {frame} tiles {} wide {} rebuilt {} walks {} outside {n} max {diff:.3}",
            local[frame].tiles, local[frame].wide, local[frame].rebuilt, local[frame].walks
        );
        assert_eq!(local[frame].rebuilt, 0, "a mover rebuilt the screen list");
        assert!(
            diff < 8.0,
            "stale bounce outside the mask, frame {frame}, {diff:.3} codes"
        );
        if !local[frame].wide && local[frame].tiles > 0 && local[frame].tiles * 2 < tile_count {
            local_disc = true;
            assert!(n > 0, "a local mask had no outside tile to compare");
        }
    }
    assert!(local_disc, "the corner box never stayed a local disc");

    // Stop tail. The box is still from frame 44. Clearing the mask while the
    // picture still differs from the unhold is an early clear. A mask that is
    // still marked at frame 59, after the picture has settled, is perpetual hot.
    for frame in 44..47 {
        let (diff, _) = max_tile_diff(&local[frame], &control[frame], true);
        let cleared = local[frame].tiles == 0 && !local[frame].wide;
        eprintln!(
            "reach tail-early {frame} tiles {} wide {} cleared {cleared} diff {diff:.3}",
            local[frame].tiles, local[frame].wide
        );
        if cleared {
            assert!(
                diff < 8.0,
                "early clear: the mask dropped while the picture still differed ({diff:.3})"
            );
        }
    }
    let tail = &local[59];
    let (tail_diff, _) = max_tile_diff(tail, &control[59], true);
    eprintln!(
        "reach tail-stop tiles {} wide {} diff {tail_diff:.3}",
        tail.tiles, tail.wide
    );
    assert!(
        tail.tiles == 0 && !tail.wide,
        "perpetual hot: the mask stayed marked after the box stopped (tiles {} wide {})",
        tail.tiles, tail.wide
    );
    assert!(
        tail_diff < 8.0,
        "the stopped picture differs from the unhold by {tail_diff:.3}"
    );

    for (label, frames, want_wide) in [
        ("offscreen", 62..66, true),
        ("large", 68..72, true),
        ("many", 74..78, true),
    ] {
        for frame in frames {
            let (diff, _) = max_tile_diff(&local[frame], &control[frame], true);
            eprintln!(
                "reach {label} {frame} tiles {} wide {} walks {} diff {diff:.3}",
                local[frame].tiles, local[frame].wide, local[frame].walks
            );
            assert!(diff < 12.0, "{label} stale outside the mask ({diff:.3})");
            if want_wide {
                assert!(
                    local[frame].wide || local[frame].tiles * 2 >= tile_count,
                    "{label} stayed a small disc (tiles {} of {tile_count})",
                    local[frame].tiles
                );
            }
        }
    }
    for frame in 78..82 {
        let (diff, _) = max_tile_diff(&local[frame], &control[frame], false);
        eprintln!(
            "reach camera {frame} tiles {} wide {} walks {} diff {diff:.3}",
            local[frame].tiles, local[frame].wide, local[frame].walks
        );
        assert!(
            local[frame].walks > 1000,
            "a camera move with a mover held the bounce (walks {})",
            local[frame].walks
        );
        assert!(
            diff < 8.0,
            "camera+mover differs from the unhold by {diff:.3}"
        );
    }
}
