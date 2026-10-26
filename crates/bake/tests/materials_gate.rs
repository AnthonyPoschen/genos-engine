//! Headless gate for the PBR material design.
//!
//! ```text
//! cargo test -p genos-bake --test materials_gate -- --case good
//! cargo test -p genos-bake --test materials_gate -- --case bad
//! cargo test -p genos-bake --test materials_gate -- --nocapture --case good
//! ```
//!
//! Prints one line per metric: `name value threshold STATUS`.
//! Writes `DIR/<case>.json`. The default directory is `genos-debug/materials`.
//! `GENOS_MATERIALS_GATE_CASE` and `GENOS_MATERIALS_GATE_OUT` apply only when
//! the matching argument was omitted. Exit 0 when nothing FAILs. Exit 1 when
//! a metric FAILs. `NOT-IMPLEMENTED` is not a failure. Exit 2 on a bad
//! argument, a failed self-check, or a write error.
//!
//! `--nocapture` is a libtest flag. `cargo test -- --nocapture` forwards it
//! to this binary, which is not a libtest harness. The flag is accepted and
//! ignored, as are `--test-threads`, `--skip`, and `--format`. The lines
//! above are already printed.
//!
//! The fixture is `tests/fixtures/materials/panel.gltf`, loaded with
//! `load_gltf_file`. `--case good` reads `BakeMesh::surface` and
//! `BakeMesh::from_gltf`. `--case bad` reads a shade in this file that
//! decodes sRGB twice, skips `(1 - metallic)`, and skips emissive strength.
//! Expected colours are the constants below. They are the IEC 61966-2-1
//! values worked out by hand. This file does not build them by calling the
//! shade.
//!
//! The good case also runs the broken shade and exits 2 if that shade would
//! pass a shade metric, or if a path that is not in the tree comes back as PASS.

use std::fmt::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use genos_bake::BakeMesh;
use genos_load::{
    load_gltf_file, mib_f32, normal_from_bytes, plan_residency, GltfScene, HotReload, ImageChain,
    Mip,
};

const SHADE: f32 = 1.0 / 255.0;
const SRGB: f32 = 1.0e-5;
const PICTURE: f32 = 0.12;
const RELATIVE: f32 = 0.02;
const BC7: f32 = 0.02;
const ASTC: f32 = 0.0;
const RESIDENCY_MB: f32 = 256.0;
const RELOAD_FRAMES: f32 = 1.0;
const GBUFFER_MS: f32 = 0.60;
const NORMAL_DEG: f32 = 0.5;
/// A sample this close to the twice-decoded constant is a double decode.
const DOUBLE_MARGIN: f32 = 0.05;

// IEC 61966-2-1, worked by hand. c = N/255.
// c <= 0.04045 → c/12.92, else ((c + 0.055) / 1.055)^2.4.
// Byte 10:  c = 10/255 = 0.039215686275 <= 0.04045
//           L = 0.039215686275 / 12.92 = 0.003035269835
// Byte 188: c = 188/255 = 0.737254901961
//           (c + 0.055) / 1.055 = 0.750952513707
//           ^2.4 = 0.502886458033
// Byte 128: c = 128/255 = 0.501960784314
//           (c + 0.055) / 1.055 = 0.527924914041
//           ^2.4 = 0.215860500114
// Byte 200: c = 200/255 = 0.784313725490
//           (c + 0.055) / 1.055 = 0.795558033640
//           ^2.4 = 0.577580440430
// Applying the same curve again, as a second mistaken decode:
// Byte 188: (0.502886458033 + 0.055) / 1.055 = 0.528802329889, ^2.4 = 0.216722530978
// Byte 128: (0.215860500114 + 0.055) / 1.055 = 0.256739810535, ^2.4 = 0.038263414225
// Metallic texel G = 255, B = 128, factor 1. Diffuse scale is 1 - 128/255 = 127/255.
// Emissive strength 2 times the byte-128 decode = 0.431721000228.
const ONCE_10: f32 = 0.003_035_27;
const ONCE_128: f32 = 0.215_860_5;
const ONCE_188: f32 = 0.502_886_5;
const ONCE_200: f32 = 0.577_580_45;
const TWICE_128: f32 = 0.038_263_414;
const TWICE_188: f32 = 0.216_722_53;
const METAL_KEEP: f32 = 0.498_039_22;
const EMIT_STRENGTH_2: f32 = 0.431_721;

