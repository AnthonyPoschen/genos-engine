#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")
)]

//! ADR 0035 amendment A (Proposed). A valid `.gmat` package matches the import
//! without entering the glTF decoder. A bad record falls back to that decoder.

use std::path::{Path, PathBuf};
use std::time::Instant;

use genos_bake::{BakeMode, BakeSettings};
use genos_cook::{cook_asset, load_cooked, package, Threads};
use genos_load::{
    decode_materials, encode_materials, hex, importer_entries, load_covered,
    load_covered_or_source, load_gltf_file, materials_path, reset_importer_entries, sha256,
    write_atomic, CookedMaterials, Package, PrimitiveShade,
};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../load/fixtures")
        .join(name)
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("genos-gmat-{name}-{}", std::process::id()));
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

fn copy_fixture(dir: &Path, names: &[&str]) -> PathBuf {
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    for name in names {
        std::fs::copy(fixture(name), src.join(name)).unwrap();
    }
    src
}

fn stomp_objects(dir: &Path) {
    let objects = dir.join("objects");
    if !objects.is_dir() {
        return;
    }
    for entry in std::fs::read_dir(&objects).unwrap() {
        std::fs::write(entry.unwrap().path(), b"not-a-gltf").unwrap();
    }
}

/// Cook `source`, write `.gmat` from its import, and point the manifest at it.
fn cover(dir: &Path, source: &Path) -> [u8; 32] {
    let asset = cook_asset(source, dir, &settings(dir), true, Threads::ONE).unwrap();
    let scene = load_gltf_file(source).unwrap();
    let id = sha256(&std::fs::read(source).unwrap());
    let prims: usize = scene.meshes.iter().map(|mesh| mesh.primitives.len()).sum();
    let record = CookedMaterials {
        asset_id: id,
        image_size: scene
            .images
            .iter()
            .map(|image| [image.width, image.height])
            .collect(),
        materials: scene.materials.clone(),
        primitive_shade: (0..prims)
            .map(|i| {
                let n = f32::from(u8::try_from(i).unwrap_or(u8::MAX));
                PrimitiveShade {
                    mean: [0.1, 0.2, n + 0.3],
                    emission: [0.4, n + 0.5, 0.6],
                }
            })
            .collect(),
    };
    let bytes = encode_materials(&record).unwrap();
    write_atomic(&materials_path(dir, &id), &bytes).unwrap();
    let cooked = load_cooked(&asset.record.1).unwrap();
    let gtex = asset
        .textures
        .as_ref()
        .map(|(_, path)| sha256(&std::fs::read(path).unwrap()));
    let mut manifest = package(&[(&cooked, gtex)]);
    let line = manifest
        .lines
        .iter_mut()
        .find(|line| line.starts_with(&hex(&id)))
        .unwrap();
    line.push(' ');
    line.push_str("mat=");
    line.push_str(&hex(&sha256(&bytes)));
    write_atomic(&dir.join("package.manifest"), manifest.text().as_bytes()).unwrap();
    id
}

