#![expect(clippy::expect_used, reason = "a test aborts when a fixture fails")]

//! `media_check.comp` on lavapipe against `genos_math::media`.
//!
//! The tolerance is 1e-5. A broken candidate is the analytic harness's wrong
//! formulas on the same inputs. That candidate must miss the tolerance.

use genos_math::{
    height_fog_density_f32, height_optical_depth_f32, homogeneous_inscatter_f32, phase_hg_f32,
    transmittance_f32,
};
use genos_render::Renderer;
use genos_window::Window;

/// Absolute error the GPU closed forms may have against the `f32` wrappers.
const PARITY_ABS: f32 = 1.0e-5;

/// The five values `media_check.comp` writes, from `genos_math`.
fn cpu_media() -> [f32; 5] {
    [
        transmittance_f32(0.5),
        height_fog_density_f32(0.08, 40.0, 10.0, 0.0),
        phase_hg_f32(0.4, 0.5),
        homogeneous_inscatter_f32(0.2, 0.15, 2.0),
        height_optical_depth_f32(0.08, 40.0, 0.0, 10.0, 20.0, 0.5, 40.0),
    ]
}

/// The broken analytic formulas, on the shader's inputs.
///
/// Transmittance is `max(0, 1 - σd)`. Height density is a linear ramp.
/// The phase returns 1. In-scatter adds `S * D` with no transmittance.
/// Height optical depth omits `/ cos θ`.
fn broken_media() -> [f32; 5] {
    let sigma_d = 0.5_f32;
    let linear = {
        let sigma0 = 0.08_f32;
        let thickness = 40.0_f32;
        let y = 10.0_f32;
        let base = 0.0_f32;
        let ramp = sigma0 * (1.0 - (y - base) / thickness);
        ramp.max(0.0)
    };
    let source = 0.15_f32;
    let distance = 2.0_f32;
    let sigma0 = 0.08_f32;
    let thickness = 40.0_f32;
    let y0 = 10.0_f32;
    let dy = 20.0_f32;
    let sigma_y0 = sigma0 * (-(y0) / thickness).exp();
    let depth_without_cos = sigma_y0 * thickness * (1.0 - (-dy / thickness).exp());
    [
        (1.0 - sigma_d).max(0.0),
        linear,
        1.0,
        source * distance,
        depth_without_cos,
    ]
}

fn max_abs(left: &[f32; 5], right: &[f32; 5]) -> f32 {
    let mut peak = 0.0_f32;
    for (a, b) in left.iter().zip(right) {
        peak = peak.max((a - b).abs());
    }
    peak
}

fn open() -> Result<genos_window::Lifecycle<Renderer>, String> {
    let mut window = Window::open_proof(64, 64)?;
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height)?;
    Ok(genos_window::Lifecycle::new(window, renderer, permit))
}

#[test]
fn a_broken_media_candidate_fails_the_parity_gate() {
    let err = max_abs(&cpu_media(), &broken_media());
    assert!(
        err > PARITY_ABS,
        "broken candidate error {err} is inside {PARITY_ABS}"
    );
}

#[test]
fn media_glsl_matches_genos_math_on_lavapipe() {
    let mut owner = open().expect("lavapipe device");
    let renderer = &mut *owner.device;
    let gpu = renderer.media_closed_forms().expect("media_check dispatch");
    let cpu = cpu_media();
    let err = max_abs(&gpu, &cpu);
    assert!(
        err <= PARITY_ABS,
        "gpu {gpu:?} cpu {cpu:?} abs {err} limit {PARITY_ABS}"
    );
    let broken = max_abs(&gpu, &broken_media());
    assert!(
        broken > PARITY_ABS,
        "gpu matched the broken formulas within {PARITY_ABS}: gpu {gpu:?} err {broken}"
    );
}