const SHADE_MAT: usize = 0;
const BRIGHT_MAT: usize = 2;
const METAL_MAT: usize = 3;

const MUST_FAIL: [&str; 11] = [
    "shade_albedo",
    "shade_emissive",
    "srgb_decode",
    "srgb_double",
    "metal_diffuse",
    "mr_channels",
    "texcoord_sample",
    "roughness_from_g",
    "mip_chain",
    "residency_mb",
    "hot_reload_frames",
];

/// Measured from the loaded file or from `from_gltf`, not from the broken shade.
/// `texel_alpha` is the raw texel. The broken reader returns that byte too, so it
/// passes, and the self-check allows it the way it allows `mask_alpha`.
const FILE_METRICS: [&str; 4] = ["mask_alpha", "texcoord_set", "emissive_default", "texel_alpha"];

struct Sample {
    material: usize,
    uv: [f32; 2],
    albedo: [f32; 3],
    emissive: [f32; 3],
}

struct Metric {
    name: &'static str,
    value: Option<f32>,
    threshold: f32,
    status: Status,
    note: Option<&'static str>,
    reference: Option<f64>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Pass,
    Fail,
    NotImplemented,
}

impl Status {
    fn text(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::NotImplemented => "NOT-IMPLEMENTED",
        }
    }
}

#[derive(Clone, Copy)]
enum Reader {
    Bake,
    Broken,
}

impl Reader {
    fn text(self) -> &'static str {
        match self {
            Self::Bake => "bake",
            Self::Broken => "broken",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Case {
    Good,
    Bad,
}

impl Case {
    fn reader(self) -> Reader {
        match self {
            Self::Good => Reader::Bake,
            Self::Bad => Reader::Broken,
        }
    }

    fn text(self) -> &'static str {
        match self {
            Self::Good => "good",
            Self::Bad => "bad",
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("materials_gate: {err}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let args = parse()?;
    let Some(args) = args else {
        println!(
            "materials_gate --case good|bad [--out DIR]\n\
             Loads tests/fixtures/materials/panel.gltf through genos-load.\n\
             --case good reads BakeMesh::surface. --case bad reads a shade that\n\
             decodes sRGB twice, skips (1 - metallic), and skips emissive strength.\n\
             --nocapture, --test-threads, --skip, and --format are ignored.\n\
             Writes DIR/<case>.json. Default DIR is genos-debug/materials.\n\
             GENOS_MATERIALS_GATE_CASE and GENOS_MATERIALS_GATE_OUT apply only when\n\
             the matching argument was omitted.\n\
             Exit 0 when no metric FAILs. Exit 1 on a FAIL. NOT-IMPLEMENTED is not a failure."
        );
        return Ok(ExitCode::SUCCESS);
    };
    let scene = load_scene()?;
    if args.case == Case::Good {
        if let Some(err) = broken_is_wrong(&evaluate(Reader::Broken, &scene)?) {
            return Err(err);
        }
    }
    let metrics = evaluate(args.case.reader(), &scene)?;
    for metric in &metrics {
        println!("{}", line(metric));
    }
    let path = args.out.join(format!("{}.json", args.case.text()));
    write_json(&path, args.case, &metrics)?;
    let failed = metrics.iter().any(|metric| metric.status == Status::Fail);
    Ok(ExitCode::from(u8::from(failed)))
}

struct Args {
    case: Case,
    out: PathBuf,
}

fn parse() -> Result<Option<Args>, String> {
    let mut case = None;
    let mut out = None;
    let mut help = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--case" => {
                let value = args.next().ok_or("--case needs good or bad")?;
                case = Some(parse_case(&value)?);
            }
            "--out" => {
                let value = args.next().ok_or("--out needs a directory")?;
                out = Some(PathBuf::from(value));
            }
            // libtest flags. `cargo test -- --nocapture` forwards them here.
            // This binary is not a libtest harness, so they are ignored.
            "--nocapture"
            | "--quiet"
            | "--exact"
            | "--ignored"
            | "--include-ignored"
            | "--exclude-should-panic"
            | "--force-run"
            | "--show-output"
            | "--list" => {}
            "--test-threads" | "--skip" | "--format" => {
                let _ = args.next();
            }
            other
                if other.starts_with("--test-threads=")
                    || other.starts_with("--skip=")
                    || other.starts_with("--format=") => {}
            "-h" | "--help" => help = true,
            other => return Err(format!("unknown option {other}")),
        }
    }
    if help {
        return Ok(None);
    }
    let case = match case {
        Some(case) => case,
        None => match env_mirror("GENOS_MATERIALS_GATE_CASE")? {
            Some(value) => parse_case(&value)?,
            None => Case::Good,
        },
    };
    let out = match out {
        Some(out) => out,
        None => match env_mirror("GENOS_MATERIALS_GATE_OUT")? {
            Some(value) => PathBuf::from(value),
            None => PathBuf::from("genos-debug/materials"),
        },
    };
    Ok(Some(Args { case, out }))
}

fn parse_case(text: &str) -> Result<Case, String> {
    match text {
        "good" => Ok(Case::Good),
        "bad" => Ok(Case::Bad),
        _ => Err(format!("case must be good or bad, got {text}")),
    }
}

#[allow(
    clippy::disallowed_methods,
    reason = "the CLI argument wins; the variable applies only when the argument was omitted"
)]
fn env_mirror(key: &str) -> Result<Option<String>, String> {
    match std::env::var(key) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{key} is not unicode")),
    }
}

