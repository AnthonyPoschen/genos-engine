//! Build the GI v2 import data (mesh SDFs and cards) for glTF files ahead of time.
//!
//! `genos-bake [--out DIR] [--texel METRES] [--voxels-per-side N] [--force] FILE.gltf|FILE.glb ...`
//!
//! Writes one `.gbake` per mesh into the cache (default `target/bake-cache`, or
//! `GENOS_BAKE_CACHE`), keyed by content, so a game in `Prebuilt` mode finds it.
//! A mesh already baked with the same content and settings is skipped unless `--force`.

use std::time::Instant;

use genos_bake::{cache_path, read, rebake, BakeMesh, BakeMode, BakeSettings};

fn main() {
    if let Err(err) = run() {
        eprintln!("genos-bake: {err}");
        std::process::exit(1);
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
    for file in files {
        let scene = genos_load::load_gltf_file(&file).map_err(|e| format!("{file}: {e}"))?;
        for index in 0..scene.meshes.len() {
            let mesh = BakeMesh::from_gltf(&scene, index);
            let path = cache_path(&mesh, &settings);
            if !force && read(&path).is_some() {
                println!("{file} {}: up to date ({})", mesh.name, path.display());
                continue;
            }
            let start = Instant::now();
            let (baked, path) = rebake(&mesh, &settings).map_err(|e| e.to_string())?;
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
    Ok(())
}

fn parse<T: std::str::FromStr>(text: &str) -> Result<T, String> {
    text.parse().map_err(|_| format!("not a number: {text}"))
}
