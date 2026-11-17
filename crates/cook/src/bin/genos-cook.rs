//! Cook glTF files into `.gcook` records and a package manifest, and check a package.
//!
//! `genos-cook [--out DIR] [--texel METRES] [--voxels-per-side N] FILE.gltf|FILE.glb ...`
//! cooks each file into `DIR/cooked/<asset>.gcook` (default `target/cook`), stores
//! the source objects under `DIR/objects/`, and writes `DIR/package.manifest`.
//! An input whose record already matches every input hash and the settings is
//! reported `up to date` and not rebuilt.
//!
//! `--threads N` caps the threads (default every core). Assets, meshes and
//! images are spread over them; the output bytes are the same for any count.
//!
//! `--textures` also cooks every image into `DIR/cooked/<asset>.gtex` (BC7, BC5
//! for normals, full mips), with the same up-to-date rule. Off by default.
//!
//! `genos-cook --verify DIR` is the load-time check: it hashes every record,
//! texture record and object the manifest names, decodes each record, and
//! prints the timings.
//!
//! `genos-cook --compare FILE` cooks one file in memory and compares the loaded
//! record with a direct `load_gltf_file`, printing both times. Exit 1 on a difference.

use std::path::{Path, PathBuf};
use std::time::Instant;

use genos_bake::{BakeMode, BakeSettings};
use genos_cook::{
    cook, cook_asset, cooked_path, decode, encode, load_cooked, load_textures, map_by_cost,
    package, textures_path, AssetCook, Manifest, Outcome, Threads,
};

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("genos-cook: {err}");
            std::process::ExitCode::FAILURE
        }
    }
}

enum Mode {
    Cook,
    Verify(PathBuf),
    Compare,
}

fn run() -> Result<(), String> {
    let mut settings = BakeSettings {
        mode: BakeMode::OnLoad,
        ..BakeSettings::default()
    };
    let mut out = PathBuf::from("target/cook");
    let mut mode = Mode::Cook;
    let mut files = Vec::new();
    let mut textures = false;
    let mut threads = Threads::all();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--out" => out = value("--out")?.into(),
            "--texel" => settings.card_texel = parse(&value("--texel")?)?,
            "--voxels-per-side" => settings.voxels_per_side = parse(&value("--voxels-per-side")?)?,
            "--verify" => mode = Mode::Verify(value("--verify")?.into()),
            "--compare" => mode = Mode::Compare,
            "--textures" => textures = true,
            "--threads" => {
                threads = Threads(
                    std::num::NonZeroUsize::new(parse(&value("--threads")?)?)
                        .ok_or("--threads needs at least 1")?,
                );
            }
            "-h" | "--help" => {
                println!("genos-cook [--out DIR] [--texel METRES] [--voxels-per-side N] [--textures] [--threads N] FILE ...\ngenos-cook --verify DIR\ngenos-cook --compare FILE ...");
                return Ok(());
            }
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            file => files.push(PathBuf::from(file)),
        }
    }
    match mode {
        Mode::Verify(dir) => verify(&dir),
        Mode::Compare => compare(&files, &settings),
        Mode::Cook => cook_all(&files, &out, &settings, textures, threads),
    }
}

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1e3
}

fn what(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Cooked => "cooked",
        Outcome::UpToDate => "up to date",
    }
}

fn cook_all(
    files: &[PathBuf],
    out: &Path,
    settings: &BakeSettings,
    textures: bool,
    threads: Threads,
) -> Result<(), String> {
    if files.is_empty() {
        return Err("no input files (try --help)".into());
    }
    let mut files = files.to_vec();
    files.sort();
    files.dedup();
    let total = Instant::now();
    let cooked = map_by_cost(
        &files,
        threads,
        |f| std::fs::metadata(f).map_or(0, |m| usize::try_from(m.len()).unwrap_or(usize::MAX)),
        |file| {
            let start = Instant::now();
            cook_asset(file, out, settings, textures, threads)
                .map(|c| (c, ms(start)))
                .map_err(|e| format!("{}: {e}", file.display()))
        },
    );
    let mut records = Vec::new();
    for (file, result) in files.iter().zip(cooked) {
        let (AssetCook { record, textures }, cook_ms) = result?;
        let (outcome, path) = record;
        let size = std::fs::metadata(&path).map_or(0, |m| m.len());
        println!(
            "{}: {} {} ({size} bytes) {cook_ms:.2} ms",
            file.display(),
            what(outcome),
            path.display(),
        );
        let gtex = textures
            .map(|(outcome, path)| report_textures(file, outcome, &path))
            .transpose()?;
        records.push((load_cooked(&path).map_err(|e| e.to_string())?, gtex));
    }
    let entries: Vec<_> = records.iter().map(|(c, g)| (c, *g)).collect();
    let text = package(&entries).text();
    genos_load::write_atomic(&out.join("package.manifest"), text.as_bytes())
        .map_err(|e| e.to_string())?;
    println!(
        "package: {} assets, {} threads, {:.2} ms",
        records.len(),
        threads.get(),
        ms(total)
    );
    Ok(())
}

