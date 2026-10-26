#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")
)]

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

#[test]
fn a_texcoord_hit_matches_its_slot() {
    use std::path::PathBuf;

    use genos_bake::BakeMesh;
    use genos_load::load_gltf_file;

    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/materials/panel.gltf");
    let scene = load_gltf_file(&path).unwrap();
    let mesh = BakeMesh::from_gltf(&scene, 0);
    let tri = &mesh.triangles[0];
    assert!((tri.uvs[0][0] - 0.25).abs() < 1.0e-5);
    assert!((tri.uvs[0][1] - 0.25).abs() < 1.0e-5);
    let reference = mesh.shade_hit(0, 0.0, 0.0);
    let legacy = mesh.shade(tri.material, tri.uvs[0]);
    let irradiance = [1.0; 3];
    let before = blurred(
        legacy.outgoing(1.0, irradiance, [0.0; 3]),
        reference.outgoing(1.0, irradiance, [0.0; 3]),
    );
    let after = blurred(
        reference.outgoing(1.0, irradiance, [0.0; 3]),
        reference.outgoing(1.0, irradiance, [0.0; 3]),
    );
    eprintln!("picture_texcoord before {before:.6} after {after:.6}");
    assert!(
        (reference.emissive[0] - 1.0).abs() < 1.0e-5,
        "set 1 emission {}",
        reference.emissive[0]
    );
    assert!(
        (reference.diffuse_albedo()[0] - 0.502_886_5).abs() < 1.0e-4,
        "set 0 albedo {}",
        reference.diffuse_albedo()[0]
    );
    assert!((before - 0.521_756).abs() < 1.0e-4, "picture_texcoord before {before}");
    assert!(after <= 1.0e-6, "picture_texcoord after {after}");
}

#[test]
fn roughness_from_g_leaves_the_lambert_picture() {
    use std::path::PathBuf;

    use genos_bake::BakeMesh;
    use genos_load::load_gltf_file;

    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/materials/panel.gltf");
    let scene = load_gltf_file(&path).unwrap();
    let mesh = BakeMesh::from_gltf(&scene, 0);
    let sample = mesh.shade(0, [0.25, 0.25]);
    let hand = 0.5 * (10.0 / 255.0);
    let before = (0.0_f32 - hand).abs();
    let after = (sample.roughness - hand).abs();
    let (metal_before, metal_after) = score(0.25);
    eprintln!(
        "picture_roughness before {before:.6} after {after:.6}\n\
         picture_lambert before {metal_before:.6} after {metal_after:.6}"
    );
    assert!((before - hand).abs() < 1.0e-6);
    assert!(after <= 1.0e-6, "roughness error {after}");
    assert!((metal_before - 1.0 / 3.0).abs() < 1.0e-5);
    assert!(metal_after <= 1.0e-6);
}
