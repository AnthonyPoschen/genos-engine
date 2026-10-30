#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")
)]
#![allow(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::many_single_char_names,
    clippy::chunks_exact_to_as_chunks,
    reason = "error metrics over small fixture images"
)]

//! Texture cook: BC7 and BC5 decode back within a bound of the source mips,
//! an unchanged input is not rebuilt, and a damaged record is refused.
//!
//! Bounds (decoded with `bcdec_rs`, against `genos_load::mip_chain` of the
//! same space, every level):
//! - Level 0: PSNR at least 40 dB (BC7 over RGBA, BC5 over X and Y). BC7: no
//!   channel off by more than 32 of 255. BC5: the rebuilt normal within 4
//!   degrees of the source normal on average.
//! - Every smaller level: PSNR at least 22 dB. The 37×29 colour gradient's
//!   5×4 level is the worst fixture at 24.4 dB: two blocks of nearly all
//!   distinct colours.

use std::path::{Path, PathBuf};

use genos_cook::ktx2::{BlockFormat, Ktx2};
use genos_cook::{
    cook_textures, cook_textures_file, decode_textures, encode_image, encode_textures,
    load_textures, CookError, Outcome,
};
use genos_load::{decode_image, import_file, mip_chain, normal_from_bytes, Image, MipSpace};

const MIN_PSNR: f64 = 40.0;
const MIN_PSNR_MIP: f64 = 22.0;
const MAX_ABS: u8 = 32;
const MAX_MEAN_DEGREES: f64 = 4.0;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../load/fixtures")
        .join(name)
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("genos-tex-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Decode one level to RGBA8 (BC5 gives R, G, 0, 255), cropped to `w`×`h`.
fn decode_level(format: BlockFormat, blocks: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (bw, bh) = (w.div_ceil(4) as usize, h.div_ceil(4) as usize);
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![0u8; w * h * 4];
    for (i, block) in blocks.chunks_exact(16).enumerate() {
        let (bx, by) = (i % bw, i / bw);
        assert!(by < bh);
        let mut texels = [0u8; 64];
        if format == BlockFormat::Bc5Unorm {
            let mut rg = [0u8; 32];
            bcdec_rs::bc5(block, &mut rg, 8, false);
            for t in 0..16 {
                texels[t * 4..t * 4 + 4].copy_from_slice(&[rg[t * 2], rg[t * 2 + 1], 0, 255]);
            }
        } else {
            bcdec_rs::bc7(block, &mut texels, 16);
        }
        for t in 0..16 {
            let (x, y) = (bx * 4 + t % 4, by * 4 + t / 4);
            if x < w && y < h {
                let at = (y * w + x) * 4;
                out[at..at + 4].copy_from_slice(&texels[t * 4..t * 4 + 4]);
            }
        }
    }
    out
}

fn psnr(a: &[u8], b: &[u8], channels: &[usize]) -> f64 {
    let (mut sum, mut n) = (0.0, 0.0);
    for (pa, pb) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        for &c in channels {
            let d = f64::from(pa[c]) - f64::from(pb[c]);
            sum += d * d;
            n += 1.0;
        }
    }
    let mse = sum / n;
    if mse == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (255.0f64 * 255.0 / mse).log10()
    }
}

/// Check every level of `k` against the source mips; returns level-0 PSNR.
fn check(name: &str, image: &Image, space: MipSpace, k: &Ktx2) -> f64 {
    let mips = mip_chain(image, space);
    assert_eq!(k.levels.len(), mips.len(), "{name}");
    let channels: &[usize] = if space == MipSpace::Normal {
        &[0, 1]
    } else {
        &[0, 1, 2, 3]
    };
    let mut first = 0.0;
    for (level, (blocks, mip)) in k.levels.iter().zip(&mips).enumerate() {
        let got = decode_level(k.format, blocks, mip.width, mip.height);
        let p = psnr(&got, &mip.pixels, channels);
        let floor = if level == 0 { MIN_PSNR } else { MIN_PSNR_MIP };
        assert!(p >= floor, "{name} level {level}: {p:.1} dB");
        if level > 0 {
            continue;
        }
        first = p;
        if space == MipSpace::Normal {
            let mut degrees = 0.0;
            for (g, s) in got.chunks_exact(4).zip(mip.pixels.chunks_exact(4)) {
                let s = normal_from_bytes([s[0], s[1], s[2], s[3]]);
                let (x, y) = (f64::from(g[0]) / 127.5 - 1.0, f64::from(g[1]) / 127.5 - 1.0);
                let z = (1.0 - x * x - y * y).max(0.0).sqrt();
                let dot = x * f64::from(s[0]) + y * f64::from(s[1]) + z * f64::from(s[2]);
                let len = (x * x + y * y + z * z).sqrt().max(1e-9);
                degrees += (dot / len).clamp(-1.0, 1.0).acos().to_degrees();
            }
            let mean = degrees / (got.len() / 4) as f64;
            assert!(mean <= MAX_MEAN_DEGREES, "{name}: {mean:.2} degrees");
        } else {
            let worst = got
                .iter()
                .zip(&mip.pixels)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap_or(0);
            assert!(worst <= MAX_ABS, "{name}: off by {worst}");
        }
    }
    first
}

