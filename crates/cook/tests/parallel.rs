#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")
)]

//! A cook on one thread and on many writes the same bytes: same `.gcook` and
//! `.gtex` hashes, whatever order the work is picked up in.

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use genos_bake::{BakeMode, BakeSettings};
use genos_cook::{cook_textures_with, cook_with, encode, encode_textures, map_by_cost, Threads};
use genos_load::{import_file, sha256, Image};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../load/fixtures")
        .join(name)
}

fn threads(n: usize) -> Threads {
    Threads(NonZeroUsize::new(n).unwrap())
}

const COUNTS: [usize; 4] = [1, 2, 3, 8];

#[test]
fn map_keeps_item_order_for_any_thread_count() {
    let items: Vec<u64> = (0..97).map(|i| (i * 37) % 11).collect();
    let one = map_by_cost(
        &items,
        Threads::ONE,
        |&c| usize::try_from(c).unwrap(),
        |&c| c * c,
    );
    for n in COUNTS {
        let many = map_by_cost(
            &items,
            threads(n),
            |&c| usize::try_from(c).unwrap(),
            |&c| c * c,
        );
        assert_eq!(many, one, "{n} threads");
    }
}

/// A gradient image whose content depends on `seed`.
fn image(width: u32, height: u32, seed: u32) -> Image {
    let mut pixels = Vec::new();
    for y in 0..height {
        for x in 0..width {
            for c in 0..4u32 {
                let v = (x * 7 + y * 13 + c * 29 + seed * 31) % 256;
                pixels.push(u8::try_from(v).unwrap());
            }
        }
    }
    Image {
        width,
        height,
        pixels,
    }
}

#[test]
fn textures_hash_the_same_on_one_thread_and_many() {
    let mut doc = import_file(fixture("materials.gltf")).unwrap();
    let scene = doc.scene.as_mut().unwrap();
    for (i, (w, h)) in [(64, 64), (5, 3), (130, 33), (16, 256), (1, 1), (40, 41)]
        .into_iter()
        .enumerate()
    {
        scene.images.push(image(w, h, u32::try_from(i).unwrap()));
    }
    let one = sha256(&encode_textures(
        &cook_textures_with(&doc, Threads::ONE).unwrap(),
    ));
    for n in COUNTS {
        let many = sha256(&encode_textures(
            &cook_textures_with(&doc, threads(n)).unwrap(),
        ));
        assert_eq!(many, one, "{n} threads");
    }
}

#[test]
fn meshes_hash_the_same_on_one_thread_and_many() {
    let dir = std::env::temp_dir().join(format!("genos-cook-parallel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let settings = BakeSettings {
        mode: BakeMode::OnLoad,
        cache_dir: dir.join("bake"),
        ..BakeSettings::default()
    };
    for name in ["cube.gltf", "tri.gltf", "quad.glb"] {
        let mut doc = import_file(fixture(name)).unwrap();
        // More meshes than threads, so the work order differs between counts.
        let scene = doc.scene.as_mut().unwrap();
        let first = scene.meshes[0].clone();
        scene.meshes.extend(std::iter::repeat_n(first, 5));
        let one = sha256(&encode(&cook_with(&doc, &settings, Threads::ONE).unwrap()));
        let _ = std::fs::remove_dir_all(dir.join("bake"));
        for n in COUNTS {
            let many = sha256(&encode(&cook_with(&doc, &settings, threads(n)).unwrap()));
            assert_eq!(many, one, "{name}, {n} threads");
        }
    }
}