fn load_scene() -> Result<GltfScene, String> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/materials/panel.gltf");
    load_gltf_file(&path).map_err(|err| format!("load {}: {err}", path.display()))
}

fn evaluate(reader: Reader, scene: &GltfScene) -> Result<Vec<Metric>, String> {
    if scene.meshes.is_empty() {
        return Err("fixture has no mesh".to_string());
    }
    let mesh = BakeMesh::from_gltf(scene, 0);
    let albedo = colour_error(reader, scene, &mesh, Colour::Albedo)?;
    let emissive = colour_error(reader, scene, &mesh, Colour::Emissive)?;
    Ok(vec![
        measured("shade_albedo", albedo, SHADE),
        measured("shade_emissive", emissive, SHADE),
        measured("srgb_decode", srgb_once(reader, scene, &mesh)?, SRGB),
        measured("srgb_double", srgb_twice(reader, scene, &mesh)?, SRGB),
        measured("metal_diffuse", metal_value(reader, scene, &mesh)?, SHADE),
        measured("mr_channels", mr_channels(reader, scene, &mesh)?, SHADE),
        measured("mask_alpha", mask_mistakes(&mesh), 0.0),
        measured("texcoord_set", texcoord_mistakes(scene)?, 0.0),
        measured("texcoord_sample", texcoord_sample(reader, scene, &mesh)?, 0.0),
        measured("emissive_default", emissive_default(scene)?, 0.0),
        measured("roughness_from_g", roughness_from_g(reader, scene, &mesh)?, SHADE),
        measured("texel_alpha", texel_alpha(reader, scene, &mesh)?, SHADE),
        pending("draw_mean", SHADE, "genos-render is not linked"),
        pending(
            "occlusion_indirect",
            SHADE,
            "BakeMesh::surface does not return occlusion",
        ),
        pending(
            "normal_angle_deg",
            NORMAL_DEG,
            "the bake shade has no tangent-space normal",
        ),
        pending(
            "picture_dielectric",
            PICTURE,
            "the path tracer does not sample textures",
        ),
        pending("picture_metal", PICTURE, "the specular lobe is not shaded"),
        pending(
            "emissive_furnace",
            RELATIVE,
            "model only; waits on lighting_gpu.rs",
        ),
        pending(
            "double_count",
            RELATIVE,
            "model only; waits on lighting_gpu.rs",
        ),
        measured("mip_chain", mip_chain_error(reader)?, 0.0),
        pending(
            "bc7_colour_rmse",
            BC7,
            "no BC7 encoder is vendored; RGBA8 stays the cook",
        ),
        pending(
            "astc_colour_rmse",
            ASTC,
            "no lossless ASTC transcode is linked",
        ),
        measured("residency_mb", residency_mb(reader), RESIDENCY_MB),
        measured("hot_reload_frames", reload_frames(reader), RELOAD_FRAMES),
        pending("gbuffer_ms", GBUFFER_MS, "no GPU timing in this gate"),
    ])
}

