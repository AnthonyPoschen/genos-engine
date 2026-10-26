//! Headless gate for the PBR material design.
//!
//! `cargo test -p genos-bake --test materials_gate -- --case good|bad [--out DIR]`
//!
//! Prints one line per metric: `name value threshold STATUS`.
//! Writes `DIR/<case>.json`. The default directory is `genos-debug/materials`.
//! `GENOS_MATERIALS_GATE_CASE` and `GENOS_MATERIALS_GATE_OUT` apply only when
//! the matching argument was omitted. Exit 0 when nothing FAILs. Exit 1 when
//! a metric FAILs. `NOT-IMPLEMENTED` is not a failure. Exit 2 on a bad
//! argument, a failed self-check, or a write error.
//!
//! `--case good` reads `BakeMesh::surface`. `--case bad` reads a shade in this
//! file that skips sRGB, skips `(1 − metallic)`, and skips emissive strength.
//! The good case also runs that broken shade and refuses to pass if the broken
//! shade would pass an implemented metric.

use std::path::PathBuf;
use std::process::ExitCode;

use genos_bake::{BakeMaterial, BakeMesh};
use genos_load::{Image, PbrMaterial, Sampler, TextureSlot};

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

const MUST_FAIL: [&str; 4] = [
    "shade_albedo",
    "shade_emissive",
    "srgb_decode",
    "metal_diffuse",
];

#[derive(Clone, Copy)]
struct Fix {
    base: [f32; 4],
    metallic: f32,
    emissive: [f32; 3],
    strength: f32,
    base_px: Option<[u8; 4]>,
    mr_px: Option<[u8; 4]>,
    emissive_px: Option<[u8; 4]>,
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
             Scores BakeMesh::surface (--case good) or a broken shade (--case bad).\n\
             Writes DIR/<case>.json. Default DIR is genos-debug/materials.\n\
             GENOS_MATERIALS_GATE_CASE and GENOS_MATERIALS_GATE_OUT apply only when\n\
             the matching argument was omitted.\n\
             Exit 0 when no metric FAILs. Exit 1 on a FAIL. NOT-IMPLEMENTED is not a failure."
        );
        return Ok(ExitCode::SUCCESS);
    };
    if args.case == Case::Good {
        if let Some(err) = broken_is_wrong(&evaluate(Reader::Broken)) {
            return Err(err);
        }
    }
    let metrics = evaluate(args.case.reader());
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

