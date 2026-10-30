//! Cook glTF files into `.gcook` records and a package manifest, and check a package.
//!
//! `genos-cook [--out DIR] [--texel METRES] [--voxels-per-side N] FILE.gltf|FILE.glb ...`
//! cooks each file into `DIR/cooked/<asset>.gcook` (default `target/cook`), stores
//! the source objects under `DIR/objects/`, and writes `DIR/package.manifest`.
//! An input whose record already matches every input hash and the settings is
//! reported `up to date` and not rebuilt.
//!
//! `--textures` also cooks every image into `DIR/cooked/<asset>.gtex` (BC7, BC5
//! for normals, full mips), with the same up-to-date rule. Off by default.
//!
//! `genos-cook --verify DIR` is the load-time check: it hashes every record and
//! object the manifest names, decodes each record, and prints the timings.
//!
//! `genos-cook --compare FILE` cooks one file in memory and compares the loaded
//! record with a direct `load_gltf_file`, printing both times. Exit 1 on a difference.

use std::path::{Path, PathBuf};
use std::time::Instant;

use genos_bake::{BakeMode, BakeSettings};
use genos_cook::{
    cook, cook_file, cook_textures_file, cooked_path, decode, encode, load_cooked, load_textures,
    manifest, Manifest, Outcome,
};

fn main() {
    if let Err(err) = run() {
        eprintln!("genos-cook: {err}");
        std::process::exit(1);
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
            "-h" | "--help" => {
                println!("genos-cook [--out DIR] [--texel METRES] [--voxels-per-side N] [--textures] FILE ...\ngenos-cook --verify DIR\ngenos-cook --compare FILE ...");
                return Ok(());
            }
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            file => files.push(PathBuf::from(file)),
        }
    }
    match mode {
        Mode::Verify(dir) => verify(&dir),
        Mode::Compare => compare(&files, &settings),
        Mode::Cook => cook_all(&files, &out, &settings, textures),
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
) -> Result<(), String> {
    if files.is_empty() {
        return Err("no input files (try --help)".into());
    }
    let total = Instant::now();
    let mut records = Vec::new();
    for file in files {
        let start = Instant::now();
        let (outcome, path) =
            cook_file(file, out, settings).map_err(|e| format!("{}: {e}", file.display()))?;
        let size = std::fs::metadata(&path).map_or(0, |m| m.len());
        println!(
            "{}: {} {} ({size} bytes) {:.2} ms",
            file.display(),
            what(outcome),
            path.display(),
            ms(start)
        );
        records.push(load_cooked(&path).map_err(|e| e.to_string())?);
        if textures {
            cook_textures(file, out)?;
        }
    }
    let text = manifest(&records).text();
    genos_load::write_atomic(&out.join("package.manifest"), text.as_bytes())
        .map_err(|e| e.to_string())?;
    println!("package: {} assets, {:.2} ms", records.len(), ms(total));
    Ok(())
}

fn cook_textures(file: &Path, out: &Path) -> Result<(), String> {
    let start = Instant::now();
    let (outcome, path) =
        cook_textures_file(file, out).map_err(|e| format!("{}: {e}", file.display()))?;
    let cook_ms = ms(start);
    let start = Instant::now();
    let t = load_textures(&path).map_err(|e| e.to_string())?;
    let load_ms = ms(start);
    let size = std::fs::metadata(&path).map_or(0, |m| m.len());
    let texels: u64 = t
        .textures
        .iter()
        .map(|t| u64::from(t.ktx2.width) * u64::from(t.ktx2.height))
        .sum();
    println!(
        "{}: textures {} {} ({} images, {texels} base texels, {size} bytes) {cook_ms:.2} ms, checked load {load_ms:.2} ms",
        file.display(),
        what(outcome),
        path.display(),
        t.textures.len()
    );
    Ok(())
}

fn verify(dir: &Path) -> Result<(), String> {
    let start = Instant::now();
    let text = std::fs::read_to_string(dir.join("package.manifest")).map_err(|e| e.to_string())?;
    let m = Manifest::parse(&text).map_err(|e| e.to_string())?;
    let n = m.verify(dir).map_err(|e| e.to_string())?;
    let checked = ms(start);
    let load = Instant::now();
    let mut triangles = 0;
    for line in &m.lines {
        let id = line.split(' ').next().unwrap_or_default();
        let path = cooked_path(dir, &unhex(id)?);
        let c = load_cooked(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        triangles += c
            .meshes
            .iter()
            .flat_map(|m| &m.mesh.primitives)
            .map(|p| p.indices.len() / 3)
            .sum::<usize>();
    }
    println!(
        "verify: {n} assets ok, {triangles} triangles, hash check {checked:.2} ms, decode {:.2} ms",
        ms(load)
    );
    Ok(())
}

fn compare(files: &[PathBuf], settings: &BakeSettings) -> Result<(), String> {
    let mut same = true;
    for file in files {
        let start = Instant::now();
        let direct = genos_load::load_gltf_file(file).map_err(|e| e.to_string())?;
        let direct_ms = ms(start);
        let doc = genos_load::import_file(file).map_err(|e| e.to_string())?;
        let bytes = encode(&cook(&doc, settings).map_err(|e| e.to_string())?);
        let start = Instant::now();
        let loaded = decode(&bytes).map_err(|e| e.to_string())?;
        let cooked_ms = ms(start);
        let meshes: Vec<_> = loaded.meshes.iter().map(|m| m.mesh.clone()).collect();
        let ok = meshes == direct.meshes && loaded.instances == direct.instances;
        same &= ok;
        println!(
            "{}: {} direct {direct_ms:.2} ms, cooked {cooked_ms:.2} ms, {} bytes",
            file.display(),
            if ok { "same" } else { "DIFFERENT" },
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
