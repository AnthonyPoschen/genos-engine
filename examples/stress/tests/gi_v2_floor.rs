//! Still stress hall against gi-v2, not against this branch's hash path.
//!
//! Slot claims race across processes, so two gi-v2 runs of the same scene are
//! the noise floor. This process must land inside that floor. Set
//! `GENOS_PICTURE_DUMP` to write one RGBA8 frame (the gi-v2 worktree does
//! this). Set `GENOS_GI_V2_A` and `GENOS_GI_V2_B` to those two dumps, and
//! `GENOS_PICTURE_PRESET` to `medium` or `high`. Without the variables the
//! test does not draw.

use std::sync::Mutex;

use genos_render::Renderer;
use genos_scene::{Camera, LightingPreset, LightingSettings, Vec3};
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};
use genos_window::Window;

const WIDTH: u32 = 960;
const HEIGHT: u32 = 540;

fn open() -> (MutexGuard, Window, Renderer) {
    static GPU: Mutex<()> = Mutex::new(());
    let guard = GPU.lock().unwrap_or_else(|err| err.into_inner());
    let mut window = Window::open_proof(WIDTH, HEIGHT).expect("proof window");
    let frame = window.pump();
    let renderer = Renderer::open(window.display, window.surface, frame.width, frame.height)
        .expect("renderer");
    (guard, window, renderer)
}

type MutexGuard = std::sync::MutexGuard<'static, ()>;

fn hall_frame(preset_name: &str) -> Vec<u8> {
    let preset = LightingPreset::parse(preset_name)
        .unwrap_or_else(|| panic!("preset is medium or high, got {preset_name}"));
    let (_gpu, mut window, mut renderer) = open();
    let settings = LightingSettings::preset(preset);
    assert!(
        !settings.denoiser.screen,
        "{preset_name} must stay on the hash launch"
    );
    renderer.set_lighting_settings(settings);
    let mut stage = Stage::new(
        Scale::SMALL,
        1,
        LightMix {
            count: 5,
            dynamic_pct: 50,
            layout: Layout::Spread,
        },
        1.0,
        0.25,
        120.0,
    );
    stage.sun_frozen = true;
    stage.boxes_still = true;
    stage.lamps_still = true;
    stage.apply();
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(14.5, 1.7, 23.5), -0.3554, 0.08);
    let mut frame = Vec::new();
    for _ in 0..9 {
        let _ = window.pump();
        frame = renderer
            .draw(&stage.world, &camera, true)
            .expect("draw")
            .expect("pixels");
    }
    frame
}

fn pixel_errors(reference: &[u8], image: &[u8]) -> Vec<f64> {
    let n = reference.len().min(image.len()) / 4;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let p = i * 4;
        let e = (u32::from(reference[p].abs_diff(image[p]))
            + u32::from(reference[p + 1].abs_diff(image[p + 1]))
            + u32::from(reference[p + 2].abs_diff(image[p + 2]))) as f64
            / 3.0;
        out.push(e);
    }
    out
}

fn mean_of(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.iter().sum::<f64>() / v.len() as f64
}

fn percentile(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let i = ((v.len() - 1) as f64 * p).round() as usize;
    v[i.min(v.len() - 1)]
}

fn mean_p99(reference: &[u8], image: &[u8]) -> (f64, f64) {
    let mut errors = pixel_errors(reference, image);
    let mean = mean_of(&errors);
    let p99 = percentile(&mut errors, 0.99);
    (mean, p99)
}

#[test]
fn hash_launch_matches_gi_v2_within_cross_process_noise() {
    let dump = std::env::var("GENOS_PICTURE_DUMP").ok();
    let preset = std::env::var("GENOS_PICTURE_PRESET").unwrap_or_else(|_| "medium".into());
    if let Some(path) = dump {
        let frame = hall_frame(&preset);
        std::fs::write(&path, &frame).unwrap_or_else(|err| panic!("write {path}: {err}"));
        eprintln!("wrote {} bytes preset {preset} to {path}", frame.len());
        return;
    }
    let Ok(path_a) = std::env::var("GENOS_GI_V2_A") else {
        eprintln!("gi-v2 floor skipped: set GENOS_GI_V2_A and GENOS_GI_V2_B");
        return;
    };
    let path_b = std::env::var("GENOS_GI_V2_B").expect("GENOS_GI_V2_B");
    let a = std::fs::read(&path_a).unwrap_or_else(|err| panic!("read {path_a}: {err}"));
    let b = std::fs::read(&path_b).unwrap_or_else(|err| panic!("read {path_b}: {err}"));
    let ours = hall_frame(&preset);
    let (cross_mean, cross_p99) = mean_p99(&a, &b);
    let (mean_a, p99_a) = mean_p99(&a, &ours);
    let (mean_b, p99_b) = mean_p99(&b, &ours);
    let (mean, p99) = if p99_a <= p99_b {
        (mean_a, p99_a)
    } else {
        (mean_b, p99_b)
    };
    let limit = cross_p99 + 1.0 / 3.0;
    eprintln!(
        "gi-v2 {preset} cross {cross_mean:.3}/{cross_p99:.3} vs A {mean_a:.3}/{p99_a:.3} vs B {mean_b:.3}/{p99_b:.3} closer {mean:.3}/{p99:.3} limit {limit:.3}"
    );
    assert!(
        p99 <= limit + 1.0e-9,
        "{preset} p99 {p99:.3} is above the gi-v2 cross-process floor {cross_p99:.3}"
    );
    assert!(
        mean <= cross_mean + 0.25,
        "{preset} mean {mean:.3} is above the gi-v2 cross-process mean {cross_mean:.3}"
    );
}
