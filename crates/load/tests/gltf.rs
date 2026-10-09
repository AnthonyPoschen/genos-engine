//! glTF 2.0 loading against small fixtures written by `fixtures/make_gltf.py`.

use std::path::{Path, PathBuf};

use genos_load::{load_gltf, load_gltf_file, transform_point, AlphaMode, LoadError, MemorySource};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name)
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
}

#[test]
fn gltf_data_uri_cube_with_node_chain_and_material() {
    let scene = load_gltf_file(fixture("cube.gltf")).unwrap();
    assert_eq!(scene.meshes.len(), 1);
    let p = &scene.meshes[0].primitives[0];
    assert_eq!(
        (p.positions.len(), p.normals.len(), p.indices.len()),
        (8, 8, 36)
    );
    assert_eq!(scene.meshes[0].name.as_deref(), Some("cube"));
    // parent T(10,0,0) x child R(y, 90 deg) S(2): local +x goes to world (10, 0, -2).
    let inst = &scene.instances[0];
    assert_eq!(inst.node_name.as_deref(), Some("child"));
    assert!(close(
        transform_point(&inst.transform, [1.0, 0.0, 0.0]),
        [10.0, 0.0, -2.0]
    ));
    let m = &scene.materials[0];
    assert_eq!(m.base_color, [0.8, 0.1, 0.1, 1.0]);
    assert_eq!((m.metallic, m.roughness), (0.25, 0.5));
    assert_eq!(m.alpha_mode, AlphaMode::Mask(0.3));
    assert!(m.double_sided);
    assert!(close(m.diffuse_albedo(), [0.6, 0.075, 0.075]));
    assert_eq!(scene.world_triangles().len(), 12);
}

#[test]
fn glb_interleaved_u8_indices_strip_flat_normals_and_jpeg() {
    let bytes = std::fs::read(fixture("quad.glb")).unwrap();
    let scene = load_gltf(&MemorySource::new(&bytes), &|_| {
        Err(LoadError::Unrecognized)
    })
    .unwrap();
    let prims = &scene.meshes[0].primitives;
    assert_eq!(prims.len(), 2, "the line primitive is skipped");
    // No normals: unwelded with face normals.
    assert_eq!((prims[0].positions.len(), prims[0].texcoords.len()), (6, 6));
    assert!(prims[0].normals.iter().all(|&n| close(n, [0.0, 0.0, 1.0])));
    assert_eq!(prims[0].texcoords[0], [0.0, 1.0]);
    // A 4-index strip is two triangles, both facing +z.
    assert_eq!(prims[1].indices.len(), 6);
    assert!(prims[1].normals.iter().all(|&n| close(n, [0.0, 0.0, 1.0])));
    assert_eq!((scene.images[0].width, scene.images[0].height), (37, 29));
    let m = &scene.materials[0];
    let slot = m.base_color_texture.unwrap();
    assert_eq!(
        (slot.image, slot.sampler.wrap_s, slot.sampler.wrap_t),
        (0, 33071, 10497)
    );
    assert_eq!((m.emissive, m.emissive_strength), ([1.0, 0.5, 0.25], 4.0));
    assert!(close(
        transform_point(&scene.instances[0].transform, [0.0; 3]),
        [0.0, 0.0, -5.0]
    ));
}

#[test]
fn gltf_external_buffer_and_image_with_sparse_accessor_and_no_scene() {
    let scene = load_gltf_file(fixture("tri.gltf")).unwrap();
    let tris = scene.world_triangles();
    assert_eq!(tris.len(), 1);
    assert!(
        close(tris[0].0[2], [0.0, 4.0, 0.0]),
        "sparse value moved vertex 2 to y = 3"
    );
    assert_eq!((scene.images[0].width, scene.images[0].height), (2, 2));
    let m = &scene.materials[0];
    assert_eq!(m.normal_texture.unwrap().scale, 0.5);
    assert!(m.emissive_texture.is_some());
}

#[test]
fn gltf_refuses_required_extensions_and_bad_files() {
    let text = std::fs::read_to_string(fixture("cube.gltf")).unwrap();
    let draco = text.replacen(
        '{',
        "{\"extensionsRequired\": [\"KHR_draco_mesh_compression\"],",
        1,
    );
    let none = |_: &str| -> Result<Vec<u8>, LoadError> { Err(LoadError::Unrecognized) };
    assert_eq!(
        load_gltf(&MemorySource::new(draco.as_bytes()), &none),
        Err(LoadError::Unrecognized)
    );
    let v1 = text.replace("\"2.0\"", "\"1.0\"");
    assert_eq!(
        load_gltf(&MemorySource::new(v1.as_bytes()), &none),
        Err(LoadError::Unrecognized)
    );
    assert!(load_gltf_file(fixture("missing.gltf")).is_err());
    let glb = std::fs::read(fixture("quad.glb")).unwrap();
    for cut in [8, 20, 100, glb.len() - 4] {
        assert!(load_gltf(&MemorySource::new(&glb[..cut]), &none).is_err());
    }
}
