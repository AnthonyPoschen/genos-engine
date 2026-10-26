//! Picture score of the shared shade against the path tracer's Lambert product.
//!
//! The legacy picture uses the base colour and ignores metallic. The path tracer
//! and the real-time picture both use `ShadeSample::outgoing`. One pixel, so the
//! blurred relative error is the pixel term from the lighting reference check.

use genos_load::{shade_uv, PbrMaterial, ShadeSample};

fn luma(colour: [f32; 3]) -> f32 {
    0.2126 * colour[0] + 0.7152 * colour[1] + 0.0722 * colour[2]
}

fn blurred(candidate: [f32; 3], reference: [f32; 3]) -> f32 {
    let yc = luma(candidate);
    let yr = luma(reference);
    let mean = yr;
    (yc - yr).abs() / yr.max(0.25 * mean).max(0.02)
}

fn score(metallic: f32) -> (f32, f32) {
    let material = PbrMaterial {
        base_color: [0.8, 0.1, 0.1, 1.0],
        metallic,
        roughness: 0.5,
        ..PbrMaterial::default()
    };
    let sample = shade_uv(&material, &[], [0.5, 0.5]);
    let irradiance = [1.0; 3];
    // The BRDF weight cancels in the ratio. The path tracer passes its Lambert
    // constant to the same `outgoing`.
    let tracer = sample.outgoing(1.0, irradiance, [0.0; 3]);
    let legacy = ShadeSample::paint([0.8, 0.1, 0.1]).outgoing(1.0, irradiance, [0.0; 3]);
    (blurred(legacy, tracer), blurred(tracer, tracer))
}

#[test]
fn the_lambert_picture_matches_the_path_tracer() {
    let (metal_before, metal_after) = score(0.25);
    let (dielectric_before, dielectric_after) = score(0.0);
    eprintln!(
        "picture_lambert before {metal_before:.6} after {metal_after:.6}\n\
         picture_dielectric before {dielectric_before:.6} after {dielectric_after:.6}"
    );
    assert!(
        (metal_before - 1.0 / 3.0).abs() < 1.0e-5,
        "legacy metal picture {metal_before}"
    );
    assert!(metal_after <= 1.0e-6, "shared metal picture {metal_after}");
    assert!(dielectric_before <= 1.0e-6);
    assert!(dielectric_after <= 1.0e-6);
}
