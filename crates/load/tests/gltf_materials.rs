#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")
)]

//! glTF metallic-roughness materials into the CPU BRDF reference, against the
//! hand-written `fixtures/materials.gltf`.

use std::path::Path;

use genos_load::{load_gltf_file, AlphaMode, GltfScene, PbrMaterial};
use genos_math::brdf::{self, Cosines};

fn scene() -> GltfScene {
    load_gltf_file(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/materials.gltf")).unwrap()
}

fn named<'a>(scene: &'a GltfScene, name: &str) -> &'a PbrMaterial {
    scene
        .materials
        .iter()
        .find(|m| m.name.as_deref() == Some(name))
        .unwrap()
}

fn close(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6)
}

#[test]
fn all_six_materials_load_in_order() {
    let names: Vec<_> = scene()
        .materials
        .iter()
        .map(|m| m.name.clone().unwrap())
        .collect();
    assert_eq!(
        names,
        ["defaults", "gold", "lamp", "leaf", "glass", "textured"]
    );
}

#[test]
fn missing_fields_take_the_spec_defaults() {
    let s = scene();
    let m = named(&s, "defaults");
    assert_eq!(
        *m,
        PbrMaterial {
            name: Some("defaults".into()),
            ..PbrMaterial::default()
        }
    );
    let b = m.brdf_material();
    assert_eq!(b, brdf::Material::metal([1.0; 3], 1.0));
    assert_eq!(m.alpha_cutoff(), None);
}

#[test]
fn gold_metal_matches_the_reference_constructor() {
    let s = scene();
    let b = named(&s, "gold").brdf_material();
    let want = brdf::Material::metal([1.0, 0.766, 0.336].map(|c: f32| f64::from(c)), 0.25);
    assert_eq!(b, want);
    assert!(close(b.f0(), want.base_color));
    assert!(close(b.diffuse_albedo(), [0.0; 3]));
}

#[test]
fn emissive_strength_multiplies_into_the_reference_emission() {
    let s = scene();
    let m = named(&s, "lamp");
    assert_eq!(m.emissive_strength, 4.0);
    let b = m.brdf_material();
    assert!(close(b.emission(), [4.0, 2.0, 1.0]));
    // Shading with no light returns the emission alone.
    let c = Cosines::from_dirs([0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]);
    assert!(close(brdf::shade(&b, c, [0.0; 3]), [4.0, 2.0, 1.0]));
    assert!(close(b.diffuse_albedo(), [0.5; 3]));
}

#[test]
fn mask_cutoff_defaults_and_double_sided() {
    let s = scene();
    let leaf = named(&s, "leaf");
    assert_eq!(leaf.alpha_mode, AlphaMode::Mask(0.5));
    assert_eq!(leaf.alpha_cutoff(), Some(0.5));
    assert!(leaf.double_sided);
    assert_eq!(leaf.base_color[3], 0.5);
    assert_eq!(leaf.base_color_texture.unwrap().image, 0);
    let glass = named(&s, "glass");
    assert_eq!(glass.alpha_mode, AlphaMode::Blend);
    assert_eq!(glass.alpha_cutoff(), None);
    assert!(!glass.double_sided);
}

#[test]
fn texture_slots_keep_texcoord_scale_strength_and_sampler() {
    let s = scene();
    let m = named(&s, "textured");
    let mr = m.metallic_roughness_texture.unwrap();
    assert_eq!(mr.texcoord, 1);
    assert_eq!(
        (mr.sampler.mag_filter, mr.sampler.wrap_s, mr.sampler.wrap_t),
        (9728, 33071, 33648)
    );
    assert_eq!(m.normal_texture.unwrap().scale, 0.5);
    assert_eq!(m.occlusion_texture.unwrap().scale, 0.8);
    assert!(m.emissive_texture.is_some());
}

#[test]
fn textured_sample_feeds_the_reference_per_texel() {
    let s = scene();
    let m = named(&s, "textured");
    let sample = genos_load::shade_uv(m, &s.images, [0.25, 0.25]);
    let b = sample.brdf_material();
    assert!((b.metallic - f64::from(sample.metallic)).abs() < 1e-7);
    assert!((b.roughness - f64::from(sample.roughness)).abs() < 1e-7);
    assert!(b.metallic <= 0.5 && b.roughness <= 0.5);
    assert!(close(b.emission(), sample.emissive.map(f64::from)));
    assert_eq!(b.emissive_strength, 1.0);
}