fn broken_is_wrong(metrics: &[Metric]) -> Option<String> {
    for name in MUST_FAIL {
        let status = metrics
            .iter()
            .find(|metric| metric.name == name)
            .map(|metric| metric.status);
        if status != Some(Status::Fail) {
            return Some(format!(
                "gate_self_check {name} did not fail on the broken reader"
            ));
        }
    }
    for metric in metrics {
        if MUST_FAIL.contains(&metric.name) || FILE_METRICS.contains(&metric.name) {
            if metric.value.is_none() {
                return Some(format!("gate_self_check {} was not measured", metric.name));
            }
            continue;
        }
        if metric.status != Status::NotImplemented {
            return Some(format!(
                "gate_self_check {} was {}, expected NOT-IMPLEMENTED",
                metric.name,
                metric.status.text()
            ));
        }
    }
    None
}

fn measured(name: &'static str, value: f32, threshold: f32) -> Metric {
    let status = if value.is_finite() && value <= threshold {
        Status::Pass
    } else {
        Status::Fail
    };
    Metric {
        name,
        value: Some(value),
        threshold,
        status,
        note: None,
        reference: None,
    }
}

fn pending(name: &'static str, threshold: f32, note: &'static str) -> Metric {
    Metric {
        name,
        value: None,
        threshold,
        status: Status::NotImplemented,
        note: Some(note),
        reference: None,
    }
}

fn line(metric: &Metric) -> String {
    match metric.value {
        Some(value) => format!(
            "{} {:.6} {:.6} {}",
            metric.name,
            value,
            metric.threshold,
            metric.status.text()
        ),
        None => match metric.note {
            Some(note) => format!(
                "{} na {:.6} {} {note}",
                metric.name,
                metric.threshold,
                metric.status.text()
            ),
            None => format!(
                "{} na {:.6} {}",
                metric.name,
                metric.threshold,
                metric.status.text()
            ),
        },
    }
}

fn write_json(path: &std::path::Path, case: Case, metrics: &[Metric]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("create {}: {err}", parent.display()))?;
        }
    }
    let mut body = format!(
        "{{\n  \"case\": \"{}\",\n  \"reader\": \"{}\",\n  \"metrics\": [\n",
        case.text(),
        case.reader().text()
    );
    for (index, metric) in metrics.iter().enumerate() {
        if index > 0 {
            body.push_str(",\n");
        }
        body.push_str("    ");
        body.push_str(&metric_json(metric));
    }
    body.push_str("\n  ]\n}\n");
    std::fs::write(path, body).map_err(|err| format!("write {}: {err}", path.display()))
}

fn metric_json(metric: &Metric) -> String {
    let value = match metric.value {
        Some(value) => format!("{value:.6}"),
        None => "null".to_string(),
    };
    let mut line = format!(
        "{{\"name\":\"{}\",\"value\":{value},\"threshold\":{:.6},\"status\":\"{}\"",
        metric.name,
        metric.threshold,
        metric.status.text()
    );
    if let Some(note) = metric.note {
        let _ = write!(line, ",\"note\":\"{note}\"");
    }
    if let Some(reference) = metric.reference {
        let _ = write!(line, ",\"reference\":{reference:.6}");
    }
    line.push('}');
    line
}

#[derive(Clone, Copy)]
enum Colour {
    Albedo,
    Emissive,
}

fn colour_error(
    reader: Reader,
    scene: &GltfScene,
    mesh: &BakeMesh<'_>,
    colour: Colour,
) -> Result<f32, String> {
    let mut worst = 0.0_f32;
    for sample in samples() {
        let (albedo, emissive) = shade(reader, scene, mesh, sample.material, sample.uv)?;
        let (got, want) = match colour {
            Colour::Albedo => (albedo, sample.albedo),
            Colour::Emissive => (emissive, sample.emissive),
        };
        worst = worst.max(max_abs(got, want));
    }
    Ok(worst)
}

