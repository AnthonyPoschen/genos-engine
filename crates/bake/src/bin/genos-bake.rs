#![expect(
    clippy::as_conversions,
    reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md"
)]
#![expect(
    clippy::cast_precision_loss,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]

//! Build the GI import data (mesh SDFs and cards) for glTF files ahead of time.
//!
//! `genos-bake [--out DIR] [--texel METRES] [--voxels-per-side N] [--force] FILE.gltf|FILE.glb ...`
//!
//! Writes one `.gbake` per mesh into the cache (default `target/bake-cache`, or
//! `GENOS_BAKE_CACHE`), keyed by content, so a game in `Prebuilt` mode finds it.
//! A mesh already baked with the same content and settings is skipped unless `--force`.

use std::time::Instant;

use genos_bake::{key_path, read, rebake_phased, BakeMesh, BakeMode, BakePhases, BakeSettings};
use genos_load::LoadPhases;

fn main() {
    if let Err(err) = run() {
        eprintln!("genos-bake: {err}");
        genos_window::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut settings = BakeSettings {
        mode: BakeMode::OnLoad,
        ..BakeSettings::default()
    };
    let mut force = false;
    let mut files = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--out" => settings.cache_dir = value("--out")?.into(),
            "--texel" => settings.card_texel = parse(&value("--texel")?)?,
            "--voxels-per-side" => settings.voxels_per_side = parse(&value("--voxels-per-side")?)?,
            "--force" => force = true,
            "-h" | "--help" => {
                println!("genos-bake [--out DIR] [--texel METRES] [--voxels-per-side N] [--force] FILE.gltf|FILE.glb ...");
                return Ok(());
            }
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            file => files.push(file.to_string()),
        }
    }
    if files.is_empty() {
        return Err("no input files (try --help)".into());
    }
    let total = Instant::now();
    let mut load = LoadPhases::default();
    let mut bake = BakePhases::default();
    let mut prepare = std::time::Duration::ZERO;
    for file in files {
        let scene =
            genos_load::load_gltf_file_phased(&file, &mut load).map_err(|e| format!("{file}: {e}"))?;
        for index in 0..scene.meshes.len() {
            let start = Instant::now();
            let mesh = BakeMesh::from_gltf(&scene, index);
            let key = mesh.content_key(&settings);
            prepare += start.elapsed();
            let path = key_path(&settings, key);
            if !force && read(&path).is_some() {
                println!("{file} {}: up to date ({})", mesh.name, path.display());
                continue;
            }
            let start = Instant::now();
            let (baked, path) = rebake_phased(&mesh, &settings, key, &mut bake).map_err(|e| e.to_string())?;
            let sdf = &baked.sdf;
            println!(
                "{file} {}: {} triangles, voxel {:.3} m, {} bricks{} ({:.1} MB), {} cards ({} texels), {:.2} s -> {}",
                mesh.name,
                mesh.triangles.len(),
                sdf.voxel,
                sdf.bricks.len(),
                if sdf.thin { ", open mesh: unsigned" } else { "" },
                sdf.bytes() as f64 / 1e6,
                baked.cards.cards.len(),
                baked.cards.texel_count(),
                start.elapsed().as_secs_f64(),
                path.display()
            );
        }
    }
    let s = |d: std::time::Duration| d.as_secs_f64();
    println!(
        "phases: import {:.2} s, decode {:.2} s, meshes {:.2} s, mips {:.2} s, prepare {:.2} s, bvh {:.2} s, sdf {:.2} s, cards {:.2} s, write {:.2} s, total {:.2} s",
        s(load.import),
        s(load.decode),
        s(load.meshes),
        s(load.mips),
        s(prepare),
        s(bake.bvh),
        s(bake.sdf),
        s(bake.cards),
        s(bake.write),
        s(total.elapsed())
    );
    Ok(())
}

fn parse<T: std::str::FromStr>(text: &str) -> Result<T, String> {
    text.parse().map_err(|_| format!("not a number: {text}"))
}
