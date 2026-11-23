#![cfg_attr(
    test,
    expect(clippy::expect_used, reason = "a test aborts when a fixture fails")
)]

//! A lit bounce region has no cell-sized black square.
//!
//! A hole is a near-black patch a few pixels across whose ring, about one
//! cell out, is already lit. A shadow fails that test: its ring is dark too.
//! A frame that is black everywhere fails it as well, because the ring is
//! black. The hole is a cell that has no data while a neighbour does.

use std::sync::Mutex;

use genos_render::{linear_from_display, luminance, DebugView, Renderer, ViewMode};
use genos_scene::{Camera, Vec3};
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};
use genos_window::Window;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
/// Linear luminance. A display byte near 16 sits under this.
const DARK: f64 = 0.004;
/// A lit neighbour. A display byte near 40 sits over this.
const LIT: f64 = 0.02;
const CORE: i32 = 4;
const RING_IN: i32 = 10;
const RING_OUT: i32 = 14;

fn open() -> (MutexGuard, genos_window::Lifecycle<Renderer>) {
    static GPU: Mutex<()> = Mutex::new(());
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

type MutexGuard = std::sync::MutexGuard<'static, ()>;

fn hall(boxes_still: bool) -> (Stage, Camera) {
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
    stage.boxes_still = boxes_still;
    stage.lamps_still = true;
    stage.apply();
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(14.5, 1.7, 23.5), -0.3554, 0.08);
    (stage, camera)
}

fn draw(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &genos_render::World,
    camera: &Camera,
) -> Vec<u8> {
    let _ = window.pump();
    renderer
        .draw(world, camera, true)
        .expect("draw")
        .expect("presented image")
}

fn show(renderer: &mut Renderer) {
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Bounce,
        lamp: 0,
    });
}

fn luma_of(bgra: &[u8]) -> Vec<f64> {
    let width = usize::try_from(WIDTH).expect("width");
    let height = usize::try_from(HEIGHT).expect("height");
    let mut out = vec![0.0; width * height];
    let (pixels, _) = bgra.as_chunks::<4>();
    for (index, px) in pixels.iter().take(out.len()).enumerate() {
        let red = linear_from_display(f32::from(px[2]) / 255.0);
        let green = linear_from_display(f32::from(px[1]) / 255.0);
        let blue = linear_from_display(f32::from(px[0]) / 255.0);
        out[index] = f64::from(luminance([red, green, blue]));
    }
    out
}

fn at(luma: &[f64], width: usize, x: i32, y: i32) -> f64 {
    let ux = usize::try_from(x).expect("x");
    let uy = usize::try_from(y).expect("y");
    luma[uy * width + ux]
}

/// Cell-sized black squares in a region that is already lit.
fn holes(luma: &[f64]) -> u32 {
    let width = usize::try_from(WIDTH).expect("width");
    let height = usize::try_from(HEIGHT).expect("height");
    let mut count = 0u32;
    let x0 = RING_OUT;
    let y0 = RING_OUT;
    let x1 = i32::try_from(width).expect("width") - RING_OUT;
    let y1 = i32::try_from(height).expect("height") - RING_OUT;
    for y in y0..y1 {
        for x in x0..x1 {
            if at(luma, width, x, y) >= DARK {
                continue;
            }
            let mut core_n = 0i32;
            let mut core_dark = 0i32;
            for dy in -CORE..=CORE {
                for dx in -CORE..=CORE {
                    core_n += 1;
                    if at(luma, width, x + dx, y + dy) < DARK {
                        core_dark += 1;
                    }
                }
            }
            if core_dark * 2 < core_n {
                continue;
            }
            let mut ring_n = 0i32;
            let mut ring_lit = 0i32;
            for dy in -RING_OUT..=RING_OUT {
                for dx in -RING_OUT..=RING_OUT {
                    let r2 = dx * dx + dy * dy;
                    if !(RING_IN * RING_IN..=RING_OUT * RING_OUT).contains(&r2) {
                        continue;
                    }
                    ring_n += 1;
                    if at(luma, width, x + dx, y + dy) > LIT {
                        ring_lit += 1;
                    }
                }
            }
            if ring_n == 0 || ring_lit * 2 <= ring_n {
                continue;
            }
            // A cell is a small dark patch. The edge of a whole unlit
            // surface is dark across this window, so it is not a cell.
            let mut window_n = 0i32;
            let mut window_dark = 0i32;
            for dy in -12..=12 {
                for dx in -12..=12 {
                    window_n += 1;
                    if at(luma, width, x + dx, y + dy) < DARK {
                        window_dark += 1;
                    }
                }
            }
            if window_dark * 5 <= window_n {
                count += 1;
            }
        }
    }
    count
}