fn srgb_once(reader: Reader, scene: &GltfScene, mesh: &BakeMesh<'_>) -> Result<f32, String> {
    let red = channel(
        reader,
        scene,
        mesh,
        SHADE_MAT,
        [0.25, 0.25],
        Channel::Albedo,
    )?;
    let linear = channel(
        reader,
        scene,
        mesh,
        SHADE_MAT,
        [0.75, 0.25],
        Channel::Albedo,
    )?;
    let emit = channel(
        reader,
        scene,
        mesh,
        SHADE_MAT,
        [0.25, 0.25],
        Channel::Emissive,
    )?;
    Ok((red - ONCE_188)
        .abs()
        .max((linear - ONCE_10).abs())
        .max((emit - ONCE_128).abs()))
}

fn srgb_twice(reader: Reader, scene: &GltfScene, mesh: &BakeMesh<'_>) -> Result<f32, String> {
    let red = channel(
        reader,
        scene,
        mesh,
        SHADE_MAT,
        [0.25, 0.25],
        Channel::Albedo,
    )?;
    let emit = channel(
        reader,
        scene,
        mesh,
        SHADE_MAT,
        [0.25, 0.25],
        Channel::Emissive,
    )?;
    Ok(double_margin(red, TWICE_188).max(double_margin(emit, TWICE_128)))
}

fn double_margin(sample: f32, twice: f32) -> f32 {
    (DOUBLE_MARGIN - (sample - twice).abs()).max(0.0)
}

fn metal_value(reader: Reader, scene: &GltfScene, mesh: &BakeMesh<'_>) -> Result<f32, String> {
    let (albedo, _) = shade(reader, scene, mesh, METAL_MAT, [0.5, 0.5])?;
    Ok(albedo[0].max(albedo[1]).max(albedo[2]))
}

fn mr_channels(reader: Reader, scene: &GltfScene, mesh: &BakeMesh<'_>) -> Result<f32, String> {
    let (albedo, _) = shade(reader, scene, mesh, SHADE_MAT, [0.25, 0.75])?;
    Ok(max_abs(albedo, [METAL_KEEP; 3]))
}

fn mask_mistakes(mesh: &BakeMesh<'_>) -> f32 {
    let mut mistakes = 0.0_f32;
    // base alpha 0.5 times texel alpha, cutoff 0.5.
    // 128/255 * 0.5 = 64/255 < 0.5, so that triangle drops.
    // 1 * 0.5 = 0.5, which is kept (alpha >= cutoff). 0 drops.
    if count_near(mesh, 7.0 / 3.0) != 0 {
        mistakes += 1.0;
    }
    if count_near(mesh, 13.0 / 3.0) != 1 {
        mistakes += 1.0;
    }
    if count_near(mesh, 19.0 / 3.0) != 0 {
        mistakes += 1.0;
    }
    mistakes
}

fn count_near(mesh: &BakeMesh<'_>, x: f32) -> usize {
    mesh.triangles
        .iter()
        .filter(|tri| {
            let mid = (tri.positions[0][0] + tri.positions[1][0] + tri.positions[2][0]) / 3.0;
            (mid - x).abs() < 0.2
        })
        .count()
}

fn texcoord_mistakes(scene: &GltfScene) -> Result<f32, String> {
    let shade = scene
        .materials
        .first()
        .ok_or("fixture has no shade material")?;
    let base = shade
        .base_color_texture
        .ok_or("shade has no base texture")?;
    let mr = shade
        .metallic_roughness_texture
        .ok_or("shade has no metallic-roughness texture")?;
    let emissive = shade
        .emissive_texture
        .ok_or("shade has no emissive texture")?;
    let uv = scene
        .meshes
        .first()
        .and_then(|mesh| mesh.primitives.first())
        .and_then(|primitive| primitive.texcoords.first())
        .copied()
        .ok_or("shade primitive has no TEXCOORD_0")?;
    let mut mistakes = 0.0_f32;
    if base.texcoord != 0 {
        mistakes += 1.0;
    }
    if mr.texcoord != 0 {
        mistakes += 1.0;
    }
    if emissive.texcoord != 1 {
        mistakes += 1.0;
    }
    // TEXCOORD_1 at this corner is (0.75, 0.75). A loader that kept set 1
    // would not land on the set 0 corner (0.25, 0.25).
    if (uv[0] - 0.25).abs() > 1.0e-5 || (uv[1] - 0.25).abs() > 1.0e-5 {
        mistakes += 1.0;
    }
    Ok(mistakes)
}

