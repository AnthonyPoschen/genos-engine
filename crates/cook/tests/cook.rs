#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")
)]

//! Cook round trip, incremental rebuild, the load-time check and the manifest.

use std::path::{Path, PathBuf};

use genos_bake::{BakeMesh, BakeMode, BakeSettings};
use genos_cook::{cook, cook_file, decode, encode, load_cooked, CookError, Outcome};
use genos_load::{import_file, load_gltf_file};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../load/fixtures")
        .join(name)
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("genos-cook-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn settings(dir: &Path) -> BakeSettings {
    BakeSettings {
        mode: BakeMode::OnLoad,
        cache_dir: dir.join("bake"),
        ..BakeSettings::default()
    }
}

#[test]
fn cooked_record_matches_direct_import() {
    let dir = temp("roundtrip");
    let s = settings(&dir);
    for name in ["cube.gltf", "tri.gltf", "quad.glb", "materials.gltf"] {
        let direct = load_gltf_file(fixture(name)).unwrap();
        let cooked = cook(&import_file(fixture(name)).unwrap(), &s).unwrap();
        let loaded = decode(&encode(&cooked)).unwrap();
        assert_eq!(loaded, cooked, "{name}");
        let meshes: Vec<_> = loaded.meshes.iter().map(|m| m.mesh.clone()).collect();
        assert_eq!(meshes, direct.meshes, "{name}");
        assert_eq!(loaded.instances, direct.instances, "{name}");
        for (i, m) in loaded.meshes.iter().enumerate() {
            assert_eq!(
                m.bake_key,
                BakeMesh::from_gltf(&direct, i).content_key(&s),
                "{name}"
            );
        }
    }
    let cube = cook(&import_file(fixture("cube.gltf")).unwrap(), &s).unwrap();
    assert_eq!(cube.meshes[0].mesh.primitives[0].indices.len() / 3, 12);
    assert_eq!(
        encode(&cube),
        encode(&cook(&import_file(fixture("cube.gltf")).unwrap(), &s).unwrap())
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unchanged_input_is_not_rebuilt() {
    let dir = temp("incremental");
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    for name in ["tri.gltf", "tri.bin", "quad.png"] {
        std::fs::copy(fixture(name), src.join(name)).unwrap();
    }
    let out = dir.join("out");
    let s = settings(&dir);
    let (first, path) = cook_file(&src.join("tri.gltf"), &out, &s).unwrap();
    assert_eq!(first, Outcome::Cooked);
    let stamp = std::fs::metadata(&path).unwrap().modified().unwrap();
    let (second, again) = cook_file(&src.join("tri.gltf"), &out, &s).unwrap();
    assert_eq!((second, &again), (Outcome::UpToDate, &path));
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), stamp);

    let mut png = std::fs::read(src.join("quad.png")).unwrap();
    png.push(0);
    std::fs::write(src.join("quad.png"), &png).unwrap();
    assert_eq!(
        cook_file(&src.join("tri.gltf"), &out, &s).unwrap().0,
        Outcome::Cooked
    );
    assert_eq!(
        cook_file(&src.join("tri.gltf"), &out, &s).unwrap().0,
        Outcome::UpToDate
    );

    let coarser = BakeSettings {
        card_texel: s.card_texel * 2.0,
        ..s.clone()
    };
    assert_eq!(
        cook_file(&src.join("tri.gltf"), &out, &coarser).unwrap().0,
        Outcome::Cooked
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn load_refuses_a_damaged_record() {
    let dir = temp("damaged");
    let (_, path) = cook_file(&fixture("cube.gltf"), &dir.join("out"), &settings(&dir)).unwrap();
    assert!(load_cooked(&path).is_ok());
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[60] ^= 1;
    assert!(matches!(decode(&bytes), Err(CookError::Corrupt(_))));
    bytes[60] ^= 1;
    bytes[4] = 9;
    assert!(matches!(decode(&bytes), Err(CookError::Corrupt(_))));
    assert!(matches!(decode(&bytes[..10]), Err(CookError::Corrupt(_))));
    std::fs::write(&path, b"GCK1").unwrap();
    assert_eq!(
        cook_file(&fixture("cube.gltf"), &dir.join("out"), &settings(&dir))
            .unwrap()
            .0,
        Outcome::Cooked
    );
    let _ = std::fs::remove_dir_all(&dir);
}
