#![expect(clippy::expect_used, reason = "a test aborts when a fixture fails")]

//! Lavapipe froxel against the CPU column integral.
//!
//! `run_fog` is the inject and integrate dispatches. The frame test draws a
//! Low-preset picture with fog and without it. The composed bytes must differ,
//! and the frame's own column buffer must match the CPU integral.
//! Times are lavapipe timestamps, not RTX 4070 measurements.

use genos_render::{
    column_peak, composed_frame_camera, composed_frame_scene, inject, integrate_unshadowed,
    picture_mean_abs, picture_unchanged, DirectSun, FogDispatch, FogView, GridSize, Renderer, World,
};
use genos_scene::FogSettings;
use genos_window::Window;

const COLUMN_ABS: f32 = 1.0e-3;

fn open(width: u32, height: u32) -> Result<genos_window::Lifecycle<Renderer>, String> {
    let mut window = Window::open_proof(width, height)?;
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height)?;
    Ok(genos_window::Lifecycle::new(window, renderer, permit))
}

fn height_fog() -> FogSettings {
    FogSettings {
        enabled: true,
        density: 0.08,
        thickness: 40.0,
        albedo: 1.0,
        phase_g: 0.4,
        history: false,
        gi: false,
        ..FogSettings::default()
    }
}

fn job(settings: FogSettings, size: GridSize) -> FogDispatch {
    FogDispatch {
        settings,
        view: FogView::look([0.0, 1.6, 0.0], 0.0, 0.0, 0.5, 1.0),
        size,
        sun: DirectSun {
            toward: [0.0, 1.0, 0.0],
            color: [1.0, 0.95, 0.85],
        },
        lamps: Vec::new(),
        scene: None,
        flags: 0,
        stop_slices: 0,
        read_extinction: true,
    }
}

fn peak_column(gpu: &[genos_render::FogColumn], cpu: &[genos_render::FogColumn]) -> f32 {
    let mut peak = 0.0_f32;
    for (left, right) in gpu.iter().zip(cpu) {
        for channel in 0..3 {
            peak = peak.max((left.inscatter[channel] - right.inscatter[channel]).abs());
        }
        peak = peak.max((left.transmittance - right.transmittance).abs());
    }
    peak
}

#[test]
fn gpu_columns_match_the_unshadowed_integral() {
    let mut owner = open(64, 64).expect("lavapipe device");
    let renderer = &mut *owner.device;
    let settings = height_fog();
    let size = GridSize {
        width: 4,
        height: 2,
        slices: 4,
    };
    let view = FogView::look([0.0, 1.6, 0.0], 0.0, 0.0, 0.5, 1.0);
    let sun = DirectSun {
        toward: [0.0, 1.0, 0.0],
        color: [1.0, 0.95, 0.85],
    };
    let cpu = integrate_unshadowed(&settings, &view, size, sun);
    let extinction = inject(&settings, &view, size);
    let gpu = renderer
        .run_fog(&job(settings, size))
        .expect("froxel dispatch");
    assert_eq!(gpu.columns.len(), cpu.len());
    let err = peak_column(&gpu.columns, &cpu);
    assert!(
        err <= COLUMN_ABS,
        "column abs {err} limit {COLUMN_ABS} pass_ms {} vram {}",
        gpu.pass_ms,
        gpu.vram_bytes
    );
    assert_eq!(gpu.extinction.len(), extinction.extinction.len());
    let mut ext = 0.0_f32;
    for (left, right) in gpu.extinction.iter().zip(&extinction.extinction) {
        ext = ext.max((left - right).abs());
    }
    assert!(ext <= COLUMN_ABS, "extinction abs {ext} limit {COLUMN_ABS}");
    assert!(gpu.vram_bytes > 0, "froxel allocation was 0");
    println!(
        "froxel parity lavapipe pass_ms {} vram_bytes {} column_abs {err} extinction_abs {ext}",
        gpu.pass_ms, gpu.vram_bytes
    );
}

#[test]
fn a_low_preset_frame_dispatches_the_froxel() {
    let mut owner = open(64, 64).expect("lavapipe device");
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let scene = composed_frame_scene(true);
    let world = World::from_scene(scene.clone());
    let camera = composed_frame_camera();
    let _ = window.pump();
    let on = renderer
        .draw(&world, &camera, true)
        .expect("draw")
        .expect("readback");
    let probe = renderer.fog_frame_probe(&scene.fog, scene.lighting.preset);
    assert_eq!(probe.dispatches, 1, "the frame did not dispatch froxel");
    assert_eq!(probe.history_accepts, 0);
    assert_eq!(probe.history_rejects, 0);
    assert!(probe.vram_bytes > 0, "frame vram was 0");
    assert!(probe.pass_ms.is_finite() && probe.pass_ms > 0.0);
    let columns = renderer.frame_fog_columns().expect("frame columns");
    let reference = renderer.frame_fog_reference(&scene);
    let err = column_peak(&columns, &reference);
    assert!(
        err <= f64::from(COLUMN_ABS),
        "frame column abs {err} limit {COLUMN_ABS}"
    );
    let off_scene = composed_frame_scene(false);
    let off_probe = renderer.fog_frame_probe(&off_scene.fog, off_scene.lighting.preset);
    assert_eq!(off_probe.dispatches, 0);
    assert_eq!(off_probe.pass_ms, 0.0);
    let off_world = World::from_scene(off_scene);
    let off = renderer
        .draw(&off_world, &camera, true)
        .expect("fog off draw")
        .expect("fog off readback");
    let unchanged = picture_unchanged(&on, &off);
    let mean = picture_mean_abs(&on, &off);
    println!(
        "froxel frame lavapipe pass_ms {} vram_bytes {} column_abs {err} picture_unchanged {unchanged} picture_mean_abs {mean}",
        probe.pass_ms, probe.vram_bytes
    );
    assert!(
        unchanged <= 0.0,
        "fog did not change the composed picture, mean abs {mean}"
    );
}