/// Wrong-slot count at the shade triangle's first corner.
///
/// Set 0 is `(0.25, 0.25)` (base, once-decoded 188). Set 1 is `(0.75, 0.75)`
/// (emissive texel 255). The broken reader samples every slot at set 0.
fn texcoord_sample(reader: Reader, scene: &GltfScene, mesh: &BakeMesh<'_>) -> Result<f32, String> {
    let corner = mesh
        .triangles
        .first()
        .ok_or("shade mesh has no triangle")?;
    if (corner.uvs[0][0] - 0.25).abs() > 1.0e-4 || (corner.uvs[0][1] - 0.25).abs() > 1.0e-4 {
        return Err("shade corner is not TEXCOORD_0 (0.25, 0.25)".to_string());
    }
    let (albedo, emissive) = match reader {
        Reader::Bake => {
            let sample = mesh.shade_hit(0, 0.0, 0.0);
            (sample.diffuse_albedo(), sample.emissive)
        }
        Reader::Broken => broken_shade(scene, SHADE_MAT, [0.25, 0.25])?,
    };
    let mut mistakes = 0.0_f32;
    if max_abs(albedo, [ONCE_188, 0.0, 0.0]) > 1.0e-4 {
        mistakes += 1.0;
    }
    if max_abs(emissive, [1.0, 0.0, 0.0]) > 1.0e-4 {
        mistakes += 1.0;
    }
    Ok(mistakes)
}

/// Absolute error of roughness against `0.5 * 10/255`.
///
/// The fixture stores roughness in G. The broken reader uses B, which is 0
/// at this texel.
fn roughness_from_g(reader: Reader, scene: &GltfScene, mesh: &BakeMesh<'_>) -> Result<f32, String> {
    let got = match reader {
        Reader::Bake => mesh.shade(u32::try_from(SHADE_MAT).map_err(|_| "material")?, [0.25, 0.25]).roughness,
        Reader::Broken => broken_roughness(scene, SHADE_MAT, [0.25, 0.25])?,
    };
    Ok((got - (0.5 * (10.0 / 255.0))).abs())
}

fn broken_roughness(scene: &GltfScene, material: usize, uv: [f32; 2]) -> Result<f32, String> {
    let material = scene
        .materials
        .get(material)
        .ok_or("broken shade material is missing")?;
    let mr = broken_sample(scene, material.metallic_roughness_texture, uv)?;
    Ok(material.roughness * mr[2])
}

/// Absolute error of the base-colour texel alpha, before the factor.
///
/// Mask UVs land on alpha.png bytes 128, 255, and 0. The product with the
/// mask factor 0.5 would miss the first sample. The broken reader returns the
/// raw byte, so this row stays a file metric.
fn texel_alpha(reader: Reader, scene: &GltfScene, mesh: &BakeMesh<'_>) -> Result<f32, String> {
    const MASK_MAT: usize = 1;
    let samples = [([0.1, 0.1], 128.0 / 255.0), ([0.6, 0.1], 1.0), ([0.1, 0.6], 0.0)];
    let mut worst = 0.0_f32;
    for (uv, expect) in samples {
        let got = match reader {
            Reader::Bake => {
                mesh.shade(u32::try_from(MASK_MAT).map_err(|_| "material")?, uv)
                    .texel_alpha
            }
            Reader::Broken => broken_texel_alpha(scene, MASK_MAT, uv)?,
        };
        worst = worst.max((got - expect).abs());
    }
    Ok(worst)
}

fn broken_texel_alpha(scene: &GltfScene, material: usize, uv: [f32; 2]) -> Result<f32, String> {
    let material = scene
        .materials
        .get(material)
        .ok_or("broken shade material is missing")?;
    let texel = broken_sample(scene, material.base_color_texture, uv)?;
    Ok(texel[3])
}

fn emissive_default(scene: &GltfScene) -> Result<f32, String> {
    let shade = scene
        .materials
        .first()
        .ok_or("fixture has no shade material")?;
    Ok((shade.emissive_strength - 1.0).abs())
}

