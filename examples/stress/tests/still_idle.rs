#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! A still High hall must not rerun the wavelet on texels that already have
//! 16 sets. That rebuild was the steady filter: five 5×5 kernels on the
//! whole screen list, every frame. The mean is applied on frame 32. A moving
//! camera does not take the hold.

use std::sync::Mutex;

use genos_render::Renderer;
use genos_scene::{Camera, LightingPreset, LightingSettings, Vec3};
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};
use genos_window::Window;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;

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

fn mean_abs(a: &[u8], b: &[u8]) -> f64 {
    let n = a.len().min(b.len()) / 4;
    if n == 0 {
        return 0.0;
    }
    let mut sum = 0.0f64;
    for i in 0..n {
        let p = i * 4;
        sum += f64::from(u32::from(a[p].abs_diff(b[p])));
        sum += f64::from(u32::from(a[p + 1].abs_diff(b[p + 1])));
        sum += f64::from(u32::from(a[p + 2].abs_diff(b[p + 2])));
    }
    sum / (3.0 * n as f64)
}

#[test]
fn a_still_hall_does_not_refilter_settled_texels() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    renderer.set_lighting_settings(LightingSettings::preset(LightingPreset::High));
    let (stage, camera) = hall();
    let mut early = Vec::new();
    let mut heavy = Vec::new();
    let mut walks = Vec::new();
    // Draw 31 is the 32-frame backstop (the frame counter starts at 1). The
    // draws after it are the hold: variance runs the texels that are not yet
    // settled, not the screen list.
    for i in 0..48 {
        let _ = window.pump();
        let pixels = renderer.draw(&stage.world, &camera, true).expect("draw").expect("pixels");
        if i == 2 {
            early = pixels.clone();
        }
        if i == 47 {
            let moved = mean_abs(&early, &pixels);
            eprintln!("still idle frame move {moved:.3} codes");
            assert!(moved > 0.5, "the picture froze at {moved:.3} codes");
        }
        if i >= 40 {
            let stats = renderer.lighting_atrous_stats();
            let sun = renderer.lighting_sun_stats();
            eprintln!(
                "still idle draw {i} heavy {} screen {} rebuilt {} appended {} blocks {} maint {} walks {}",
                stats.filter_heavy,
                stats.screen_pixels,
                stats.list_rebuilt,
                stats.list_appended,
                sun.blocks,
                u32::from(sun.maint_ran),
                stats.cache_walks
            );
            heavy.push(stats.filter_heavy);
            walks.push(stats.cache_walks);
            assert!(stats.screen_pixels > 0, "the screen wavelet published no pixels");
            assert_eq!(stats.list_rebuilt, 0, "a held camera rebuilt the screen list");
            assert!(!sun.maint_ran, "maintenance walked a held frame");
        }
    }
    let least_heavy = heavy.iter().copied().min().unwrap_or(u32::MAX);
    let least_walks = walks.iter().copied().min().unwrap_or(u32::MAX);
    let screen = WIDTH * HEIGHT;
    // A settled hall reruns a few dozen texels. Rerunning the list is about
    // ten thousand. Five percent of the picture is the line between those.
    assert!(
        least_heavy * 20 < screen,
        "the wavelet reran the picture: min heavy {least_heavy} of {screen} pixels"
    );
    // Direct flicker still marks most blocks (the print). Those blocks add the
    // new direct to the stored bounce. They do not walk the cache. Five
    // percent of the picture is the line.
    assert!(
        least_walks * 20 < screen,
        "compose walked the cache: min walks {least_walks} of {screen}"
    );
}