#[test]
fn covered_package_matches_the_import_with_the_source_removed() {
    let cases = [
        (
            "tri",
            ["tri.gltf", "tri.bin", "quad.png"].as_slice(),
            "tri.gltf",
        ),
        (
            "materials",
            ["materials.gltf", "quad.png"].as_slice(),
            "materials.gltf",
        ),
    ];
    for (name, files, source_name) in cases {
        let dir = temp(name);
        let src = copy_fixture(&dir, files);
        let source = src.join(source_name);
        let oracle = load_gltf_file(&source).unwrap();
        let id = cover(&dir, &source);
        assert!(Package::open(&dir).unwrap().verify(&dir).is_ok(), "{name}");
        stomp_objects(&dir);
        for file in files {
            std::fs::remove_file(src.join(file)).unwrap();
        }
        reset_importer_entries();
        let loaded = load_covered(&dir, &id).unwrap();
        assert_eq!(importer_entries(), 0, "{name}");
        assert_eq!(loaded.meshes, oracle.meshes, "{name}");
        assert_eq!(loaded.instances, oracle.instances, "{name}");
        assert_eq!(loaded.materials, oracle.materials, "{name}");
        assert_eq!(
            loaded.image_size,
            oracle
                .images
                .iter()
                .map(|image| [image.width, image.height])
                .collect::<Vec<_>>(),
            "{name}"
        );
        assert!(!loaded.cooked.is_empty(), "{name}");
        for image in &loaded.cooked {
            let index = usize::try_from(image.image).unwrap();
            assert_eq!(
                [image.width, image.height],
                loaded.image_size[index],
                "{name}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn a_cooked_image_size_that_differs_is_corrupt() {
    let dir = temp("size");
    let src = copy_fixture(&dir, &["tri.gltf", "tri.bin", "quad.png"]);
    let source = src.join("tri.gltf");
    let id = cover(&dir, &source);
    let path = materials_path(&dir, &id);
    let old = std::fs::read(&path).unwrap();
    let mut record = decode_materials(&old).unwrap();
    record.image_size[0][0] += 4;
    let bytes = encode_materials(&record).unwrap();
    let manifest = dir.join("package.manifest");
    let text = std::fs::read_to_string(&manifest)
        .unwrap()
        .replace(&hex(&sha256(&old)), &hex(&sha256(&bytes)));
    write_atomic(&path, &bytes).unwrap();
    write_atomic(&manifest, text.as_bytes()).unwrap();
    let err = load_covered(&dir, &id).unwrap_err();
    assert!(
        err.to_string()
            .contains("image size differs from the cooked texture"),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn corrupt_material_record_falls_back_to_the_source() {
    let dir = temp("fallback");
    let src = copy_fixture(&dir, &["tri.gltf", "tri.bin", "quad.png"]);
    let source = src.join("tri.gltf");
    let oracle = load_gltf_file(&source).unwrap();
    let id = cover(&dir, &source);
    let mat = materials_path(&dir, &id);
    let mut bytes = std::fs::read(&mat).unwrap();
    let at = bytes.len() / 2;
    bytes[at] ^= 0x01;
    std::fs::write(&mat, &bytes).unwrap();
    assert!(
        Package::open(&dir).unwrap().verify(&dir).is_ok(),
        "the accepted verifier does not read mat="
    );
    stomp_objects(&dir);
    reset_importer_entries();
    let loaded = load_covered_or_source(&dir, &id, &source);
    assert!(loaded.used_source);
    assert_eq!(importer_entries(), 1);
    assert!(
        loaded
            .fallback
            .iter()
            .any(|line| line.contains("material hash differs from the manifest")),
        "{:?}",
        loaded.fallback
    );
    assert_eq!(loaded.asset.materials, oracle.materials);
    assert_eq!(loaded.asset.meshes, oracle.meshes);
    assert_eq!(loaded.asset.instances, oracle.instances);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn covered_load_time_against_the_gltf_import() {
    let dir = temp("timing");
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    let text = std::fs::read_to_string(fixture("tri.gltf"))
        .unwrap()
        .replace("quad.png", "big.png");
    std::fs::write(src.join("tri.gltf"), text).unwrap();
    std::fs::copy(fixture("tri.bin"), src.join("tri.bin")).unwrap();
    let edge = 256u32;
    let texels = usize::try_from(edge).unwrap() * usize::try_from(edge).unwrap();
    let mut rgba = vec![0u8; texels * 4];
    for i in 0..texels {
        rgba[i * 4] = u8::try_from(i % 251).unwrap_or(0);
        rgba[i * 4 + 1] = 40;
        rgba[i * 4 + 2] = 90;
        rgba[i * 4 + 3] = 255;
    }
    write_png(&src.join("big.png"), edge, edge, &rgba);
    let source = src.join("tri.gltf");
    let id = cover(&dir, &source);

    let time_ms = |mut call: Box<dyn FnMut()>| -> f64 {
        call();
        let mut samples = Vec::new();
        for _ in 0..5 {
            let start = Instant::now();
            call();
            samples.push(start.elapsed().as_secs_f64() * 1e3);
        }
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        samples[samples.len() / 2]
    };
    let gltf_ms = time_ms(Box::new(|| {
        load_gltf_file(&source).unwrap();
    }));
    let covered_ms = time_ms(Box::new(|| {
        load_covered(&dir, &id).unwrap();
    }));
    println!("load-ab fixture=generated-256 gltf_ms={gltf_ms:.3} covered_ms={covered_ms:.3} n=5");

    std::fs::remove_file(&source).unwrap();
    std::fs::remove_file(src.join("tri.bin")).unwrap();
    std::fs::remove_file(src.join("big.png")).unwrap();
    stomp_objects(&dir);
    reset_importer_entries();
    let without_source = load_covered(&dir, &id).unwrap();
    assert_eq!(importer_entries(), 0);
    assert_eq!(without_source.materials.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

fn adler(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn png_chunk(kind: [u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
    out.extend_from_slice(&kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[4..]);
    out.extend_from_slice(&crc.to_be_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) {
    let mut raw = Vec::new();
    for row in rgba.chunks(usize::try_from(width).unwrap() * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut zlib = vec![0x78, 0x01];
    let mut i = 0;
    while i < raw.len() {
        let n = u16::try_from((raw.len() - i).min(65535)).unwrap();
        let last = u8::from(i + usize::from(n) == raw.len());
        zlib.push(last);
        zlib.extend_from_slice(&n.to_le_bytes());
        zlib.extend_from_slice(&(!n).to_le_bytes());
        zlib.extend_from_slice(&raw[i..i + usize::from(n)]);
        i += usize::from(n);
    }
    zlib.extend_from_slice(&adler(&raw).to_be_bytes());
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut png = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend(png_chunk(*b"IHDR", &ihdr));
    png.extend(png_chunk(*b"IDAT", &zlib));
    png.extend(png_chunk(*b"IEND", &[]));
    std::fs::write(path, png).unwrap();
}