/// Wrong-chain count for the loader's mip chains.
///
/// The colour image is flat sRGB 188. The chain must reach 1×1 and stay 188.
/// The normal image is opposite X encodings. The 1×1 texel must be (0, 0, 1).
/// The broken reader keeps only the source level, which has no 1×1 texel.
fn mip_chain_error(reader: Reader) -> Result<f32, String> {
    let scene = load_mip_scene()?;
    let material = scene
        .materials
        .first()
        .ok_or("mip fixture has no material")?;
    let colour_index = material
        .base_color_texture
        .ok_or("mip fixture has no base colour")?
        .image;
    let normal_index = material
        .normal_texture
        .ok_or("mip fixture has no normal")?
        .image;
    let (colour, normal) = match reader {
        Reader::Bake => (
            chain_at(&scene.mips, colour_index),
            chain_at(&scene.mips, normal_index),
        ),
        Reader::Broken => (
            source_only(scene.images.get(colour_index)),
            source_only(scene.images.get(normal_index)),
        ),
    };
    Ok(mip_mistakes(&colour, &normal))
}

fn load_mip_scene() -> Result<GltfScene, String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/materials/mips.gltf");
    load_gltf_file(&path).map_err(|err| format!("load {}: {err}", path.display()))
}

fn chain_at(mips: &[Vec<Mip>], index: usize) -> Vec<Mip> {
    mips.get(index).cloned().unwrap_or_default()
}

fn source_only(image: Option<&genos_load::Image>) -> Vec<Mip> {
    let Some(image) = image else {
        return Vec::new();
    };
    vec![Mip {
        width: image.width,
        height: image.height,
        pixels: image.pixels.clone(),
    }]
}

fn mip_mistakes(colour: &[Mip], normal: &[Mip]) -> f32 {
    let mut mistakes = 0.0_f32;
    if !colour_chain_ok(colour) {
        mistakes += 1.0;
    }
    if !normal_chain_ok(normal) {
        mistakes += 1.0;
    }
    mistakes
}

fn colour_chain_ok(chain: &[Mip]) -> bool {
    let (Some(first), Some(last)) = (chain.first(), chain.last()) else {
        return false;
    };
    first.width == 2
        && first.height == 2
        && last.width == 1
        && last.height == 1
        && last.pixels.as_slice() == [188, 0, 0, 255]
}

fn normal_chain_ok(chain: &[Mip]) -> bool {
    let (Some(first), Some(last)) = (chain.first(), chain.last()) else {
        return false;
    };
    if first.width != 2 || first.height != 2 || last.width != 1 || last.height != 1 {
        return false;
    }
    let pixel = last.pixels.as_slice();
    if pixel.len() < 4 {
        return false;
    }
    let normal = normal_from_bytes([pixel[0], pixel[1], pixel[2], pixel[3]]);
    normal[0].abs() <= 1.0e-6 && normal[1].abs() <= 1.0e-6 && (normal[2] - 1.0).abs() <= 1.0e-6
}

fn mib(n: u64) -> u64 {
    n * 1024 * 1024
}

/// Resident MiB under the High cap. The loader's planner drops the finest mip.
/// The broken reader keeps every level.
fn residency_mb(reader: Reader) -> f32 {
    let images = [ImageChain {
        levels: vec![mib(300), mib(75), mib(20)],
        rank: 1,
    }];
    let bytes = match reader {
        Reader::Bake => plan_residency(&images, mib(256)).bytes,
        Reader::Broken => images.iter().flat_map(|image| &image.levels).sum(),
    };
    mib_f32(bytes)
}

/// Frames from a finished upload to the sample.
///
/// The broken reader holds the upload and never flips, so the new page is
/// still absent when the frame limit lands.
fn reload_frames(reader: Reader) -> f32 {
    let mut reload = HotReload::open(1);
    match reader {
        Reader::Bake => reload.upload(2),
        Reader::Broken => reload.hold(2),
    }
    reload.frames_until_visible(5)
}

#[derive(Clone, Copy)]
enum Channel {
    Albedo,
    Emissive,
}

fn channel(
    reader: Reader,
    scene: &GltfScene,
    mesh: &BakeMesh<'_>,
    material: usize,
    uv: [f32; 2],
    which: Channel,
) -> Result<f32, String> {
    let (albedo, emissive) = shade(reader, scene, mesh, material, uv)?;
    Ok(match which {
        Channel::Albedo => albedo[0],
        Channel::Emissive => emissive[0],
    })
}

