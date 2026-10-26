#![expect(clippy::expect_used, reason = "a test aborts when a fixture fails")]

//! Lavapipe froxel against the CPU column integral.
//!
//! `run_fog` is the inject and integrate dispatches. The frame test draws one
//! Low-preset picture so the same pass is recorded after the depth prepass.
//! Times are lavapipe timestamps, not RTX 4070 measurements.

use genos_render::{
    inject, integrate_unshadowed, DirectSun, FogDispatch, FogView, GridSize, Renderer, World,
};
use genos_scene::{Camera, Floor, FogSettings, LightingPreset, LightingSettings, Scene, Vec3};
use genos_window::Window;

const COLUMN_ABS: f32 = 1.0e-3;

fn open(width: u32, height: u32) -> Result<(Window, Renderer), String> {
    let mut window = Window::open_proof(width, height)?;
    let frame = window.pump();
    let renderer = Renderer::open(window.display, window.surface, frame.width, frame.height)?;
    Ok((window, renderer))
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
    let (_window, mut renderer) = open(64, 64).expect("lavapipe device");
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
    let (mut window, mut renderer) = open(64, 64).expect("lavapipe device");
    let mut scene = Scene {
        floor: Floor {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 4.0,
            half_z: 4.0,
            color: [0.6, 0.6, 0.6],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights: Vec::new(),
        ceiling: None,
        sky: None,
        lighting: LightingSettings::preset(LightingPreset::Low),
        debug: genos_scene::DebugSettings::default(),
        fps_counter: genos_scene::FpsCounterSettings::default(),
        event_loop: genos_scene::EventLoopSettings::default(),
        fog: height_fog(),
        menu: genos_scene::MenuSettings::default(),
    };
    let world = World::from_scene(scene.clone());
    let camera = Camera::new(0.0, 2.0, 0.0);
    let _ = window.pump();
    renderer
        .draw(&world, &camera, true)
        .expect("draw")
        .expect("readback");
    let probe = renderer.fog_frame_probe(&scene.fog, scene.lighting.preset);
    assert_eq!(probe.dispatches, 1);
    assert_eq!(probe.history_accepts, 0);
    assert_eq!(probe.history_rejects, 0);
    assert!(probe.vram_bytes > 0, "frame vram was 0");
    assert!(probe.pass_ms.is_finite() && probe.pass_ms >= 0.0);
    println!(
        "froxel frame lavapipe pass_ms {} vram_bytes {}",
        probe.pass_ms, probe.vram_bytes
    );
    scene.fog.enabled = false;
    let off = renderer.fog_frame_probe(&scene.fog, scene.lighting.preset);
    assert_eq!(off.dispatches, 0);
    assert_eq!(off.pass_ms, 0.0);
}