fn frame_holes(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &genos_render::World,
    camera: &Camera,
) -> (u32, f64) {
    let picture = draw(window, renderer, world, camera);
    let luma = luma_of(&picture);
    let mean = luma.iter().sum::<f64>() / f64::from(u32::try_from(luma.len()).expect("len"));
    (holes(&luma), mean)
}

#[test]
fn a_lit_region_has_no_cell_sized_black_square() {
    let (_guard, mut life) = open();
    let window = &mut *life.window;
    let renderer = &mut *life.device;
    renderer.apply_lighting_preset("high").expect("high preset");
    show(renderer);

    renderer.reset_lighting_cache();
    let (stage, mut camera) = hall(true);
    let mut saw_light = false;
    for frame in 0..8 {
        let (n, mean) = frame_holes(window, renderer, &stage.world, &camera);
        if mean > LIT {
            saw_light = true;
        }
        assert!(
            !saw_light || n == 0,
            "first visibility frame {frame} has {n} black-square pixels, mean {mean:.4}"
        );
    }
    assert!(saw_light, "the hall never lit");

    for _ in 0..12 {
        let _ = draw(window, renderer, &stage.world, &camera);
    }
    camera.set_pose(Vec3::new(14.5, 1.7, 23.5), -0.3554 + 0.55, 0.08);
    for frame in 0..4 {
        let (n, mean) = frame_holes(window, renderer, &stage.world, &camera);
        assert!(
            n == 0,
            "camera turn frame {frame} has {n} black-square pixels, mean {mean:.4}"
        );
    }

    renderer.reset_lighting_cache();
    let (moving, camera) = hall(false);
    saw_light = false;
    for frame in 0..10 {
        let (n, mean) = frame_holes(window, renderer, &moving.world, &camera);
        if mean > LIT {
            saw_light = true;
        }
        assert!(
            !saw_light || n == 0,
            "mover frame {frame} has {n} black-square pixels, mean {mean:.4}"
        );
    }
    assert!(saw_light, "the moving hall never lit");
}

/// A settled hall, then a moving camera. A new voxel has no ray of its own
/// yet. The same frame must publish its ancestor, so no visible surface is
/// left with an empty sample.
#[test]
fn a_moving_camera_has_no_empty_voxel() {
    let (_guard, mut life) = open();
    let window = &mut *life.window;
    let renderer = &mut *life.device;
    renderer.apply_lighting_preset("high").expect("high preset");
    show(renderer);
    renderer.reset_lighting_cache();
    let (stage, mut camera) = hall(true);
    let mut saw_light = false;
    for frame in 0..24 {
        let picture = draw(window, renderer, &stage.world, &camera);
        let luma = luma_of(&picture);
        let mean = luma.iter().sum::<f64>() / f64::from(u32::try_from(luma.len()).expect("len"));
        if mean > LIT {
            saw_light = true;
        }
        if frame + 1 == 24 {
            let gaps = renderer.lighting_cascade_gaps();
            assert!(
                gaps == 0,
                "settled frame has {gaps} visible surfaces with no data, mean {mean:.4}"
            );
        }
    }
    assert!(saw_light, "the hall never lit");
    let mut yaw = -0.3554;
    for frame in 0..8 {
        yaw += 0.12;
        camera.set_pose(Vec3::new(14.5, 1.7, 23.5), yaw, 0.08);
        let _ = draw(window, renderer, &stage.world, &camera);
        let gaps = renderer.lighting_cascade_gaps();
        assert!(
            gaps == 0,
            "motion frame {frame} has {gaps} visible surfaces with no data"
        );
    }
}
