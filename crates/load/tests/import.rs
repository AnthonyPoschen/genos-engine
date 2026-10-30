#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")
)]

//! The import document around glTF (docs/systems/assets/plan.md, step 1).

use std::path::{Path, PathBuf};

use genos_load::{
    import_bytes, import_file, load_gltf, load_gltf_file, sha256, MemorySource, Support,
};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name)
}

fn no_files(_: &str) -> Result<Vec<u8>, genos_load::LoadError> {
    Err(genos_load::LoadError::Unrecognized)
}

#[test]
fn cube_imports_with_no_unsupported_notes() {
    let doc = import_file(fixture("cube.gltf")).unwrap();
    assert_eq!(doc.unsupported(), 0);
    assert_eq!(
        doc.scene.as_ref(),
        Some(&load_gltf_file(fixture("cube.gltf")).unwrap())
    );
    assert_eq!(
        doc.asset_id,
        sha256(&std::fs::read(fixture("cube.gltf")).unwrap())
    );
    assert!(doc
        .notes
        .iter()
        .any(|n| n.feature == "mesh" && n.support == Support::Used));
}

#[test]
fn skin_and_morph_target_are_kept_as_unsupported() {
    let text = std::fs::read_to_string(fixture("cube.gltf")).unwrap();
    let text = text
        .replacen("\"asset\"", "\"skins\": [{\"joints\": [0]}], \"asset\"", 1)
        .replacen(
            "\"attributes\"",
            "\"targets\": [{\"POSITION\": 0}], \"attributes\"",
            1,
        );
    let doc = import_bytes(text.as_bytes(), &no_files).unwrap();
    let triangles = doc.scene.as_ref().unwrap().meshes[0].primitives[0]
        .indices
        .len()
        / 3;
    assert_eq!(triangles, 12);
    assert_eq!(doc.unsupported(), 2);
    assert_eq!(doc.kept.len(), 2);
    assert!(doc
        .kept
        .iter()
        .any(|(p, raw)| p == "skins[0]" && raw.contains("joints")));
}

#[test]
fn required_extension_keeps_bytes_and_load_gltf_still_refuses() {
    let text = std::fs::read_to_string(fixture("cube.gltf")).unwrap();
    let text = text.replacen(
        "\"asset\"",
        "\"extensionsRequired\": [\"KHR_texture_transform\"], \"asset\"",
        1,
    );
    assert!(load_gltf(&MemorySource::new(text.as_bytes()), &no_files).is_err());
    let doc = import_bytes(text.as_bytes(), &no_files).unwrap();
    assert!(doc.scene.is_none());
    assert_eq!(doc.source, text.as_bytes());
    assert_eq!(doc.notes[0].feature, "KHR_texture_transform");
}

#[test]
fn external_files_feed_the_inputs_hash() {
    let doc = import_file(fixture("tri.gltf")).unwrap();
    let uris: Vec<&str> = doc.externals.iter().map(|(u, _)| u.as_str()).collect();
    assert_eq!(uris, ["tri.bin", "quad.png"]);
    let mut changed = doc.clone();
    changed.externals[0].1[0] ^= 1;
    assert_ne!(doc.inputs_hash(), changed.inputs_hash());
}