fn shade(
    reader: Reader,
    scene: &GltfScene,
    mesh: &BakeMesh<'_>,
    material: usize,
    uv: [f32; 2],
) -> Result<([f32; 3], [f32; 3]), String> {
    match reader {
        Reader::Bake => {
            let index = u32::try_from(material).map_err(|_| "material index does not fit u32")?;
            Ok(mesh.surface(index, uv))
        }
        Reader::Broken => broken_shade(scene, material, uv),
    }
}

fn broken_shade(
    scene: &GltfScene,
    material: usize,
    uv: [f32; 2],
) -> Result<([f32; 3], [f32; 3]), String> {
    let material = scene
        .materials
        .get(material)
        .ok_or("broken shade material is missing")?;
    let base = broken_sample(scene, material.base_color_texture, uv)?;
    let albedo = [
        material.base_color[0] * base[0],
        material.base_color[1] * base[1],
        material.base_color[2] * base[2],
    ];
    let emit = broken_sample(scene, material.emissive_texture, uv)?;
    let emissive = [
        material.emissive[0] * emit[0],
        material.emissive[1] * emit[1],
        material.emissive[2] * emit[2],
    ];
    Ok((albedo, emissive))
}

fn broken_sample(
    scene: &GltfScene,
    slot: Option<genos_load::TextureSlot>,
    uv: [f32; 2],
) -> Result<[f32; 4], String> {
    let Some(slot) = slot else {
        return Ok([1.0; 4]);
    };
    let image = scene
        .images
        .get(slot.image)
        .ok_or("broken shade image is missing")?;
    if image.width != 2 || image.height != 2 {
        return Err("broken shade expected a 2x2 image".to_string());
    }
    let pixel = image
        .pixel(wrap2(uv[0]), wrap2(uv[1]))
        .ok_or("broken shade texel is outside the image")?;
    Ok([
        double_eotf(pixel[0]),
        double_eotf(pixel[1]),
        double_eotf(pixel[2]),
        f32::from(pixel[3]) / 255.0,
    ])
}

/// The injected bug: the IEC curve applied twice. Not an expected value.
fn double_eotf(byte: u8) -> f32 {
    eotf(eotf(f32::from(byte) / 255.0))
}

fn eotf(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn wrap2(t: f32) -> u32 {
    let frac = t - t.floor();
    u32::from(frac * 2.0 >= 1.0)
}

fn max_abs(a: [f32; 3], b: [f32; 3]) -> f32 {
    let mut worst = 0.0_f32;
    for index in 0..3 {
        worst = worst.max((a[index] - b[index]).abs());
    }
    worst
}

fn samples() -> [Sample; 9] {
    let red = [ONCE_188, 0.0, 0.0];
    let linear = [ONCE_10, 0.0, 0.0];
    let partial = [METAL_KEEP; 3];
    let black = [0.0; 3];
    let emit_128 = [ONCE_128, 0.0, 0.0];
    let emit_200 = [ONCE_200, 0.0, 0.0];
    let emit_full = [1.0, 0.0, 0.0];
    let emit_x2 = [EMIT_STRENGTH_2, 0.0, 0.0];
    [
        sample(SHADE_MAT, [0.25, 0.25], red, emit_128),
        sample(SHADE_MAT, [0.75, 0.25], linear, emit_200),
        sample(SHADE_MAT, [0.25, 0.75], partial, black),
        sample(SHADE_MAT, [0.75, 0.75], black, emit_full),
        sample(SHADE_MAT, [1.25, 0.25], red, emit_128),
        sample(SHADE_MAT, [1.75, 1.25], linear, emit_200),
        sample(SHADE_MAT, [1.25, -0.25], partial, black),
        sample(BRIGHT_MAT, [0.25, 0.25], red, emit_x2),
        sample(METAL_MAT, [0.5, 0.5], black, black),
    ]
}

fn sample(material: usize, uv: [f32; 2], albedo: [f32; 3], emissive: [f32; 3]) -> Sample {
    Sample {
        material,
        uv,
        albedo,
        emissive,
    }
}
