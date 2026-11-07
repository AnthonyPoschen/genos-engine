#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")
)]

//! ADR 0035 amendment A (Proposed). The material record round-trips the
//! materials a glTF import produced, and decoding it does not enter the importer.

use std::path::Path;

use genos_load::{
    decode_materials, encode_materials, importer_entries, load_gltf_file, reset_importer_entries,
    CookedMaterials, PrimitiveShade,
};

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name)
}

#[test]
fn material_record_matches_the_gltf_materials() {
    for name in ["materials.gltf", "tri.gltf"] {
        let scene = load_gltf_file(fixture(name)).unwrap();
        let prims: usize = scene.meshes.iter().map(|mesh| mesh.primitives.len()).sum();
        let primitive_shade = (0..prims)
            .map(|i| {
                let n = f32::from(u8::try_from(i).unwrap_or(u8::MAX));
                PrimitiveShade {
                    mean: [0.1, 0.2, n + 0.3],
                    emission: [0.4, n + 0.5, 0.6],
                }
            })
            .collect();
        let record = CookedMaterials {
            asset_id: [7; 32],
            image_size: scene
                .images
                .iter()
                .map(|image| [image.width, image.height])
                .collect(),
            materials: scene.materials.clone(),
            primitive_shade,
        };
        let bytes = encode_materials(&record).unwrap();
        reset_importer_entries();
        let loaded = decode_materials(&bytes).unwrap();
        assert_eq!(importer_entries(), 0, "{name}");
        assert_eq!(loaded, record, "{name}");
    }
}

#[test]
fn a_damaged_material_record_does_not_decode() {
    let scene = load_gltf_file(fixture("materials.gltf")).unwrap();
    let record = CookedMaterials {
        asset_id: [3; 32],
        image_size: Vec::new(),
        materials: scene.materials,
        primitive_shade: Vec::new(),
    };
    let mut bytes = encode_materials(&record).unwrap();
    bytes[0] ^= 0xff;
    assert!(decode_materials(&bytes).is_err());
    let mut trailer = encode_materials(&record).unwrap();
    let last = trailer.len() - 1;
    trailer[last] ^= 0xff;
    assert!(decode_materials(&trailer).is_err());
}
