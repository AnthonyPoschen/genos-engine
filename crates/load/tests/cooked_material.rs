#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")
)]

//! ADR 0035 amendment A (Proposed). The material record round-trips the
//! materials a glTF import produced, and decoding it does not enter the importer.

use std::path::Path;

use genos_load::{
    decode_materials, encode_materials, importer_entries, load_gltf_file, primitive_shades,
    reset_importer_entries, CookedMaterials, GltfMesh, GltfScene, PbrMaterial, Primitive,
    PrimitiveShade,
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

#[test]
fn primitive_shades_follow_mesh_order() {
    let scene = GltfScene {
        materials: vec![
            PbrMaterial {
                base_color: [0.2, 0.4, 0.6, 1.0],
                metallic: 0.0,
                ..PbrMaterial::default()
            },
            PbrMaterial {
                base_color: [0.0, 1.0, 0.0, 1.0],
                metallic: 0.5,
                emissive: [0.25, 0.0, 0.0],
                emissive_strength: 2.0,
                ..PbrMaterial::default()
            },
        ],
        meshes: vec![
            GltfMesh {
                primitives: vec![prim(0), prim(1)],
                ..GltfMesh::default()
            },
            GltfMesh {
                primitives: vec![prim(0)],
                ..GltfMesh::default()
            },
        ],
        ..GltfScene::default()
    };
    let shades = primitive_shades(&scene);
    assert_eq!(shades.len(), 3);
    assert_eq!(
        shades[0],
        PrimitiveShade {
            mean: [0.2, 0.4, 0.6],
            emission: [0.0; 3],
        }
    );
    assert_eq!(
        shades[1],
        PrimitiveShade {
            mean: [0.0, 0.5, 0.0],
            emission: [0.5, 0.0, 0.0],
        }
    );
    assert_eq!(shades[2], shades[0]);
}

fn prim(material: usize) -> Primitive {
    Primitive {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        indices: vec![0, 1, 2],
        material: Some(material),
        ..Primitive::default()
    }
}