fn evaluate(reader: Reader) -> Vec<Metric> {
    vec![
        measured(
            "shade_albedo",
            max_albedo_error(reader, &albedo_fixtures()),
            SHADE,
        ),
        measured(
            "shade_emissive",
            max_emissive_error(reader, &emissive_fixtures()),
            SHADE,
        ),
        measured("srgb_decode", srgb_error(reader), SRGB),
        measured("metal_diffuse", metal_value(reader), SHADE),
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
        pending_ref(
            "emissive_furnace",
            RELATIVE,
            "no GI run in this gate",
            furnace_outgoing(),
        ),
        pending("double_count", RELATIVE, "no promoted emitter"),
        pending("bc7_colour_rmse", BC7, "no BC7 encoder"),
        pending("astc_colour_rmse", ASTC, "no ASTC encoder"),
        pending("residency_mb", RESIDENCY_MB, "no page table"),
        pending("hot_reload_frames", RELOAD_FRAMES, "no watcher"),
        pending("gbuffer_ms", GBUFFER_MS, "no GPU timing in this gate"),
    ]
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
        if MUST_FAIL.contains(&metric.name) {
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

fn pending_ref(name: &'static str, threshold: f32, note: &'static str, reference: f64) -> Metric {
    Metric {
        name,
        value: None,
        threshold,
        status: Status::NotImplemented,
        note: Some(note),
        reference: Some(reference),
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
        None => format!(
            "{} na {:.6} {}",
            metric.name,
            metric.threshold,
            metric.status.text()
        ),
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
        line.push_str(&format!(",\"note\":\"{note}\""));
    }
    if let Some(reference) = metric.reference {
        line.push_str(&format!(",\"reference\":{reference:.6}"));
    }
    line.push('}');
    line
}

fn max_albedo_error(reader: Reader, fixes: &[Fix]) -> f32 {
    let mut worst = 0.0_f32;
    for fix in fixes {
        let (got, _) = shade(reader, fix);
        let (want, _) = analytic(fix);
        worst = worst.max(max_abs(got, want));
    }
    worst
}

fn max_emissive_error(reader: Reader, fixes: &[Fix]) -> f32 {
    let mut worst = 0.0_f32;
    for fix in fixes {
        let (_, got) = shade(reader, fix);
        let (_, want) = analytic(fix);
        worst = worst.max(max_abs(got, want));
    }
    worst
}

fn srgb_error(reader: Reader) -> f32 {
    let fix = srgb_fixture();
    let (got, _) = shade(reader, &fix);
    let (want, _) = analytic(&fix);
    max_abs(got, want)
}

fn metal_value(reader: Reader) -> f32 {
    let (albedo, _) = shade(reader, &metal_fixture());
    albedo[0].max(albedo[1]).max(albedo[2])
}

fn furnace_outgoing() -> f64 {
    let albedo = 0.8_f64;
    let emit = 1.0_f64;
    emit / (1.0 - albedo)
}

fn shade(reader: Reader, fix: &Fix) -> ([f32; 3], [f32; 3]) {
    match reader {
        Reader::Bake => bake_shade(fix),
        Reader::Broken => broken_shade(fix),
    }
}

fn analytic(fix: &Fix) -> ([f32; 3], [f32; 3]) {
    let base_tex = match fix.base_px {
        Some(px) => [
            srgb_channel(px[0]),
            srgb_channel(px[1]),
            srgb_channel(px[2]),
        ],
        None => [1.0; 3],
    };
    let metal_tex = match fix.mr_px {
        Some(px) => byte_unit(px[2]),
        None => 1.0,
    };
    let metallic = (fix.metallic * metal_tex).clamp(0.0, 1.0);
    let albedo =
        std::array::from_fn(|channel| fix.base[channel] * base_tex[channel] * (1.0 - metallic));
    let emis_tex = match fix.emissive_px {
        Some(px) => [
            srgb_channel(px[0]),
            srgb_channel(px[1]),
            srgb_channel(px[2]),
        ],
        None => [1.0; 3],
    };
    let emissive =
        std::array::from_fn(|channel| fix.emissive[channel] * emis_tex[channel] * fix.strength);
    (albedo, emissive)
}

fn broken_shade(fix: &Fix) -> ([f32; 3], [f32; 3]) {
    let base_tex = match fix.base_px {
        Some(px) => [byte_unit(px[0]), byte_unit(px[1]), byte_unit(px[2])],
        None => [1.0; 3],
    };
    let albedo = std::array::from_fn(|channel| fix.base[channel] * base_tex[channel]);
    let emis_tex = match fix.emissive_px {
        Some(px) => [byte_unit(px[0]), byte_unit(px[1]), byte_unit(px[2])],
        None => [1.0; 3],
    };
    let emissive = std::array::from_fn(|channel| fix.emissive[channel] * emis_tex[channel]);
    (albedo, emissive)
}

fn bake_shade(fix: &Fix) -> ([f32; 3], [f32; 3]) {
    let mut images = Vec::new();
    let mut material = PbrMaterial {
        metallic: fix.metallic,
        roughness: 1.0,
        base_color: fix.base,
        emissive: fix.emissive,
        emissive_strength: fix.strength,
        ..PbrMaterial::default()
    };
    if let Some(px) = fix.base_px {
        material.base_color_texture = Some(slot(images.len()));
        images.push(texel(px));
    }
    if let Some(px) = fix.mr_px {
        material.metallic_roughness_texture = Some(slot(images.len()));
        images.push(texel(px));
    }
    if let Some(px) = fix.emissive_px {
        material.emissive_texture = Some(slot(images.len()));
        images.push(texel(px));
    }
    let mesh = BakeMesh {
        name: "gate".to_string(),
        triangles: Vec::new(),
        materials: vec![BakeMaterial { pbr: material }],
        images: &images,
    };
    mesh.surface(0, [0.5, 0.5])
}

fn slot(image: usize) -> TextureSlot {
    TextureSlot {
        image,
        texcoord: 0,
        sampler: Sampler::default(),
        scale: 1.0,
    }
}

fn texel(px: [u8; 4]) -> Image {
    Image {
        width: 1,
        height: 1,
        pixels: vec![px[0], px[1], px[2], px[3]],
    }
}

fn srgb_channel(byte: u8) -> f32 {
    let value = f32::from(byte) / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn byte_unit(byte: u8) -> f32 {
    f32::from(byte) / 255.0
}

fn max_abs(a: [f32; 3], b: [f32; 3]) -> f32 {
    let mut worst = 0.0_f32;
    for index in 0..3 {
        worst = worst.max((a[index] - b[index]).abs());
    }
    worst
}

fn albedo_fixtures() -> Vec<Fix> {
    vec![
        plain([0.2, 0.4, 0.6, 1.0], 0.0, [0.0; 3], 1.0),
        plain([0.8, 0.8, 0.8, 1.0], 0.25, [0.0; 3], 1.0),
        Fix {
            base: [1.0, 1.0, 1.0, 1.0],
            metallic: 0.0,
            emissive: [0.0; 3],
            strength: 1.0,
            base_px: Some([188, 40, 220, 255]),
            mr_px: None,
            emissive_px: None,
        },
        Fix {
            base: [1.0, 0.0, 0.0, 1.0],
            metallic: 1.0,
            emissive: [0.0; 3],
            strength: 1.0,
            base_px: None,
            mr_px: Some([0, 0, 128, 255]),
            emissive_px: None,
        },
    ]
}

fn emissive_fixtures() -> Vec<Fix> {
    vec![
        plain([1.0; 4], 0.0, [0.25, 0.0, 0.5], 4.0),
        Fix {
            base: [1.0; 4],
            metallic: 0.0,
            emissive: [1.0, 0.0, 0.0],
            strength: 2.0,
            base_px: None,
            mr_px: None,
            emissive_px: Some([128, 0, 0, 255]),
        },
    ]
}

fn srgb_fixture() -> Fix {
    Fix {
        base: [1.0, 1.0, 1.0, 1.0],
        metallic: 0.0,
        emissive: [0.0; 3],
        strength: 1.0,
        base_px: Some([188, 40, 220, 255]),
        mr_px: None,
        emissive_px: None,
    }
}

fn metal_fixture() -> Fix {
    plain([1.0, 1.0, 1.0, 1.0], 1.0, [0.0; 3], 1.0)
}

fn plain(base: [f32; 4], metallic: f32, emissive: [f32; 3], strength: f32) -> Fix {
    Fix {
        base,
        metallic,
        emissive,
        strength,
        base_px: None,
        mr_px: None,
        emissive_px: None,
    }
}