/// A hemisphere of normals, encoded the glTF way.
fn normal_map(size: u32) -> Image {
    let mut pixels = Vec::new();
    for y in 0..size {
        for x in 0..size {
            let u = (f64::from(x) + 0.5) / f64::from(size) * 1.6 - 0.8;
            let v = (f64::from(y) + 0.5) / f64::from(size) * 1.6 - 0.8;
            let z = (1.0 - u * u - v * v).max(0.0).sqrt();
            let l = (u * u + v * v + z * z).sqrt();
            for c in [u / l, v / l, z / l] {
                pixels.push(((c * 0.5 + 0.5) * 255.0).round() as u8);
            }
            pixels.push(255);
        }
    }
    Image {
        width: size,
        height: size,
        pixels,
    }
}

#[test]
fn bc7_and_bc5_decode_within_bound_of_source_mips() {
    let mut cases = Vec::new();
    for name in [
        "gradient-444.jpg",
        "gradient-420.jpg",
        "gradient-grey.jpg",
        "quad.png",
    ] {
        let image = decode_image(&std::fs::read(fixture(name)).unwrap()).unwrap();
        for space in [MipSpace::Srgb, MipSpace::Linear] {
            cases.push((format!("{name} {space:?}"), image.clone(), space));
        }
    }
    cases.push(("hemisphere 64".into(), normal_map(64), MipSpace::Normal));
    cases.push(("hemisphere 37".into(), normal_map(37), MipSpace::Normal));
    for (name, image, space) in &cases {
        let k = encode_image(image, *space);
        let want = match space {
            MipSpace::Srgb => BlockFormat::Bc7Srgb,
            MipSpace::Linear => BlockFormat::Bc7Unorm,
            MipSpace::Normal => BlockFormat::Bc5Unorm,
        };
        assert_eq!(k.format, want, "{name}");
        let p = check(name, image, *space, &k);
        println!("{name}: {}x{} level 0 {p:.1} dB", image.width, image.height);
    }
}

#[test]
fn gltf_textures_take_the_space_of_their_slot_and_round_trip() {
    for name in ["materials.gltf", "quad.glb"] {
        let doc = import_file(fixture(name)).unwrap();
        let cooked = cook_textures(&doc).unwrap();
        let scene = doc.scene.as_ref().unwrap();
        assert_eq!(cooked.textures.len(), scene.images.len(), "{name}");
        assert_eq!(decode_textures(&encode_textures(&cooked)).unwrap(), cooked);
        let spaces = genos_load::image_spaces(scene.images.len(), &scene.materials);
        for (t, space) in cooked.textures.iter().zip(spaces) {
            let image = &scene.images[t.image as usize];
            check(name, image, space, &t.ktx2);
        }
    }
}

#[test]
fn unchanged_input_is_not_rebuilt_and_a_changed_image_is() {
    let dir = temp("incremental");
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    for f in ["materials.gltf", "quad.png"] {
        std::fs::copy(fixture(f), src.join(f)).unwrap();
    }
    let out = dir.join("out");
    let gltf = src.join("materials.gltf");
    let (first, path) = cook_textures_file(&gltf, &out).unwrap();
    assert_eq!(first, Outcome::Cooked);
    let before = std::fs::metadata(&path).unwrap().modified().unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let (second, again) = cook_textures_file(&gltf, &out).unwrap();
    assert_eq!((second, &again), (Outcome::UpToDate, &path));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        before
    );

    let mut changed = std::fs::read(src.join("quad.png")).unwrap();
    changed.extend_from_slice(b"\0");
    std::fs::write(src.join("quad.png"), &changed).unwrap();
    let (third, same_path) = cook_textures_file(&gltf, &out).unwrap();
    assert_eq!((third, &same_path), (Outcome::Cooked, &path));
    assert_ne!(std::fs::read(&path).unwrap(), bytes);
}

#[test]
fn damaged_record_is_refused() {
    let doc = import_file(fixture("materials.gltf")).unwrap();
    let bytes = encode_textures(&cook_textures(&doc).unwrap());
    let dir = temp("damaged");
    let path = dir.join("x.gtex");
    for at in [0, 5, bytes.len() / 2, bytes.len() - 1] {
        let mut bad = bytes.clone();
        bad[at] ^= 1;
        std::fs::write(&path, &bad).unwrap();
        assert!(
            matches!(load_textures(&path), Err(CookError::Corrupt(_))),
            "{at}"
        );
    }
    assert!(decode_textures(&bytes[..bytes.len() - 1]).is_err());
}