/// Print a texture record's size and checked-load time; return its file hash.
fn report_textures(file: &Path, outcome: Outcome, path: &Path) -> Result<[u8; 32], String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let start = Instant::now();
    let t = genos_cook::decode_textures(&bytes).map_err(|e| e.to_string())?;
    let load_ms = ms(start);
    let texels: u64 = t
        .textures
        .iter()
        .map(|t| u64::from(t.ktx2.width) * u64::from(t.ktx2.height))
        .sum();
    println!(
        "{}: textures {} {} ({} images, {texels} base texels, {} bytes), checked load {load_ms:.2} ms",
        file.display(),
        what(outcome),
        path.display(),
        t.textures.len(),
        bytes.len()
    );
    Ok(genos_load::sha256(&bytes))
}

fn verify(dir: &Path) -> Result<(), String> {
    let start = Instant::now();
    let text = std::fs::read_to_string(dir.join("package.manifest")).map_err(|e| e.to_string())?;
    let m = Manifest::parse(&text).map_err(|e| e.to_string())?;
    let n = m.verify(dir).map_err(|e| e.to_string())?;
    let checked = ms(start);
    let load = Instant::now();
    let mut triangles = 0;
    let mut images = 0;
    for line in &m.lines {
        let id = line.split(' ').next().unwrap_or_default();
        let path = cooked_path(dir, &unhex(id)?);
        let c = load_cooked(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if line.contains(" gtex=") {
            let tex = textures_path(dir, &unhex(id)?);
            images += load_textures(&tex)
                .map_err(|e| format!("{}: {e}", tex.display()))?
                .textures
                .len();
        }
        triangles += c
            .meshes
            .iter()
            .flat_map(|m| &m.mesh.primitives)
            .map(|p| p.indices.len() / 3)
            .sum::<usize>();
    }
    println!(
        "verify: {n} assets ok, {triangles} triangles, {images} cooked images, hash check {checked:.2} ms, decode {:.2} ms",
        ms(load)
    );
    Ok(())
}

fn compare(files: &[PathBuf], settings: &BakeSettings) -> Result<(), String> {
    let mut same = true;
    for file in files {
        let start = Instant::now();
        let mut phases = genos_load::LoadPhases::default();
        let direct =
            genos_load::load_gltf_file_phased(file, &mut phases).map_err(|e| e.to_string())?;
        let direct_ms = ms(start);
        let pms = |d: std::time::Duration| d.as_secs_f64() * 1e3;
        let doc = genos_load::import_file(file).map_err(|e| e.to_string())?;
        let bytes = encode(&cook(&doc, settings).map_err(|e| e.to_string())?);
        let start = Instant::now();
        let loaded = decode(&bytes).map_err(|e| e.to_string())?;
        let cooked_ms = ms(start);
        let meshes: Vec<_> = loaded.meshes.iter().map(|m| m.mesh.clone()).collect();
        let ok = meshes == direct.meshes && loaded.instances == direct.instances;
        same &= ok;
        println!(
            "{}: {} direct {direct_ms:.2} ms (import {:.2}, decode {:.2}, meshes {:.2}, mips {:.2}), cooked {cooked_ms:.2} ms, {} bytes",
            file.display(),
            if ok { "same" } else { "DIFFERENT" },
            pms(phases.import),
            pms(phases.decode),
            pms(phases.meshes),
            pms(phases.mips),
            bytes.len()
        );
    }
    if same {
        Ok(())
    } else {
        Err("cooked geometry differs from the direct import".into())
    }
}

fn unhex(s: &str) -> Result<[u8; 32], String> {
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = s
            .get(i * 2..i * 2 + 2)
            .and_then(|d| u8::from_str_radix(d, 16).ok())
            .ok_or("bad asset id in manifest")?;
    }
    Ok(out)
}

fn parse<T: std::str::FromStr>(s: &str) -> Result<T, String> {
    s.parse().map_err(|_| format!("bad number {s}"))
}
