#![cfg_attr(
    test,
    expect(clippy::expect_used, reason = "a test aborts when a fixture fails")
)]
#![cfg_attr(
    test,
    expect(
        clippy::disallowed_methods,
        reason = "the renderer reads GENOS_LIGHTING_DUMP; this test is the argument that asks for the dump, and CACHE_ENERGY_* are harness knobs"
    )
)]
#![allow(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "pixel statistics in a test"
)]

//! Surface-cache energy against the path-traced reference (ADR 0037, stage 1
//! of the lighting simplification).
//!
//! A still scene in the default surface cache must settle at the light the
//! CPU path trace finds: the bounce view (indirect light only) over the
//! reference's indirect (64 bounces minus direct only) is between 0.9 and
//! 1.1 on the pixels the reference hits, in a closed box, an open floor and
//! the stress hall at night. The direct view against the direct-only trace
//! checks the harness (the oracle: the same pixels, units and exposure).
//!
//! The cache itself is graded too: the 25 cm cells (and 12.5 cm texels) in
//! a lighting dump against the CPU bounce irradiance at each cell's point and
//! normal (`genos_debug::reference::bounce_irradiance`). Their sum and their
//! median lit cell must hold at least 0.9 of it, and a still scene must settle.
//! A fourth case starts the hall dark and grades it 300 lit frames later (lag:
//! at least 0.9). A fifth draws the lit hall, then grades 120 frames after the
//! lamps go out (linger: at most 0.05 of the lit bounce). The picture ratio is
//! summed cache over summed reference on the pixels the reference hits.
//!
//! Stage 2 acceptance sets `surface_cache.sh_cells` in the test and checks the
//! flag the renderer resolved after `apply_env`. It is not `CACHE_ENERGY_SH`
//! being present. A preset, a frame count, a warm count, or a research trace
//! is refused here: those are measurement changes, and a research trace is
//! labeled non-acceptance and written outside the acceptance cache. The CPU
//! weight model in `lighting_sh` is not this gate. A path trace that misses
//! 0.02 noise, or is nonfinite or missing, is blocked. A trace that reaches
//! the 600 s or 4096 spp cap still passes when the noise is already at or
//! under 0.02. A reconstructed linger from a rejected run is not a pass. The
//! offline precompute in `docs/systems/lighting/cache-energy-precompute.md`
//! can trace the same setup for longer, and the open floor can also raise the
//! sample cap to 262144, then copy a converged file into the acceptance cache.
//! Those are execution limits. The 600 s and 4096 spp acceptance call stays.
//! The oracle's `energy_mean` is the full picture,
//! direct light included, and is a separate report.
//!
//! Per scene the test also reports blotch (mean relative error of 8x8 block
//! means of the indirect) and the still cost: cache batch entries per frame
//! (16 rays each) over the last 120 frames, the counter proxy for the idle
//! relight cost on lavapipe, where milliseconds mean nothing.
//!
//! Lavapipe and minutes of CPU tracing, so ignored by default:
//! `genos-box-test cargo test --release -p genos-stress --test cache_energy -- --ignored --nocapture --test-threads=1`.
//! The reference is cached in `target/reference-cache`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use genos_render::{DebugView, Renderer, ViewMode, World};
use genos_scene::{Camera, LightingPreset, LightingSettings, Scene, Vec3};
use genos_stress::building::{Layout, LightMix};
use genos_stress::stage::{Scale, Stage};
use genos_window::Window;

/// Acceptance path trace: noise, spp cap, wall-clock cap. Not lowered for a run.
const NOISE_TARGET: f32 = 0.02;

const MAX_SPP: u32 = 4096;

const TRACE_SECONDS: f32 = 600.0;

const PRECOMPUTE_SECONDS_CAP: f32 = 2400.0;

/// Open-floor offline sample ceiling. An execution limit, not a noise target.
/// 2^18 stays below 2^20, where `(done + k)` meets `x` in the existing seed
/// `(y << 40) ^ (x << 20) ^ (done + k)`.
const OFFLINE_MAX_SPP: u32 = 262_144;

const RESEARCH_SECONDS_CAP: f32 = 1200.0;

const DIRECT_LOW: f64 = 0.95;

const DIRECT_HIGH: f64 = 1.05;

const WIDTH: u32 = 160;

const HEIGHT: u32 = 90;

/// 22:00 on the stage clock (0 is 06:00).
const NIGHT: f32 = 16.0 / 24.0;

/// Still frames before grading: three times the old 300-frame freeze.
const FRAMES: usize = 900;

const LOW: f64 = 0.9;

/// Linger gate: bounce left at 120 frames after the lamps go out, as a share
/// of the lit bounce (ADR 0039 stage 1, still required at stage 2).
const LINGER: f64 = 0.05;

/// The highest picture byte read back: 0.745 on the shoulder, linear 3.9.
const SHOULDER_TOP: u8 = 190;

const HIGH: f64 = 1.1;

fn luma(c: [f64; 3]) -> f64 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn scene_text(name: &str, text: &str) -> Scene {
    let dir = std::env::temp_dir().join(format!("cache-energy-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(format!("{name}.rhai"));
    std::fs::write(&path, text).expect("scene file");
    genos_scene::load_path(&path).expect("scene")
}

/// 6 x 6 x 3 m closed room, 10 cm walls, three grey blocks (the scene
/// script wants three solids), one warm lamp in a corner, no sun: every
/// bounce stays inside.
fn closed_box() -> (Scene, Camera) {
    let scene = scene_text(
        "box",
        "floor(0.0, 0.0, 6.0, 6.0);\n\
         roof(3.0, 0.9, 0.9, 0.9);\n\
         wall(0.0, -3.0, 6.1, 0.1, 3.0);\n\
         wall(-3.0, 0.0, 0.1, 6.1, 3.0);\n\
         wall(0.0, 3.0, 6.1, 0.1, 3.0);\n\
         wall(3.0, 0.0, 0.1, 6.1, 3.0);\n\
         solid(\"square\", 0.6, 0.4, 0.5, 0.7, 0.7, 0.7);\n\
         solid(\"square\", -0.8, 1.0, 0.4, 0.7, 0.7, 0.7);\n\
         solid(\"circle\", 1.2, -1.0, 0.3, 0.7, 0.7, 0.7);\n\
         light(-1.5, 2.2, -1.5, 0.4, 0.375, 0.325);\n",
    );
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(2.2, 1.6, 2.2), -2.356, -0.15);
    (scene, camera)
}

/// A 24 m floor, open above, with one wall (and a low far one: the script
/// wants two), three grey blocks and a lamp beside the wall: light that leaves the scene does not come back.
fn open_floor() -> (Scene, Camera) {
    let scene = scene_text(
        "floor",
        "floor(0.0, 0.0, 24.0, 24.0);\n\
         wall(0.0, 0.0, 8.0, 0.2, 3.0);\n\
         wall(-9.0, 4.0, 0.2, 3.0, 1.0);\n\
         solid(\"square\", -2.0, 2.5, 0.5, 0.7, 0.7, 0.7);\n\
         solid(\"square\", 2.5, 3.0, 0.4, 0.7, 0.7, 0.7);\n\
         solid(\"circle\", 0.5, 4.0, 0.3, 0.7, 0.7, 0.7);\n\
         light(0.0, 1.5, 1.2, 0.8, 0.75, 0.65);\n",
    );
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(0.0, 1.7, 6.0), 0.0, -0.12);
    (scene, camera)
}

/// The stress hall at 22:00 with 32 still lamps, the drain tests' pose.
fn hall() -> (Scene, Camera) {
    let (stage, camera) = hall_stage(1.0);
    (stage.world.scene.clone(), camera)
}

fn hall_stage(power: f32) -> (Stage, Camera) {
    let mix = LightMix {
        count: 32,
        dynamic_pct: 0,
        layout: Layout::Spread,
    };
    let mut stage = Stage::new(Scale::SMALL, 1, mix, 0.0, NIGHT, 120.0);
    stage.sun_frozen = true;
    stage.boxes_still = true;
    stage.lamps_still = true;
    stage.power = power;
    stage.apply();
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(14.5, 1.7, 23.5), -0.3554, 0.08);
    (stage, camera)
}

const CELL_SAMPLES: usize = 384;

/// A slot whose reference is under this share of the mean is dim: its few
/// bounce paths make its value noise, which is blotch, not missing energy.
const LIT: f64 = 0.5;

const CELL_RAYS: u32 = 1024;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn acceptance_cache() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/reference-cache")
}

fn artifact_dir(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/cache-energy").join(name)
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

fn keep(dir: &Path, body: &str) {
    std::fs::create_dir_all(dir).expect("artifact directory");
    std::fs::write(dir.join("result.json"), body).expect("result.json");
    eprintln!("cache_energy artifacts {}", dir.display());
}

#[allow(clippy::panic, reason = "the dump and result.json are already on disk; the test then aborts")]
fn abort_kept(detail: &str) -> ! {
    panic!("{detail}");
}

fn plain_key(key: &str) -> bool {
    !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn json_value(rest: &str) -> Option<(String, &str)> {
    let bytes = rest.as_bytes();
    if bytes.first() == Some(&b'"') {
        let mut index = 1;
        while index < bytes.len() {
            if bytes[index] == b'\\' {
                index += 2;
                continue;
            }
            if bytes[index] == b'"' {
                index += 1;
                return Some((rest[..index].to_string(), &rest[index..]));
            }
            index += 1;
        }
        return None;
    }
    if bytes.first() == Some(&b'[') {
        let end = rest.find(']')?;
        return Some((rest[..=end].to_string(), &rest[end + 1..]));
    }
    let end = rest.find([',', '\n', '}']).unwrap_or(rest.len());
    let value = rest[..end].trim();
    if value.is_empty() {
        None
    } else {
        Some((value.to_string(), &rest[end..]))
    }
}

fn json_fields(body: &str) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find('"') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('"') else { break };
        let key = rest[..end].to_string();
        rest = &rest[end + 1..];
        let Some(colon) = rest.find(':') else { break };
        if rest[..colon].chars().any(|c| !c.is_whitespace()) {
            continue;
        }
        rest = rest[colon + 1..].trim_start();
        let Some((value, next)) = json_value(rest) else {
            break;
        };
        if plain_key(&key) {
            fields.push((key, value));
        }
        rest = next;
    }
    fields
}

/// One integer field. Repeated fields and non-integers are refused.
fn json_u32_field(note: &str, field: &str) -> Option<u32> {
    let mut values = json_fields(note)
        .into_iter()
        .filter(|(key, _)| key == field)
        .map(|(_, value)| value);
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }
    value.parse().ok()
}

fn keep_fail(dir: &Path, detail: &str) -> ! {
    let prior = std::fs::read_to_string(dir.join("result.json")).unwrap_or_default();
    let mut rows = vec![
        "  \"verdict\": \"fail\"".to_string(),
        format!("  \"detail\": \"{}\"", escape(detail)),
    ];
    for (key, value) in json_fields(&prior) {
        if key == "verdict" || key == "detail" {
            continue;
        }
        rows.push(format!("  \"{key}\": {value}"));
    }
    keep(dir, &format!("{{\n{}\n}}\n", rows.join(",\n")));
    abort_kept(detail);
}

fn dir_list(dir: &Path) -> String {
    let mut names = Vec::new();
    if let Ok(read) = std::fs::read_dir(dir) {
        for entry in read.flatten() {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();
    names.join(",")
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum DirectOracle {
    ExactBlack,
    Ratio(f64),
}

fn parse_flag(value: Option<&str>) -> Result<bool, String> {
    let Some(value) = value else {
        return Err("missing flag".into());
    };
    match value.trim() {
        "1" | "true" | "on" | "yes" => Ok(true),
        "0" | "false" | "off" | "no" => Ok(false),
        other => Err(format!("expected on or off, got {other}")),
    }
}

fn finite(name: &str, value: f64) -> Result<f64, String> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(format!("{name} is not finite"))
    }
}

/// Share of a positive finite denominator. Zero over zero is an error, so it
/// cannot satisfy `<= LINGER`.
fn linger_share(live: f64, lit: f64) -> Result<f64, String> {
    let live = finite("observed bounce", live)?;
    let lit = finite("lit bounce denominator", lit)?;
    if lit <= 0.0 {
        return Err(format!("lit bounce denominator {lit} is not positive"));
    }
    finite("linger share", live / lit)
}

fn direct_oracle(live: f64, reference: f64) -> Result<DirectOracle, String> {
    let live = finite("observed direct", live)?;
    let reference = finite("reference direct", reference)?;
    if reference == 0.0 {
        return if live == 0.0 {
            Ok(DirectOracle::ExactBlack)
        } else {
            Err(format!("direct {live} against a black reference"))
        };
    }
    if reference < 0.0 {
        return Err(format!("reference direct {reference} is negative"));
    }
    Ok(DirectOracle::Ratio(finite("direct ratio", live / reference)?))
}

fn direct_ok(oracle: DirectOracle) -> bool {
    match oracle {
        DirectOracle::ExactBlack => true,
        DirectOracle::Ratio(ratio) => (DIRECT_LOW..=DIRECT_HIGH).contains(&ratio),
    }
}

#[derive(Clone, Copy)]
struct ReferenceFacts {
    present: bool,
    noise: f32,
    spp: u32,
    seconds: f32,
    seconds_budget: f32,
    max_spp: u32,
    noise_target: f32,
    hit_count: usize,
    linear_finite: bool,
}

fn reference_verdict(facts: &ReferenceFacts) -> Result<(), String> {
    if !facts.present {
        return Err("blocked: reference is missing".into());
    }
    if !facts.linear_finite || !facts.noise.is_finite() || !facts.seconds.is_finite() {
        return Err(format!(
            "blocked: reference is nonfinite (noise {}, seconds {})",
            facts.noise, facts.seconds
        ));
    }
    if facts.hit_count == 0 {
        return Err("blocked: reference has no hits".into());
    }
    if facts.noise < 0.0 || facts.noise > facts.noise_target {
        let why = if facts.noise < 0.0 {
            "below 0"
        } else if facts.seconds >= facts.seconds_budget || facts.spp >= facts.max_spp {
            "capped"
        } else {
            "above the noise target"
        };
        return Err(format!(
            "blocked: reference noise {:.6} is outside 0..={:.3} ({why}, spp {}, seconds {:.3})",
            facts.noise, facts.noise_target, facts.spp, facts.seconds
        ));
    }
    Ok(())
}

fn facts_of(reference: &genos_debug::Reference, seconds_budget: f32) -> ReferenceFacts {
    ReferenceFacts {
        present: true,
        noise: reference.noise,
        spp: reference.spp,
        seconds: reference.seconds,
        seconds_budget,
        max_spp: MAX_SPP,
        noise_target: NOISE_TARGET,
        hit_count: reference.hit.iter().filter(|hit| **hit).count(),
        linear_finite: reference.linear.iter().flatten().all(|channel| channel.is_finite()),
    }
}

const OVERRIDE_KEYS: &[&str] = &[
    "CACHE_ENERGY_WARM",
    "CACHE_ENERGY_FRAMES",
    "CACHE_ENERGY_TRAIL",
    "CACHE_ENERGY_SH",
    "CACHE_ENERGY_PGM",
    "CACHE_ENERGY_CSV",
    "GENOS_LIGHTING_PRESET",
    "GENOS_LIGHTING_DUMP",
    "GENOS_LIGHTING_DUMP_DIR",
    "GENOS_LIGHTING_SURFACE_CACHE_SH_CELLS",
];

fn acceptance_refusal(env: &[(&str, &str)]) -> Result<(), String> {
    if let Some(value) = env.iter().find(|(key, _)| *key == "CACHE_ENERGY_RESEARCH").map(|(_, value)| *value) {
        match parse_flag(Some(value)) {
            Ok(true) => {
                return Err("research non-acceptance: this run is not the stage-2 gate".into());
            }
            Ok(false) => {}
            Err(err) => return Err(format!("CACHE_ENERGY_RESEARCH: {err}")),
        }
    }
    let set: Vec<&str> = OVERRIDE_KEYS
        .iter()
        .copied()
        .filter(|key| env.iter().any(|(name, _)| name == key))
        .collect();
    if set.is_empty() {
        Ok(())
    } else {
        Err(format!("acceptance refuses measurement overrides: {}", set.join(", ")))
    }
}

fn acceptance_from_process() -> Result<(), String> {
    let mut owned = Vec::new();
    for key in OVERRIDE_KEYS.iter().copied().chain(["CACHE_ENERGY_RESEARCH"]) {
        if let Ok(value) = std::env::var(key) {
            owned.push((key.to_string(), value));
        }
    }
    let refs: Vec<(&str, &str)> = owned.iter().map(|(key, value)| (key.as_str(), value.as_str())).collect();
    acceptance_refusal(&refs)
}

fn research_plan(
    noise_target: f32,
    max_spp: u32,
    seconds: f32,
    acceptance_dir: &Path,
    research_dir: &Path,
) -> Result<(), String> {
    if !noise_target.is_finite() || noise_target > NOISE_TARGET {
        return Err("research refuses a looser noise target".into());
    }
    if max_spp < MAX_SPP {
        return Err("research refuses a lower spp cap".into());
    }
    if !seconds.is_finite() || seconds <= 0.0 || seconds > RESEARCH_SECONDS_CAP {
        return Err(format!(
            "research seconds must be finite and at most {RESEARCH_SECONDS_CAP}"
        ));
    }
    if research_dir.starts_with(acceptance_dir) || acceptance_dir.starts_with(research_dir) {
        return Err("research trace would poison the acceptance cache".into());
    }
    let name = research_dir.file_name().and_then(|text| text.to_str()).unwrap_or("");
    if name != "research-non-acceptance" {
        return Err("research output must be labeled research-non-acceptance".into());
    }
    Ok(())
}

/// A new trace in a directory that is not the acceptance cache.
///
/// The saved acceptance file has no sample sums, so this does not resume a
/// capped trace and it does not write one back. A result that misses the
/// noise target comes back `blocked`. The label is non-acceptance either way.
fn research_continue(
    scene: &Scene,
    camera: &Camera,
    bounces: u32,
    seconds: f32,
    acceptance_dir: &Path,
    research_dir: &Path,
) -> Result<genos_debug::Reference, String> {
    research_plan(NOISE_TARGET, MAX_SPP, seconds, acceptance_dir, research_dir)?;
    let mut setup = genos_debug::reference::RefSetup::from_camera(scene, camera, WIDTH, HEIGHT, 32);
    setup.max_bounces = bounces;
    setup.noise_target = NOISE_TARGET;
    setup.max_spp = MAX_SPP;
    setup.seconds = seconds;
    std::fs::create_dir_all(research_dir).map_err(|err| err.to_string())?;
    let reference = genos_debug::reference::render(&setup);
    let note = format!(
        "research non-acceptance\nnoise {}\nspp {}\nseconds {}\n",
        reference.noise, reference.spp, reference.seconds
    );
    std::fs::write(research_dir.join("trace.txt"), note).map_err(|err| err.to_string())?;
    reference_verdict(&facts_of(&reference, seconds))?;
    Ok(reference)
}

fn acceptance_setup(
    scene: &Scene,
    camera: &Camera,
    bounces: u32,
) -> genos_debug::reference::RefSetup {
    let mut setup = genos_debug::reference::RefSetup::from_camera(scene, camera, WIDTH, HEIGHT, 32);
    setup.max_bounces = bounces;
    setup.noise_target = NOISE_TARGET;
    setup.max_spp = MAX_SPP;
    setup.seconds = TRACE_SECONDS;
    setup
}

/// Same trace as [`acceptance_setup`] with a longer bound. Seconds are the only
/// field that changes, and they stay in `(0, 2400]`.
fn precompute_setup(
    scene: &Scene,
    camera: &Camera,
    bounces: u32,
    seconds: f32,
) -> Result<genos_debug::reference::RefSetup, String> {
    if !seconds.is_finite() || seconds <= 0.0 || seconds > PRECOMPUTE_SECONDS_CAP {
        return Err(format!(
            "precompute seconds must be finite and in (0, {PRECOMPUTE_SECONDS_CAP}]"
        ));
    }
    let mut setup = acceptance_setup(scene, camera, bounces);
    setup.seconds = seconds;
    if setup.triangles || setup.specular || !setup.materials.is_empty() {
        return Err("precompute keeps the CPU analytic picture".into());
    }
    Ok(setup)
}

fn floor_budget(max_spp: u32, seconds: f32) -> Result<(), String> {
    if max_spp == 0 || max_spp > OFFLINE_MAX_SPP {
        return Err(format!(
            "floor max spp {max_spp} is outside 1..={OFFLINE_MAX_SPP}"
        ));
    }
    if !seconds.is_finite() || seconds <= 0.0 || seconds > PRECOMPUTE_SECONDS_CAP {
        return Err(format!(
            "floor seconds {seconds} must be finite and in (0, {PRECOMPUTE_SECONDS_CAP}]"
        ));
    }
    Ok(())
}

/// Acceptance trace with an explicit sample cap and time cap. Both stay inside
/// the offline ceilings. Noise, batch, bounces, pose, and the analytic picture
/// stay on the acceptance setup.
fn floor_precompute_setup(
    scene: &Scene,
    camera: &Camera,
    bounces: u32,
    max_spp: u32,
    seconds: f32,
) -> Result<genos_debug::reference::RefSetup, String> {
    floor_budget(max_spp, seconds)?;
    let mut setup = acceptance_setup(scene, camera, bounces);
    setup.max_spp = max_spp;
    setup.seconds = seconds;
    if setup.triangles || setup.specular || !setup.materials.is_empty() {
        return Err("floor precompute keeps the CPU analytic picture".into());
    }
    Ok(setup)
}

/// Scene, pose, size, batch, noise target, bounces, and analytic shapes.
/// Sample cap and seconds are budgets, checked apart from this.
fn shares_acceptance_identity(
    acceptance: &genos_debug::reference::RefSetup,
    produced: &genos_debug::reference::RefSetup,
) -> Result<(), String> {
    if acceptance.scene != produced.scene {
        return Err("scene differs".into());
    }
    if acceptance.eye.map(f32::to_bits) != produced.eye.map(f32::to_bits) {
        return Err("eye differs".into());
    }
    if acceptance.yaw.to_bits() != produced.yaw.to_bits() {
        return Err("yaw differs".into());
    }
    if acceptance.pitch.to_bits() != produced.pitch.to_bits() {
        return Err("pitch differs".into());
    }
    if acceptance.width != produced.width || acceptance.height != produced.height {
        return Err("size differs".into());
    }
    if acceptance.spp != produced.spp
        || acceptance.noise_target.to_bits() != produced.noise_target.to_bits()
        || acceptance.max_bounces != produced.max_bounces
    {
        return Err("estimator differs".into());
    }
    if acceptance.triangles != produced.triangles
        || acceptance.specular != produced.specular
        || acceptance.materials != produced.materials
    {
        return Err("shape or lobe differs".into());
    }
    Ok(())
}

fn shares_acceptance_trace(
    acceptance: &genos_debug::reference::RefSetup,
    produced: &genos_debug::reference::RefSetup,
) -> Result<(), String> {
    shares_acceptance_identity(acceptance, produced)?;
    if acceptance.max_spp != produced.max_spp {
        return Err("sampling budget differs".into());
    }
    Ok(())
}

struct Promotion<'a> {
    reference: &'a genos_debug::Reference,
    produced: &'a genos_debug::reference::RefSetup,
    acceptance: &'a genos_debug::reference::RefSetup,
    commit: &'a str,
    expected_commit: &'a str,
    isolated: &'a Path,
    acceptance_dir: &'a Path,
}

fn reference_picture(
    reference: &genos_debug::Reference,
    width: u32,
    height: u32,
) -> Result<(), String> {
    if reference.image.width != width || reference.image.height != height {
        return Err(format!(
            "blocked: reference is {}x{}, acceptance is {width}x{height}",
            reference.image.width, reference.image.height
        ));
    }
    if reference.specular.is_some() {
        return Err("blocked: a specular layer is not the cached picture".into());
    }
    if !reference.noise.is_finite()
        || reference
            .linear
            .iter()
            .flatten()
            .any(|channel| !channel.is_finite())
    {
        return Err(format!(
            "blocked: reference is nonfinite (noise {})",
            reference.noise
        ));
    }
    if reference.linear.is_empty() || reference.hit.iter().all(|hit| !hit) {
        return Err("blocked: reference has no hits".into());
    }
    Ok(())
}

fn reference_acceptable(
    reference: &genos_debug::Reference,
    width: u32,
    height: u32,
) -> Result<(), String> {
    reference_picture(reference, width, height)?;
    if reference.spp == 0 || reference.spp > MAX_SPP {
        return Err(format!(
            "blocked: spp {} is outside 1..={MAX_SPP}",
            reference.spp
        ));
    }
    reject_noise(reference.noise, reference.spp, MAX_SPP)
}

fn reference_within_offline_cap(
    reference: &genos_debug::Reference,
    width: u32,
    height: u32,
    spp_cap: u32,
) -> Result<(), String> {
    if spp_cap == 0 || spp_cap > OFFLINE_MAX_SPP {
        return Err(format!(
            "blocked: offline spp cap {spp_cap} is outside 1..={OFFLINE_MAX_SPP}"
        ));
    }
    reference_picture(reference, width, height)?;
    if reference.spp == 0 || reference.spp > spp_cap {
        return Err(format!(
            "blocked: spp {} is outside 1..={spp_cap}",
            reference.spp
        ));
    }
    reject_noise(reference.noise, reference.spp, spp_cap)
}

fn reject_noise(noise: f32, spp: u32, spp_cap: u32) -> Result<(), String> {
    if (0.0..=NOISE_TARGET).contains(&noise) {
        return Ok(());
    }
    let why = if noise < 0.0 {
        "below 0"
    } else if spp >= spp_cap {
        "capped"
    } else {
        "above the noise target"
    };
    Err(format!(
        "blocked: reference noise {noise:.6} is outside 0..={NOISE_TARGET:.3} ({why}, spp {spp})"
    ))
}

fn pictures_match(left: &genos_debug::Reference, right: &genos_debug::Reference) -> bool {
    left.noise.to_bits() == right.noise.to_bits()
        && left.spp == right.spp
        && left.hit == right.hit
        && left.linear.len() == right.linear.len()
        && left
            .linear
            .iter()
            .zip(&right.linear)
            .all(|(a, b)| a.map(f32::to_bits) == b.map(f32::to_bits))
}

fn promotion_note(
    commit: &str,
    key: u64,
    produced: &genos_debug::reference::RefSetup,
    reference: &genos_debug::Reference,
    promoted: bool,
) -> String {
    format!(
        "{{\n  \"source_commit\": \"{commit}\",\n  \"cache_key\": \"{key:016x}\",\n  \"seconds\": {},\n  \"acceptance_seconds\": {TRACE_SECONDS},\n  \"noise\": {},\n  \"spp\": {},\n  \"noise_target\": {},\n  \"max_spp\": {},\n  \"spp_batch\": {},\n  \"bounces\": {},\n  \"width\": {},\n  \"height\": {},\n  \"yaw_bits\": {},\n  \"pitch_bits\": {},\n  \"eye_bits\": [{}, {}, {}],\n  \"triangles\": {},\n  \"specular\": {},\n  \"materials\": {},\n  \"estimator\": \"cpu-analytic\",\n  \"scene\": \"hall\",\n  \"power\": 1.0,\n  \"promoted\": {promoted}\n}}\n",
        produced.seconds,
        reference.noise,
        reference.spp,
        produced.noise_target,
        produced.max_spp,
        produced.spp,
        produced.max_bounces,
        produced.width,
        produced.height,
        produced.yaw.to_bits(),
        produced.pitch.to_bits(),
        produced.eye[0].to_bits(),
        produced.eye[1].to_bits(),
        produced.eye[2].to_bits(),
        produced.triangles,
        produced.specular,
        produced.materials.len()
    )
}

/// Copy a converged isolated trace into the acceptance cache. A miss deletes nothing
/// and writes nothing there.
fn promote_isolated(promotion: &Promotion<'_>) -> Result<PathBuf, String> {
    let commit = promotion.commit;
    if commit.len() != 40
        || !commit.chars().all(|c| c.is_ascii_hexdigit())
        || commit != promotion.expected_commit
    {
        return Err(format!("source commit {commit} is not this precompute"));
    }
    if promotion.acceptance.seconds.to_bits() != TRACE_SECONDS.to_bits() {
        return Err("acceptance seconds are not 600".into());
    }
    let seconds = promotion.produced.seconds;
    if !seconds.is_finite() || seconds <= 0.0 || seconds > PRECOMPUTE_SECONDS_CAP {
        return Err(format!(
            "precompute seconds {seconds} are outside (0, {PRECOMPUTE_SECONDS_CAP}]"
        ));
    }
    shares_acceptance_trace(promotion.acceptance, promotion.produced)?;
    if promotion.produced.triangles
        || promotion.produced.specular
        || !promotion.produced.materials.is_empty()
    {
        return Err("precompute keeps CPU analytic shapes and the full picture".into());
    }
    if genos_debug::reference::gpu_wanted() {
        return Err("precompute refuses GENOS_REFERENCE_GPU".into());
    }
    let key = genos_debug::reference::cache_key(promotion.acceptance);
    if key != genos_debug::reference::cache_key(promotion.produced) {
        return Err("seconds leaked into the cache key".into());
    }
    reference_acceptable(
        promotion.reference,
        promotion.acceptance.width,
        promotion.acceptance.height,
    )?;
    let loaded = genos_debug::reference::read_cached(
        promotion.isolated,
        promotion.acceptance.width,
        promotion.acceptance.height,
    )
    .ok_or_else(|| {
        format!(
            "isolated reference {} is missing, empty, or the wrong picture",
            promotion.isolated.display()
        )
    })?;
    reference_acceptable(
        &loaded,
        promotion.acceptance.width,
        promotion.acceptance.height,
    )?;
    if !pictures_match(&loaded, promotion.reference) {
        return Err("isolated file does not match the trace".into());
    }
    std::fs::create_dir_all(promotion.acceptance_dir).map_err(|err| err.to_string())?;
    let dest = promotion.acceptance_dir.join(format!("{key:016x}.ref"));
    let note_path = promotion
        .acceptance_dir
        .join(format!("{key:016x}.provenance.json"));
    let note = promotion_note(commit, key, promotion.produced, promotion.reference, true);
    publish_cached_pair(
        promotion.isolated,
        &dest,
        &note_path,
        &note,
        promotion.acceptance.width,
        promotion.acceptance.height,
    )?;
    Ok(dest)
}

/// Stage the reference and its sidecar, then rename the `.ref` and then the
/// sidecar. A failure before the `.ref` rename leaves the existing pair byte
/// for byte. The two renames are not one transaction. If the `.ref` rename
/// has succeeded and the sidecar rename fails, this returns an error and the
/// new `.ref` may remain beside the previous sidecar. That picture was already
/// checked (finite samples, a hit, noise in `0..=0.02`). `render_cached` reads
/// the `.ref` only and does not read the sidecar.
fn publish_cached_pair(
    isolated: &Path,
    dest: &Path,
    note_path: &Path,
    note: &str,
    width: u32,
    height: u32,
) -> Result<(), String> {
    let parent = dest
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let token = publish_token();
    let ref_tmp = parent.join(format!(".{token}.ref.partial"));
    let note_tmp = parent.join(format!(".{token}.provenance.partial"));
    let cleanup = || {
        let _ = std::fs::remove_file(&ref_tmp);
        let _ = std::fs::remove_file(&note_tmp);
    };
    if let Err(err) = std::fs::copy(isolated, &ref_tmp) {
        cleanup();
        return Err(format!("staged reference copy failed: {err}"));
    }
    let staged = match std::fs::read(&ref_tmp) {
        Ok(bytes) => bytes,
        Err(err) => {
            cleanup();
            return Err(format!("staged reference read failed: {err}"));
        }
    };
    let source = match std::fs::read(isolated) {
        Ok(bytes) => bytes,
        Err(err) => {
            cleanup();
            return Err(format!("isolated reference read failed: {err}"));
        }
    };
    if staged != source || genos_debug::reference::read_cached(&ref_tmp, width, height).is_none() {
        cleanup();
        return Err("staged reference does not match the isolated file".into());
    }
    if let Err(err) = std::fs::write(&note_tmp, note) {
        cleanup();
        return Err(format!("staged provenance write failed: {err}"));
    }
    if let Err(err) = std::fs::rename(&ref_tmp, dest) {
        cleanup();
        return Err(format!("reference replace failed: {err}"));
    }
    if let Err(err) = std::fs::rename(&note_tmp, note_path) {
        let _ = std::fs::remove_file(&note_tmp);
        return Err(format!(
            "provenance replace failed after the reference replace: {err}"
        ));
    }
    Ok(())
}

fn publish_token() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static TOKEN: AtomicU64 = AtomicU64::new(1);
    TOKEN.fetch_add(1, Ordering::Relaxed)
}

fn finite_clock(seconds: f32) -> Option<f32> {
    seconds.is_finite().then_some(seconds)
}

/// Sidecar for one floor trace. This is a record of the budget and the
/// result, not a cryptographic proof of the samples.
fn floor_promotion_note(
    commit: &str,
    key: u64,
    produced: &genos_debug::reference::RefSetup,
    reference: &genos_debug::Reference,
    promoted: bool,
    elapsed: Option<f32>,
) -> String {
    let seconds = match elapsed {
        Some(seconds) => seconds.to_string(),
        None => "null".to_string(),
    };
    format!(
        "{{\n  \"source_commit\": \"{commit}\",\n  \"cache_key\": \"{key:016x}\",\n  \"scene\": \"floor\",\n  \"seconds\": {seconds},\n  \"seconds_budget\": {},\n  \"acceptance_seconds\": {TRACE_SECONDS},\n  \"noise\": {},\n  \"spp\": {},\n  \"acceptance_max_spp\": {MAX_SPP},\n  \"offline_max_spp\": {},\n  \"offline_spp_ceiling\": {OFFLINE_MAX_SPP},\n  \"noise_target\": {},\n  \"spp_batch\": {},\n  \"bounces\": {},\n  \"width\": {},\n  \"height\": {},\n  \"yaw_bits\": {},\n  \"pitch_bits\": {},\n  \"eye_bits\": [{}, {}, {}],\n  \"triangles\": {},\n  \"specular\": {},\n  \"materials\": {},\n  \"estimator\": \"cpu-analytic\",\n  \"promoted\": {promoted}\n}}\n",
        produced.seconds,
        reference.noise,
        reference.spp,
        produced.max_spp,
        produced.noise_target,
        produced.spp,
        produced.max_bounces,
        produced.width,
        produced.height,
        produced.yaw.to_bits(),
        produced.pitch.to_bits(),
        produced.eye[0].to_bits(),
        produced.eye[1].to_bits(),
        produced.eye[2].to_bits(),
        produced.triangles,
        produced.specular,
        produced.materials.len(),
    )
}

/// Trace clock recorded beside an isolated floor file.
///
/// A loaded `.ref` reports seconds 0 because the file has no clock. That 0 is
/// not a measurement. A missing sidecar clock stays absent.
fn recorded_trace_seconds(reused: bool, traced: f32, prior_note: &str) -> Option<f32> {
    if reused && traced.to_bits() == 0.0f32.to_bits() {
        let seconds = json_fields(prior_note)
            .into_iter()
            .find(|(key, _)| key == "seconds")
            .and_then(|(_, value)| value.parse::<f32>().ok());
        return seconds.filter(|seconds| seconds.is_finite() && *seconds > 0.0);
    }
    traced.is_finite().then_some(traced)
}

fn cached_reuse_line(label: &str, commit: &str, key: u64) -> String {
    format!(
        "{label}: isolated file has provenance for {commit} key {key:016x}; render_cached reuses the saved file when its noise is at or under {NOISE_TARGET} or its spp is already at the setup cap, including a capped noisy file that is not traced again"
    )
}

fn write_floor_isolated_sidecar(
    note_path: &Path,
    commit: &str,
    key: u64,
    produced: &genos_debug::reference::RefSetup,
    reference: &genos_debug::Reference,
    promoted: bool,
    elapsed: Option<f32>,
) -> Result<(), String> {
    let note = floor_promotion_note(commit, key, produced, reference, promoted, elapsed);
    if let Some(parent) = note_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    std::fs::write(note_path, note).map_err(|err| err.to_string())
}

fn hall_isolated_note(
    commit: &str,
    key: u64,
    produced: &genos_debug::reference::RefSetup,
    reference: &genos_debug::Reference,
    promote_succeeded: bool,
) -> String {
    promotion_note(commit, key, produced, reference, promote_succeeded)
}

fn floor_isolated_trust(
    path: &Path,
    note: &str,
    commit: &str,
    produced: &genos_debug::reference::RefSetup,
    key: u64,
) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let trusted = note.contains(&format!("\"source_commit\": \"{commit}\""))
        && note.contains(&format!("\"cache_key\": \"{key:016x}\""))
        && note.contains("\"scene\": \"floor\"")
        && note.contains(&format!("\"acceptance_max_spp\": {MAX_SPP}"))
        && json_u32_field(note, "offline_max_spp") == Some(produced.max_spp)
        && note.contains(&format!("\"offline_spp_ceiling\": {OFFLINE_MAX_SPP}"))
        && note.contains(&format!("\"acceptance_seconds\": {TRACE_SECONDS}"))
        && note.contains(&format!("\"yaw_bits\": {}", produced.yaw.to_bits()))
        && note.contains(&format!("\"pitch_bits\": {}", produced.pitch.to_bits()))
        && note.contains(&format!(
            "\"eye_bits\": [{}, {}, {}]",
            produced.eye[0].to_bits(),
            produced.eye[1].to_bits(),
            produced.eye[2].to_bits()
        ))
        && note.contains(&format!("\"bounces\": {}", produced.max_bounces))
        && note.contains(&format!("\"spp_batch\": {}", produced.spp))
        && note.contains("\"estimator\": \"cpu-analytic\"")
        && note.contains("\"triangles\": false")
        && note.contains("\"specular\": false");
    if trusted {
        Ok(())
    } else {
        Err(format!(
            "refusing untrusted isolated reference {}",
            path.display()
        ))
    }
}

/// Copy a converged floor trace into the acceptance cache. A miss leaves that
/// directory byte for byte, including a file that was already there.
fn promote_floor_isolated(promotion: &Promotion<'_>) -> Result<PathBuf, String> {
    let commit = promotion.commit;
    if commit.len() != 40
        || !commit.chars().all(|c| c.is_ascii_hexdigit())
        || commit != promotion.expected_commit
    {
        return Err(format!("source commit {commit} is not this precompute"));
    }
    if promotion.acceptance.max_spp != MAX_SPP {
        return Err("acceptance spp cap is not 4096".into());
    }
    if promotion.acceptance.seconds.to_bits() != TRACE_SECONDS.to_bits() {
        return Err("acceptance seconds are not 600".into());
    }
    floor_budget(promotion.produced.max_spp, promotion.produced.seconds)?;
    shares_acceptance_identity(promotion.acceptance, promotion.produced)?;
    if promotion.produced.triangles
        || promotion.produced.specular
        || !promotion.produced.materials.is_empty()
    {
        return Err("floor precompute keeps CPU analytic shapes and the full picture".into());
    }
    if genos_debug::reference::gpu_wanted() {
        return Err("floor precompute refuses GENOS_REFERENCE_GPU".into());
    }
    let key = genos_debug::reference::cache_key(promotion.acceptance);
    if key != genos_debug::reference::cache_key(promotion.produced) {
        return Err("sampling budget leaked into the cache key".into());
    }
    reference_within_offline_cap(
        promotion.reference,
        promotion.acceptance.width,
        promotion.acceptance.height,
        promotion.produced.max_spp,
    )?;
    let loaded = genos_debug::reference::read_cached(
        promotion.isolated,
        promotion.acceptance.width,
        promotion.acceptance.height,
    )
    .ok_or_else(|| {
        format!(
            "isolated reference {} is missing, empty, or the wrong picture",
            promotion.isolated.display()
        )
    })?;
    reference_within_offline_cap(
        &loaded,
        promotion.acceptance.width,
        promotion.acceptance.height,
        promotion.produced.max_spp,
    )?;
    if !pictures_match(&loaded, promotion.reference) {
        return Err("isolated file does not match the trace".into());
    }
    std::fs::create_dir_all(promotion.acceptance_dir).map_err(|err| err.to_string())?;
    let dest = promotion.acceptance_dir.join(format!("{key:016x}.ref"));
    let note_path = promotion
        .acceptance_dir
        .join(format!("{key:016x}.provenance.json"));
    let note = floor_promotion_note(
        commit,
        key,
        promotion.produced,
        promotion.reference,
        true,
        finite_clock(promotion.reference.seconds),
    );
    publish_cached_pair(
        promotion.isolated,
        &dest,
        &note_path,
        &note,
        promotion.acceptance.width,
        promotion.acceptance.height,
    )?;
    Ok(dest)
}

fn parse_floor_spp(value: &str) -> Result<u32, String> {
    let text = value.trim();
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!(
            "CACHE_ENERGY_FLOOR_MAX_SPP {value} is not a u32 in 1..={OFFLINE_MAX_SPP}"
        ));
    }
    let spp: u32 = text.parse().map_err(|_| {
        format!("CACHE_ENERGY_FLOOR_MAX_SPP {value} is not a u32 in 1..={OFFLINE_MAX_SPP}")
    })?;
    if spp == 0 || spp > OFFLINE_MAX_SPP {
        return Err(format!(
            "CACHE_ENERGY_FLOOR_MAX_SPP {value} is outside 1..={OFFLINE_MAX_SPP}"
        ));
    }
    Ok(spp)
}

fn parse_floor_seconds(value: &str) -> Result<f32, String> {
    let seconds: f32 = value
        .trim()
        .parse()
        .map_err(|_| format!("CACHE_ENERGY_FLOOR_SECONDS {value} is not a number"))?;
    if !seconds.is_finite() || seconds <= 0.0 || seconds > PRECOMPUTE_SECONDS_CAP {
        return Err(format!(
            "CACHE_ENERGY_FLOOR_SECONDS {value} must be finite and in (0, {PRECOMPUTE_SECONDS_CAP}]"
        ));
    }
    Ok(seconds)
}

fn floor_budgets_from_env() -> Result<(u32, f32), String> {
    let spp = std::env::var("CACHE_ENERGY_FLOOR_MAX_SPP").map_err(|_| {
        format!("CACHE_ENERGY_FLOOR_MAX_SPP is required and must be in 1..={OFFLINE_MAX_SPP}")
    })?;
    let seconds = std::env::var("CACHE_ENERGY_FLOOR_SECONDS").map_err(|_| {
        "CACHE_ENERGY_FLOOR_SECONDS is required and must be finite and in (0, 2400]".to_string()
    })?;
    Ok((parse_floor_spp(&spp)?, parse_floor_seconds(&seconds)?))
}

fn precompute_seconds_from_env() -> Result<f32, String> {
    let value = std::env::var("CACHE_ENERGY_PRECOMPUTE_SECONDS").map_err(|_| {
        "CACHE_ENERGY_PRECOMPUTE_SECONDS is required and must be finite and in (0, 2400]"
            .to_string()
    })?;
    let seconds: f32 = value
        .parse()
        .map_err(|_| format!("CACHE_ENERGY_PRECOMPUTE_SECONDS {value} is not a number"))?;
    if !seconds.is_finite() || seconds <= 0.0 || seconds > PRECOMPUTE_SECONDS_CAP {
        return Err(format!(
            "CACHE_ENERGY_PRECOMPUTE_SECONDS {value} must be finite and in (0, {PRECOMPUTE_SECONDS_CAP}]"
        ));
    }
    Ok(seconds)
}

fn isolated_trust(
    path: &Path,
    note: &str,
    commit: &str,
    produced: &genos_debug::reference::RefSetup,
    key: u64,
) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let trusted = note.contains(&format!("\"source_commit\": \"{commit}\""))
        && note.contains(&format!("\"cache_key\": \"{key:016x}\""))
        && note.contains(&format!("\"yaw_bits\": {}", produced.yaw.to_bits()))
        && note.contains(&format!("\"pitch_bits\": {}", produced.pitch.to_bits()))
        && note.contains(&format!("\"bounces\": {}", produced.max_bounces))
        && note.contains(&format!("\"spp_batch\": {}", produced.spp))
        && note.contains("\"estimator\": \"cpu-analytic\"")
        && note.contains("\"triangles\": false")
        && note.contains("\"specular\": false");
    if trusted {
        Ok(())
    } else {
        Err(format!(
            "refusing untrusted isolated reference {}",
            path.display()
        ))
    }
}

fn precompute_hall_lit64_offline(seconds: f32) -> Result<String, String> {
    if genos_debug::reference::gpu_wanted() {
        return Err(
            "precompute refuses GENOS_REFERENCE_GPU; acceptance is the CPU analytic trace".into(),
        );
    }
    let (commit, clean) = source_commit(&repo_root())?;
    if !clean {
        return Err(format!("precompute refuses a dirty worktree at {commit}"));
    }
    let (scene, camera) = hall();
    let acceptance = acceptance_setup(&scene, &camera, 64);
    let produced = precompute_setup(&scene, &camera, 64, seconds)?;
    shares_acceptance_trace(&acceptance, &produced)?;
    if acceptance.seconds.to_bits() != TRACE_SECONDS.to_bits() {
        return Err("acceptance seconds changed".into());
    }
    let key = genos_debug::reference::cache_key(&acceptance);
    if key != genos_debug::reference::cache_key(&produced) {
        return Err("seconds leaked into the cache key".into());
    }
    let isolated_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/cache-energy/precompute-isolated")
        .join(&commit)
        .join(format!("{key:016x}"));
    let isolated = isolated_dir.join(format!("{key:016x}.ref"));
    let note_path = isolated_dir.join("provenance.json");
    let existing = if isolated.exists() {
        std::fs::read_to_string(&note_path).unwrap_or_default()
    } else {
        String::new()
    };
    isolated_trust(&isolated, &existing, &commit, &produced, key)?;
    if isolated.exists() {
        eprintln!("{}", cached_reuse_line("precompute", &commit, key));
    }
    eprintln!(
        "precompute identity\ncommit {commit}\nscene hall\nlit power 1\nbounces 64\nkey {key:016x}\neye {:?} yaw {} bits {} pitch {} bits {}\nsize {}x{}\nspp_batch {} noise_target {} max_spp {} seconds {}\ntriangles {} specular {} materials {}\nacceptance_seconds {}\nisolated {}\nacceptance_cache {}",
        produced.eye,
        produced.yaw,
        produced.yaw.to_bits(),
        produced.pitch,
        produced.pitch.to_bits(),
        produced.width,
        produced.height,
        produced.spp,
        produced.noise_target,
        produced.max_spp,
        produced.seconds,
        produced.triangles,
        produced.specular,
        produced.materials.len(),
        acceptance.seconds,
        isolated.display(),
        acceptance_cache().display()
    );
    let reference = genos_debug::reference::render_cached(&produced, &isolated_dir);
    let promote_result = promote_isolated(&Promotion {
        reference: &reference,
        produced: &produced,
        acceptance: &acceptance,
        commit: &commit,
        expected_commit: &commit,
        isolated: &isolated,
        acceptance_dir: &acceptance_cache(),
    });
    std::fs::create_dir_all(&isolated_dir).map_err(|err| err.to_string())?;
    let note = hall_isolated_note(&commit, key, &produced, &reference, promote_result.is_ok());
    std::fs::write(&note_path, note).map_err(|err| err.to_string())?;
    let dest = promote_result?;
    eprintln!(
        "precompute promoted {} noise {} spp {} seconds {}",
        dest.display(),
        reference.noise,
        reference.spp,
        reference.seconds
    );
    Ok(format!("{key:016x}"))
}

/// One open-floor trace at an explicit spp cap and time cap. A noisy result
/// stays in the isolated directory and is not copied, and this does not trace
/// a second time. The sidecar is metadata, not a proof of the samples.
fn precompute_floor_b64_offline(max_spp: u32, seconds: f32) -> Result<String, String> {
    if genos_debug::reference::gpu_wanted() {
        return Err(
            "floor precompute refuses GENOS_REFERENCE_GPU; acceptance is the CPU analytic trace"
                .into(),
        );
    }
    let (commit, clean) = source_commit(&repo_root())?;
    if !clean {
        return Err(format!("floor precompute refuses a dirty worktree at {commit}"));
    }
    let (scene, camera) = open_floor();
    let acceptance = acceptance_setup(&scene, &camera, 64);
    let produced = floor_precompute_setup(&scene, &camera, 64, max_spp, seconds)?;
    shares_acceptance_identity(&acceptance, &produced)?;
    if acceptance.max_spp != MAX_SPP || acceptance.seconds.to_bits() != TRACE_SECONDS.to_bits() {
        return Err("acceptance budget changed".into());
    }
    let key = genos_debug::reference::cache_key(&acceptance);
    if key != genos_debug::reference::cache_key(&produced) {
        return Err("sampling budget leaked into the cache key".into());
    }
    let isolated_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/cache-energy/precompute-isolated")
        .join(&commit)
        .join(format!("{key:016x}"));
    let isolated = isolated_dir.join(format!("{key:016x}.ref"));
    let note_path = isolated_dir.join("provenance.json");
    let existing = if isolated.exists() {
        std::fs::read_to_string(&note_path).unwrap_or_default()
    } else {
        String::new()
    };
    floor_isolated_trust(&isolated, &existing, &commit, &produced, key)?;
    let reused = isolated.exists();
    if reused {
        eprintln!("{}", cached_reuse_line("floor precompute", &commit, key));
    }
    eprintln!(
        "floor precompute identity\ncommit {commit}\nscene floor\nbounces 64\nkey {key:016x}\neye {:?} yaw {} bits {} pitch {} bits {}\nsize {}x{}\nspp_batch {} noise_target {} acceptance_max_spp {} offline_max_spp {} offline_spp_ceiling {OFFLINE_MAX_SPP}\nseconds_budget {} acceptance_seconds {}\ntriangles {} specular {} materials {}\nisolated {}\nacceptance_cache {}",
        produced.eye,
        produced.yaw,
        produced.yaw.to_bits(),
        produced.pitch,
        produced.pitch.to_bits(),
        produced.width,
        produced.height,
        produced.spp,
        produced.noise_target,
        acceptance.max_spp,
        produced.max_spp,
        produced.seconds,
        acceptance.seconds,
        produced.triangles,
        produced.specular,
        produced.materials.len(),
        isolated.display(),
        acceptance_cache().display()
    );
    let traced = genos_debug::reference::render_cached(&produced, &isolated_dir);
    let elapsed = recorded_trace_seconds(reused, traced.seconds, &existing);
    let mut reference = traced;
    // The saved file has no clock, so a reload reports 0. NaN is only the
    // in-memory mark that the sidecar clock was missing; the note writes null.
    reference.seconds = match elapsed {
        Some(seconds) => seconds,
        None if reused => f32::NAN,
        None => reference.seconds,
    };
    let promote_result = promote_floor_isolated(&Promotion {
        reference: &reference,
        produced: &produced,
        acceptance: &acceptance,
        commit: &commit,
        expected_commit: &commit,
        isolated: &isolated,
        acceptance_dir: &acceptance_cache(),
    });
    std::fs::create_dir_all(&isolated_dir).map_err(|err| err.to_string())?;
    write_floor_isolated_sidecar(
        &note_path,
        &commit,
        key,
        &produced,
        &reference,
        promote_result.is_ok(),
        elapsed,
    )?;
    let dest = promote_result?;
    let clock = elapsed.map_or_else(|| "null".to_string(), |seconds| seconds.to_string());
    eprintln!(
        "floor precompute promoted {} noise {} spp {} seconds {}",
        dest.display(),
        reference.noise,
        reference.spp,
        clock
    );
    Ok(format!("{key:016x}"))
}

fn trace_acceptance(
    scene: &Scene,
    camera: &Camera,
    bounces: u32,
) -> Result<genos_debug::Reference, String> {
    let setup = acceptance_setup(scene, camera, bounces);
    let reference = genos_debug::reference::render_cached(&setup, &acceptance_cache());
    reference_verdict(&facts_of(&reference, TRACE_SECONDS))?;
    Ok(reference)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorldPhase {
    Prefix,
    Graded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DrawRole {
    Settle,
    BounceFirst,
    BounceSecond,
    DirectFirst,
    DirectSecond,
    Flush,
}

#[derive(Clone, Debug)]
struct DrawStep {
    frame: u64,
    phase: WorldPhase,
    role: DrawRole,
    arm_dump: Option<u64>,
}

#[derive(Debug, PartialEq)]
struct Boundary {
    picture_frame: u64,
    picture_index: u64,
    direct_frame: u64,
    dump_frame: u64,
    dump_ready_frame: u64,
}

struct LampSchedule {
    steps: Vec<DrawStep>,
    warm: Option<Boundary>,
    graded: Boundary,
    lamp_off_frame: Option<u64>,
    /// `(copied frame, draw that wrote it)`. The write runs in `lighting_begin`,
    /// before that draw increments, once the counter is already `last + 3`.
    ready: Vec<(u64, u64)>,
}

struct Clock {
    frame: u64,
    dump_at: Option<u64>,
    pending: Vec<u64>,
    ready: Vec<(u64, u64)>,
}

impl Clock {
    fn draw(&mut self, arm: Option<u64>) -> u64 {
        if let Some(frame) = arm {
            self.dump_at = Some(frame);
        }
        let produced = self.frame + 1;
        if let Some(last) = self.pending.iter().copied().max() {
            if self.frame >= last + 3 {
                for frame in self.pending.drain(..) {
                    self.ready.push((frame, produced));
                }
            }
        }
        self.frame = produced;
        if let Some(target) = self.dump_at {
            if produced == target || produced == target + 1 {
                self.pending.push(produced);
            }
        }
        produced
    }
}

struct Builder {
    clock: Clock,
    steps: Vec<DrawStep>,
}

impl Builder {
    fn draw(&mut self, phase: WorldPhase, role: DrawRole, arm: Option<u64>) -> u64 {
        let frame = self.clock.draw(arm);
        self.steps.push(DrawStep { frame, phase, role, arm_dump: arm });
        frame
    }
}

fn role_before_picture(left: u64) -> DrawRole {
    match left {
        0 => DrawRole::BounceSecond,
        1 => DrawRole::BounceFirst,
        2 => DrawRole::DirectSecond,
        3 => DrawRole::DirectFirst,
        _ => DrawRole::Settle,
    }
}

fn build_schedule(prefix: u64, graded: u64, warm_grade: bool) -> LampSchedule {
    if warm_grade {
        assert!(prefix >= 4, "warm prefix");
        assert!(graded > 5, "dark run");
    } else {
        assert!(graded >= 4, "graded run");
    }
    let mut builder = Builder {
        clock: Clock { frame: 0, dump_at: None, pending: Vec::new(), ready: Vec::new() },
        steps: Vec::new(),
    };
    if warm_grade {
        for index in 1..=prefix {
            let arm = (index == 1).then_some(prefix);
            builder.draw(WorldPhase::Prefix, role_before_picture(prefix - index), arm);
        }
        let dark_picture = prefix + graded;
        for index in 1..=graded {
            let role = match graded - index {
                0 => DrawRole::BounceSecond,
                1 => DrawRole::BounceFirst,
                _ => DrawRole::Settle,
            };
            let next = builder.clock.frame + 1;
            let arm = (next == prefix + 5).then_some(dark_picture);
            builder.draw(WorldPhase::Graded, role, arm);
        }
        builder.draw(WorldPhase::Graded, DrawRole::DirectFirst, None);
        builder.draw(WorldPhase::Graded, DrawRole::DirectSecond, None);
        finish_dump(&mut builder, dark_picture);
    } else {
        for _ in 0..prefix {
            builder.draw(WorldPhase::Prefix, DrawRole::Settle, None);
        }
        let picture = prefix + graded;
        for index in 1..=graded {
            let arm = (index == 1).then_some(picture);
            builder.draw(WorldPhase::Graded, role_before_picture(graded - index), arm);
        }
        finish_dump(&mut builder, picture);
    }
    let warm = warm_grade.then(|| boundary_of(&builder.steps, &builder.clock.ready, WorldPhase::Prefix));
    let graded_boundary = boundary_of(&builder.steps, &builder.clock.ready, WorldPhase::Graded);
    let lamp_off_frame = warm_grade.then(|| {
        builder
            .steps
            .iter()
            .find(|step| step.phase == WorldPhase::Graded)
            .expect("dark draw")
            .frame
    });
    LampSchedule {
        steps: builder.steps,
        warm,
        graded: graded_boundary,
        lamp_off_frame,
        ready: builder.clock.ready,
    }
}

fn finish_dump(builder: &mut Builder, picture: u64) {
    let mut extra = 0u32;
    while !builder.clock.ready.iter().any(|(frame, _)| *frame == picture) {
        extra += 1;
        assert!(extra <= 8, "dump of frame {picture} was not written");
        builder.draw(WorldPhase::Graded, DrawRole::Flush, None);
    }
}

fn boundary_of(steps: &[DrawStep], ready: &[(u64, u64)], phase: WorldPhase) -> Boundary {
    let picture = steps
        .iter()
        .find(|step| step.phase == phase && step.role == DrawRole::BounceSecond)
        .expect("bounce picture");
    let direct = steps
        .iter()
        .find(|step| step.phase == phase && step.role == DrawRole::DirectSecond)
        .expect("direct picture");
    let picture_index = steps.iter().filter(|step| step.phase == phase && step.frame <= picture.frame).count() as u64;
    let dump_ready_frame = ready
        .iter()
        .find(|(frame, _)| *frame == picture.frame)
        .map(|(_, at)| *at)
        .expect("dump write");
    Boundary {
        picture_frame: picture.frame,
        picture_index,
        direct_frame: direct.frame,
        dump_frame: picture.frame,
        dump_ready_frame,
    }
}

fn acceptance_energy(frames: u64) -> LampSchedule {
    build_schedule(0, frames, false)
}

fn acceptance_lag(dark_prefix: u64, lit: u64) -> LampSchedule {
    build_schedule(dark_prefix, lit, false)
}

fn acceptance_linger(warm: u64, dark: u64) -> LampSchedule {
    build_schedule(warm, dark, true)
}

struct RejectedFrames {
    lamp_off: u64,
    dump_dark_index: u64,
    bounce_dark_index: u64,
    dump_lit_index: u64,
    bounce_lit_index: u64,
}

fn rejected_linger_frames(warm: u64, dark: u64) -> RejectedFrames {
    let lamp_off = warm + 4 + 1;
    let dump = warm + dark - 8;
    let bounce = warm + 4 + dark + 2;
    RejectedFrames {
        lamp_off,
        dump_dark_index: dump - lamp_off + 1,
        bounce_dark_index: bounce - lamp_off + 1,
        dump_lit_index: 0,
        bounce_lit_index: 0,
    }
}

fn rejected_lag_frames(dark_prefix: u64, lit: u64) -> RejectedFrames {
    let first = dark_prefix + 1;
    let dump = dark_prefix + lit - 8;
    let bounce = dark_prefix + lit + 2;
    RejectedFrames {
        lamp_off: 0,
        dump_dark_index: 0,
        bounce_dark_index: 0,
        dump_lit_index: dump - first + 1,
        bounce_lit_index: bounce - first + 1,
    }
}

fn ref_luma(reference: &genos_debug::Reference, index: usize) -> f64 {
    let channel = reference.linear[index];
    luma([f64::from(channel[0]), f64::from(channel[1]), f64::from(channel[2])])
}

fn bounce_sums(
    live: &[Option<f64>],
    direct: &genos_debug::Reference,
    full: &genos_debug::Reference,
) -> Result<(f64, f64, usize), String> {
    let mut live_sum = 0.0;
    let mut ref_sum = 0.0;
    let mut count = 0usize;
    for (index, sample) in live.iter().enumerate() {
        if !full.hit[index] {
            continue;
        }
        let Some(value) = *sample else { continue };
        let observed = finite("observed bounce pixel", value)?;
        let bounce = (ref_luma(full, index) - ref_luma(direct, index)).max(0.0);
        finite("reference bounce pixel", bounce)?;
        live_sum += observed;
        ref_sum += bounce;
        count += 1;
    }
    if count == 0 {
        return Err("observed bounce count is 0".into());
    }
    Ok((finite("observed bounce", live_sum)?, finite("reference bounce", ref_sum)?, count))
}

fn direct_sums(live: &[Option<f64>], reference: &genos_debug::Reference) -> Result<(f64, f64, usize), String> {
    let mut live_sum = 0.0;
    let mut ref_sum = 0.0;
    let mut count = 0usize;
    for (index, sample) in live.iter().enumerate() {
        if !reference.hit[index] {
            continue;
        }
        let Some(value) = *sample else { continue };
        live_sum += finite("observed direct pixel", value)?;
        ref_sum += finite("reference direct pixel", ref_luma(reference, index))?;
        count += 1;
    }
    if count == 0 {
        return Err("observed direct count is 0".into());
    }
    Ok((finite("observed direct", live_sum)?, finite("reference direct", ref_sum)?, count))
}

fn blotch_of(live: &[Option<f64>], direct: &genos_debug::Reference, full: &genos_debug::Reference) -> Option<f64> {
    let (mut err, mut weight) = (0.0, 0.0);
    for by in (0..HEIGHT).step_by(8) {
        for bx in (0..WIDTH).step_by(8) {
            let (mut left, mut right, mut count) = (0.0, 0.0, 0usize);
            for y in by..(by + 8).min(HEIGHT) {
                for x in bx..(bx + 8).min(WIDTH) {
                    let index = (y * WIDTH + x) as usize;
                    if let (true, Some(value)) = (full.hit[index], live[index]) {
                        if !value.is_finite() {
                            return None;
                        }
                        left += value;
                        right += (ref_luma(full, index) - ref_luma(direct, index)).max(0.0);
                        count += 1;
                    }
                }
            }
            if count >= 32 && right > 0.0 {
                err += (left - right).abs();
                weight += right;
            }
        }
    }
    (weight > 0.0 && err.is_finite() && weight.is_finite()).then_some(err / weight)
}

struct PictureSums {
    live_bounce: f64,
    ref_bounce: f64,
    live_direct: f64,
    ref_direct: f64,
    bounce_n: usize,
    direct_n: usize,
    blotch: Option<f64>,
}

fn picture_sums(
    bounce: &[Option<f64>],
    direct_picture: &[Option<f64>],
    bounce_b0: &genos_debug::Reference,
    bounce_b64: &genos_debug::Reference,
    direct_reference: &genos_debug::Reference,
) -> Result<PictureSums, String> {
    let (live_bounce, ref_bounce, bounce_n) = bounce_sums(bounce, bounce_b0, bounce_b64)?;
    let (live_direct, ref_direct, direct_n) = direct_sums(direct_picture, direct_reference)?;
    Ok(PictureSums {
        live_bounce,
        ref_bounce,
        live_direct,
        ref_direct,
        bounce_n,
        direct_n,
        blotch: blotch_of(bounce, bounce_b0, bounce_b64),
    })
}

struct SlotGrade {
    live: f64,
    reference: f64,
    n: usize,
    median: f64,
    under: f64,
    median_lit: f64,
}

fn slot_share(grade: &SlotGrade) -> Result<f64, String> {
    if grade.n == 0 {
        return Err("observed cell count is 0".into());
    }
    linger_share(grade.live, grade.reference)
}

type Slot = ([f32; 3], [f32; 3], [f32; 3], f32);

struct CellGrade {
    coarse: SlotGrade,
    fine: SlotGrade,
}

fn cell_grade(scene: &Scene, dir: &Path, at: u64, tag: &str) -> Result<CellGrade, String> {
    let layout = std::fs::read_to_string(dir.join(format!("f{at}_layout.json")))
        .map_err(|_| format!("missing lighting dump layout f{at} ({})", dir_list(dir)))?;
    let slots: usize = layout
        .split("\"cache_slots\": ")
        .nth(1)
        .and_then(|text| text.trim_end_matches(|c: char| !c.is_ascii_digit()).split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| format!("cache_slots missing in f{at}"))?;
    let words = |name: &str| -> Result<Vec<u32>, String> {
        let bytes = std::fs::read(dir.join(format!("f{at}_{name}.bin")))
            .map_err(|_| format!("missing f{at}_{name}.bin ({})", dir_list(dir)))?;
        Ok(bytes.as_chunks::<4>().0.iter().map(|word| u32::from_le_bytes(*word)).collect())
    };
    let keys = words("keys")?;
    let cache = words("cache")?;
    let channel = |index: usize| f32::from_bits(cache[index]);
    let vec3 = |row: usize| [channel(4 * row), channel(4 * row + 1), channel(4 * row + 2)];
    let mut coarse = Vec::new();
    let mut fine = Vec::new();
    for (index, _) in keys.iter().take(slots).enumerate().filter(|(_, key)| **key != 0) {
        let size = channel(4 * (3 * index + 1) + 3);
        let sets = channel(4 * (3 * index + 2) + 3);
        if sets.is_nan() || sets < 1.0 {
            continue;
        }
        let slot = (vec3(3 * index), vec3(3 * index + 1), vec3(3 * index + 2), sets);
        if size > 0.13 {
            coarse.push(slot);
        } else {
            fine.push(slot);
        }
    }
    let grade = |all: Vec<Slot>, label: String| -> Result<SlotGrade, String> {
        if all.is_empty() {
            return Ok(SlotGrade { live: 0.0, reference: 0.0, n: 0, median: 0.0, under: 0.0, median_lit: 0.0 });
        }
        let step = all.len().div_ceil(CELL_SAMPLES);
        let picked: Vec<_> = all.into_iter().step_by(step).collect();
        let threads = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
        let refs: Vec<f64> = std::thread::scope(|scope| {
            let chunk = picked.len().div_ceil(threads);
            let handles: Vec<_> = picked
                .chunks(chunk)
                .enumerate()
                .map(|(chunk_index, part)| {
                    scope.spawn(move || {
                        part.iter()
                            .enumerate()
                            .map(|(offset, (point, normal, _, _))| {
                                let energy = genos_debug::reference::bounce_irradiance(
                                    scene,
                                    *point,
                                    *normal,
                                    CELL_RAYS,
                                    64,
                                    (chunk_index * chunk + offset) as u64,
                                );
                                luma([f64::from(energy[0]), f64::from(energy[1]), f64::from(energy[2])])
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles.into_iter().flat_map(|handle| handle.join().expect("trace")).collect()
        });
        let live: Vec<f64> = picked
            .iter()
            .map(|(_, _, value, _)| luma([f64::from(value[0]), f64::from(value[1]), f64::from(value[2])]))
            .collect();
        if live.iter().any(|value| !value.is_finite()) || refs.iter().any(|value| !value.is_finite()) {
            return Err(format!("{label} cell sample is nonfinite"));
        }
        let total_ref: f64 = refs.iter().sum();
        let total_live: f64 = live.iter().sum();
        let floor = total_ref / refs.len() as f64 * 0.05;
        let mut per: Vec<f64> = live.iter().zip(&refs).filter(|(_, reference)| **reference > floor).map(|(value, reference)| value / reference).collect();
        per.sort_by(f64::total_cmp);
        let lit = total_ref / refs.len() as f64 * LIT;
        let mut per_lit: Vec<f64> = live.iter().zip(&refs).filter(|(_, reference)| **reference > lit).map(|(value, reference)| value / reference).collect();
        per_lit.sort_by(f64::total_cmp);
        Ok(SlotGrade {
            median_lit: per_lit.get(per_lit.len() / 2).copied().unwrap_or(0.0),
            live: total_live,
            reference: total_ref,
            n: picked.len(),
            median: per.get(per.len() / 2).copied().unwrap_or(0.0),
            under: per.iter().filter(|ratio| **ratio < LOW).count() as f64 / per.len().max(1) as f64,
        })
    };
    Ok(CellGrade { coarse: grade(coarse, format!("{tag}_coarse"))?, fine: grade(fine, format!("{tag}_fine"))? })
}

fn energy_holds(picture: &PictureSums, cells: &SlotGrade) -> Result<(), String> {
    let direct = direct_oracle(picture.live_direct, picture.ref_direct)?;
    if !direct_ok(direct) {
        return Err(format!("direct oracle {direct:?} (n {})", picture.direct_n));
    }
    let bounce = linger_share(picture.live_bounce, picture.ref_bounce)?;
    if !(LOW..=HIGH).contains(&bounce) {
        return Err(format!(
            "bounce {bounce} outside [{LOW}, {HIGH}] (lit denominator {}, n {})",
            picture.ref_bounce, picture.bounce_n
        ));
    }
    let cells_ratio = slot_share(cells)?;
    if cells.n < 32 || cells_ratio < LOW {
        return Err(format!("25 cm cells {cells_ratio} (n {}) under {LOW}", cells.n));
    }
    if !cells.median_lit.is_finite() || cells.median_lit < LOW {
        return Err(format!(
            "median lit 25 cm cell {} under {LOW} (median {}, under {})",
            cells.median_lit, cells.median, cells.under
        ));
    }
    Ok(())
}

fn linger_holds(picture: &PictureSums, cells: &SlotGrade) -> Result<(), String> {
    let direct = direct_oracle(picture.live_direct, picture.ref_direct)?;
    if !direct_ok(direct) {
        return Err(format!(
            "dark direct oracle {direct:?}; black/black is an exact match, not a ratio (n {})",
            picture.direct_n
        ));
    }
    let share = linger_share(picture.live_bounce, picture.ref_bounce)?;
    if share > LINGER {
        return Err(format!(
            "bounce linger {share} above {LINGER}; lit denominator {}",
            picture.ref_bounce
        ));
    }
    let cells_ratio = slot_share(cells)?;
    if cells.n < 32 || cells_ratio > LINGER {
        return Err(format!(
            "25 cm cells {cells_ratio} (n {}, median diagnostic {}) above {LINGER}; lit denominator {}",
            cells.n, cells.median, cells.reference
        ));
    }
    Ok(())
}

fn decode(pixels: &[u8]) -> Vec<Option<f64>> {
    assert_eq!(pixels.len(), (WIDTH * HEIGHT * 4) as usize, "picture size");
    let untone = |byte: u8| {
        let value = f64::from(byte) / 255.0;
        if value <= 0.64 {
            value
        } else {
            let x = value - 0.64;
            1.1 * x / (0.14 - x) + 0.64
        }
    };
    pixels
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| {
            pixel[..3]
                .iter()
                .all(|&byte| byte <= SHOULDER_TOP)
                .then(|| luma([untone(pixel[2]), untone(pixel[1]), untone(pixel[0])]))
        })
        .collect()
}

struct Shots {
    bounce: Option<Vec<Option<f64>>>,
    direct: Option<Vec<Option<f64>>>,
}

impl Shots {
    fn empty() -> Self {
        Self { bounce: None, direct: None }
    }

    fn take(&mut self, role: DrawRole, pixels: Vec<Option<f64>>) {
        match role {
            DrawRole::BounceSecond => self.bounce = Some(pixels),
            DrawRole::DirectSecond => self.direct = Some(pixels),
            DrawRole::Settle | DrawRole::BounceFirst | DrawRole::DirectFirst | DrawRole::Flush => {}
        }
    }
}

struct Drawn {
    warm_captured: bool,
    graded_bounce: Vec<Option<f64>>,
    graded_direct: Vec<Option<f64>>,
    settled_at: Option<u64>,
    entries: Vec<f64>,
    draining: u32,
}

fn draw_pixels(
    artifacts: &Path,
    schedule: &LampSchedule,
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
) -> Vec<u8> {
    let _ = window.pump();
    match renderer.draw(world, camera, true) {
        Ok(Some(pixels)) => pixels,
        Ok(None) => keep_fail(artifacts, "draw returned no picture"),
        Err(err) => keep_fail(artifacts, &format!("draw failed: {err} ({})", schedule.graded.picture_frame)),
    }
}

fn world_of<'a>(step: &DrawStep, prefix: Option<&'a World>, graded: &'a World) -> &'a World {
    match step.phase {
        WorldPhase::Prefix => prefix.expect("prefix world"),
        WorldPhase::Graded => graded,
    }
}

struct FrameHost<'a> {
    window: &'a mut Window,
    renderer: &'a mut Renderer,
    camera: &'a Camera,
    artifacts: &'a Path,
}

struct WarmGate<'a> {
    scene: Option<&'a Scene>,
    b0: Option<&'a genos_debug::Reference>,
    b64: Option<&'a genos_debug::Reference>,
}

fn execute(
    schedule: &LampSchedule,
    prefix: Option<&World>,
    graded: &World,
    host: &mut FrameHost<'_>,
    warm: &WarmGate<'_>,
) -> Drawn {
    let mut warm_shots = Shots::empty();
    let mut graded_shots = Shots::empty();
    let mut settled_at = None;
    let mut entries = Vec::new();
    let mut draining = 0u32;
    for step in &schedule.steps {
        if let Some(frame) = step.arm_dump {
            std::env::set_var("GENOS_LIGHTING_DUMP", frame.to_string());
        }
        let world = world_of(step, prefix, graded);
        match step.role {
            DrawRole::BounceFirst => host.renderer.set_debug_view(DebugView { mode: ViewMode::Bounce, lamp: 0 }),
            DrawRole::DirectFirst => host.renderer.set_debug_view(DebugView { mode: ViewMode::Direct, lamp: 0 }),
            DrawRole::Settle | DrawRole::BounceSecond | DrawRole::DirectSecond | DrawRole::Flush => {}
        }
        let pixels = draw_pixels(host.artifacts, schedule, host.window, host.renderer, world, host.camera);
        if matches!(step.role, DrawRole::BounceSecond | DrawRole::DirectSecond) {
            host.renderer.set_debug_view(DebugView::default());
            let decoded = decode(&pixels);
            if step.phase == WorldPhase::Prefix {
                warm_shots.take(step.role, decoded);
            } else {
                graded_shots.take(step.role, decoded);
            }
        }
        if step.frame == 1 {
            let resolved = host.renderer.lighting_settings().surface_cache.sh_cells;
            if !resolved {
                keep_fail(
                    host.artifacts,
                    "resolved surface_cache.sh_cells is false after apply_env; a preset cleared it or the override was not applied",
                );
            }
        }
        let stats = host.renderer.settle_stats();
        if stats.settled && settled_at.is_none() {
            settled_at = Some(step.frame);
        }
        if step.phase == WorldPhase::Graded
            && step.role == DrawRole::Settle
            && schedule.graded.picture_frame.saturating_sub(step.frame) < 120
        {
            entries.push(f64::from(stats.round_entries));
            draining += u32::from(stats.draining);
        }
        if schedule.warm.as_ref().is_some_and(|warm| step.frame == warm.dump_ready_frame) {
            grade_warm(host.artifacts, schedule, warm.scene, warm.b0, warm.b64, &warm_shots);
        }
    }
    let graded_bounce = graded_shots.bounce.unwrap_or_else(|| keep_fail(host.artifacts, "graded bounce picture was not read"));
    let graded_direct = graded_shots.direct.unwrap_or_else(|| keep_fail(host.artifacts, "graded direct picture was not read"));
    let warm_captured = match (&schedule.warm, warm_shots.bounce, warm_shots.direct) {
        (None, _, _) => false,
        (Some(_), Some(_), Some(_)) => true,
        (Some(_), _, _) => keep_fail(host.artifacts, "warm bounce and direct were not both read"),
    };
    Drawn { warm_captured, graded_bounce, graded_direct, settled_at, entries, draining }
}

fn grade_warm(
    artifacts: &Path,
    schedule: &LampSchedule,
    scene: Option<&Scene>,
    b0: Option<&genos_debug::Reference>,
    b64: Option<&genos_debug::Reference>,
    shots: &Shots,
) {
    let (Some(scene), Some(b0), Some(b64)) = (scene, b0, b64) else {
        keep_fail(artifacts, "warm grade has no lit reference");
    };
    let Some(warm) = &schedule.warm else {
        return;
    };
    let layout = artifacts.join(format!("f{}_layout.json", warm.picture_frame));
    if !layout.exists() {
        keep_fail(
            artifacts,
            &format!("warm dump f{} missing at draw {} ({})", warm.picture_frame, warm.dump_ready_frame, dir_list(artifacts)),
        );
    }
    let (Some(bounce), Some(direct)) = (shots.bounce.as_ref(), shots.direct.as_ref()) else {
        keep_fail(artifacts, "warm pictures were not read before the dump was written");
    };
    let picture = match picture_sums(bounce, direct, b0, b64, b0) {
        Ok(picture) => picture,
        Err(err) => keep_fail(artifacts, &format!("warm picture: {err}")),
    };
    let cells = match cell_grade(scene, artifacts, warm.picture_frame, "warm") {
        Ok(cells) => cells,
        Err(err) => keep_fail(artifacts, &format!("warm cells: {err}")),
    };
    if let Err(err) = energy_holds(&picture, &cells.coarse) {
        keep_fail(
            artifacts,
            &format!(
                "warm energy before lamp off failed: {err}; lit denominator {}; cell median diagnostic {}",
                picture.ref_bounce, cells.coarse.median
            ),
        );
    }
    eprintln!(
        "cache_energy warm: picture {:.3} cells {} lit denominator {} frame {}",
        linger_share(picture.live_bounce, picture.ref_bounce).unwrap_or(0.0),
        slot_share(&cells.coarse).map_or_else(|err| err, |ratio| format!("{ratio:.3}")),
        picture.ref_bounce,
        warm.picture_frame
    );
}

#[derive(Clone, Copy)]
enum Measure {
    Energy { frames: u64 },
    Lag { dark: u64, lit: u64 },
    Linger { warm: u64, dark: u64 },
}

fn schedule_of(measure: &Measure) -> LampSchedule {
    match *measure {
        Measure::Energy { frames } => acceptance_energy(frames),
        Measure::Lag { dark, lit } => acceptance_lag(dark, lit),
        Measure::Linger { warm, dark } => acceptance_linger(warm, dark),
    }
}

fn schedule_json(schedule: &LampSchedule) -> String {
    let warm = schedule.warm.as_ref().map_or(0, |boundary| boundary.picture_frame);
    format!(
        "\"warm_picture_frame\": {warm}, \"warm_dump_ready\": {}, \"graded_picture_frame\": {}, \"graded_picture_index\": {}, \"graded_direct_frame\": {}, \"graded_dump_ready\": {}, \"lamp_off_frame\": {}",
        schedule.warm.as_ref().map_or(0, |boundary| boundary.dump_ready_frame),
        schedule.graded.picture_frame,
        schedule.graded.picture_index,
        schedule.graded.direct_frame,
        schedule.graded.dump_ready_frame,
        schedule.lamp_off_frame.unwrap_or(0)
    )
}

fn measured_json(
    name: &str,
    schedule: &LampSchedule,
    detail: &str,
    verdict: &str,
    provenance: &str,
) -> String {
    let schedule = if provenance.is_empty() {
        format!("{},", schedule_json(schedule))
    } else {
        format!("{},\n  {provenance},", schedule_json(schedule))
    };
    format!(
        "{{\n  \"verdict\": \"{verdict}\",\n  \"name\": \"{name}\",\n  \"detail\": \"{}\",\n  {schedule}\n  \"files\": \"{}\"\n}}\n",
        escape(detail),
        escape(&dir_list(&artifact_dir(name)))
    )
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn source_commit(repo: &Path) -> Result<(String, bool), String> {
    let head = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|err| err.to_string())?;
    if !head.status.success() {
        return Err(String::from_utf8_lossy(&head.stderr).trim().to_string());
    }
    let commit = String::from_utf8_lossy(&head.stdout).trim().to_string();
    if commit.len() != 40 || !commit.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("git HEAD is not a commit: {commit}"));
    }
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["status", "--porcelain"])
        .output()
        .map_err(|err| err.to_string())?;
    if !status.status.success() {
        return Err(String::from_utf8_lossy(&status.stderr).trim().to_string());
    }
    Ok((commit, status.stdout.iter().all(u8::is_ascii_whitespace)))
}

fn trace_provenance(camera: &Camera, graded: &Scene, lit: Option<&Scene>) -> String {
    let (commit, clean) = match source_commit(&repo_root()) {
        Ok(pair) => pair,
        Err(err) => (err, false),
    };
    let b0 = genos_debug::reference::cache_key(&acceptance_setup(graded, camera, 0));
    let b64 = genos_debug::reference::cache_key(&acceptance_setup(graded, camera, 64));
    let mut rows = vec![
        format!("\"source_commit\": \"{}\"", escape(&commit)),
        format!("\"worktree_clean\": {clean}"),
        format!("\"cache_key_b0\": \"{b0:016x}\""),
        format!("\"cache_key_b64\": \"{b64:016x}\""),
        "\"estimator\": \"cpu-analytic\"".to_string(),
        format!("\"acceptance_seconds\": {TRACE_SECONDS}"),
        format!("\"noise_target\": {NOISE_TARGET}"),
        format!("\"max_spp\": {MAX_SPP}"),
        "\"spp_batch\": 32".to_string(),
    ];
    if let Some(scene) = lit {
        let key = genos_debug::reference::cache_key(&acceptance_setup(scene, camera, 64));
        rows.push(format!("\"lit_cache_key_b64\": \"{key:016x}\""));
    }
    rows.join(", ")
}

struct TracePair {
    b0: genos_debug::Reference,
    b64: genos_debug::Reference,
}

fn load_traces(
    name: &str,
    schedule: &LampSchedule,
    artifacts: &Path,
    camera: &Camera,
    graded_scene: &Scene,
    lit_scene: Option<&Scene>,
    provenance: &str,
) -> (TracePair, genos_debug::Reference) {
    let blocked = |err: String| -> ! {
        keep(
            artifacts,
            &measured_json(name, schedule, &err, "fail", provenance),
        );
        abort_kept(&err);
    };
    let graded_b0 = match trace_acceptance(graded_scene, camera, 0) {
        Ok(reference) => reference,
        Err(err) => blocked(err),
    };
    if let Some(lit) = lit_scene {
        let b0 = match trace_acceptance(lit, camera, 0) {
            Ok(reference) => reference,
            Err(err) => blocked(err),
        };
        let b64 = match trace_acceptance(lit, camera, 64) {
            Ok(reference) => reference,
            Err(err) => blocked(err),
        };
        (TracePair { b0, b64 }, graded_b0)
    } else {
        let b64 = match trace_acceptance(graded_scene, camera, 64) {
            Ok(reference) => reference,
            Err(err) => blocked(err),
        };
        let direct = graded_b0.clone();
        (TracePair { b0: graded_b0, b64 }, direct)
    }
}

fn apply_gate(measure: &Measure, drawn: &Drawn, picture: &PictureSums, cells: &CellGrade, artifacts: &Path) {
    match *measure {
        Measure::Linger { .. } => {
            if !drawn.warm_captured {
                keep_fail(artifacts, "linger finished without a warm picture");
            }
            if let Err(err) = linger_holds(picture, &cells.coarse) {
                keep_fail(artifacts, &err);
            }
        }
        Measure::Energy { .. } => {
            if let Err(err) = energy_holds(picture, &cells.coarse) {
                keep_fail(artifacts, &err);
            }
            if drawn.settled_at.is_none() {
                let cost = drawn.entries.iter().sum::<f64>() / drawn.entries.len().max(1) as f64;
                keep_fail(
                    artifacts,
                    &format!("a still scene never settled ({cost:.0} entries a frame, draining {})", drawn.draining),
                );
            }
        }
        Measure::Lag { .. } => {
            if let Err(err) = energy_holds(picture, &cells.coarse) {
                keep_fail(artifacts, &err);
            }
        }
    }
}

fn run_case(
    name: &str,
    measure: Measure,
    graded_world: &World,
    prefix_world: Option<&World>,
    graded_scene: &Scene,
    lit_scene: Option<&Scene>,
    camera: &Camera,
) {
    let artifacts = artifact_dir(name);
    keep(&artifacts, "{\n  \"verdict\": \"running\"\n}\n");
    if let Err(err) = acceptance_from_process() {
        keep_fail(&artifacts, &err);
    }
    let schedule = schedule_of(&measure);
    let provenance = trace_provenance(camera, graded_scene, lit_scene);
    keep(
        &artifacts,
        &measured_json(name, &schedule, "tracing", "running", &provenance),
    );
    let (bounce, direct_reference) = load_traces(
        name,
        &schedule,
        &artifacts,
        camera,
        graded_scene,
        lit_scene,
        &provenance,
    );
    eprintln!(
        "cache_energy {name}: b0 spp {} noise {:.4} seconds {:.1}, b64 spp {} noise {:.4} seconds {:.1}",
        bounce.b0.spp, bounce.b0.noise, bounce.b0.seconds, bounce.b64.spp, bounce.b64.noise, bounce.b64.seconds
    );
    std::env::set_var("GENOS_LIGHTING_DUMP_DIR", &artifacts);
    let mut window = match Window::open_proof(WIDTH, HEIGHT) {
        Ok(window) => window,
        Err(err) => keep_fail(&artifacts, &format!("proof window: {err}")),
    };
    let size = window.pump();
    let (renderer, permit) =
        match Renderer::open(window.display, window.surface, size.width, size.height) {
            Ok(open) => open,
            Err(err) => keep_fail(&artifacts, &format!("renderer: {err}")),
        };
    let mut owner = genos_window::Lifecycle::new(window, renderer, permit);
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let mut settings = LightingSettings::preset(LightingPreset::High);
    settings.surface_cache.sh_cells = true;
    renderer.set_lighting_settings(settings);
    let mut host = FrameHost {
        window,
        renderer,
        camera,
        artifacts: &artifacts,
    };
    let warm = WarmGate {
        scene: lit_scene,
        b0: Some(&bounce.b0),
        b64: Some(&bounce.b64),
    };
    let drawn = execute(&schedule, prefix_world, graded_world, &mut host, &warm);
    let picture = match picture_sums(
        &drawn.graded_bounce,
        &drawn.graded_direct,
        &bounce.b0,
        &bounce.b64,
        &direct_reference,
    ) {
        Ok(picture) => picture,
        Err(err) => keep_fail(&artifacts, &err),
    };
    let cell_scene = lit_scene.unwrap_or(graded_scene);
    let cells = match cell_grade(cell_scene, &artifacts, schedule.graded.picture_frame, name) {
        Ok(cells) => cells,
        Err(err) => keep_fail(&artifacts, &err),
    };
    let fine = slot_share(&cells.fine).map_or_else(|err| err, |ratio| format!("{ratio:.3}"));
    let blotch = picture
        .blotch
        .map_or_else(|| "none".to_string(), |value| format!("{value:.3}"));
    let detail = format!(
        "lit bounce denominator {} direct {:?} bounce n {} direct n {} cells live {} cells ref {} median {} median_lit {} fine {fine} blotch {blotch}",
        picture.ref_bounce,
        direct_oracle(picture.live_direct, picture.ref_direct).ok(),
        picture.bounce_n,
        picture.direct_n,
        cells.coarse.live,
        cells.coarse.reference,
        cells.coarse.median,
        cells.coarse.median_lit
    );
    keep(
        &artifacts,
        &measured_json(name, &schedule, &detail, "measured", &provenance),
    );
    eprintln!("cache_energy {name}: {detail}");
    apply_gate(&measure, &drawn, &picture, &cells, &artifacts);
    keep(
        &artifacts,
        &measured_json(name, &schedule, &detail, "pass", &provenance),
    );
}

fn run_energy(name: &str, scene: &Scene, camera: &Camera) {
    let world = World::from_scene(scene.clone());
    run_case(name, Measure::Energy { frames: u64::try_from(FRAMES).expect("frames") }, &world, None, scene, None, camera);
}

#[test]
#[ignore = "lavapipe and a CPU path trace, minutes"]
fn a_closed_box_reaches_the_reference_bounce() {
    let (scene, camera) = closed_box();
    run_energy("box", &scene, &camera);
}

#[test]
#[ignore = "lavapipe and a CPU path trace, minutes"]
fn an_open_floor_reaches_the_reference_bounce() {
    let (scene, camera) = open_floor();
    run_energy("floor", &scene, &camera);
}

#[test]
#[ignore = "lavapipe and a CPU path trace, minutes"]
fn the_night_hall_reaches_the_reference_bounce() {
    let (scene, camera) = hall();
    run_energy("hall", &scene, &camera);
}

/// Lag: 300 dark draws, then the 300th lit draw is both the bounce picture and
/// the cell dump. The energy gate is the existing 0.9 sum and median.
#[test]
#[ignore = "lavapipe and a CPU path trace, minutes"]
fn the_night_hall_fills_from_dark_in_300_frames() {
    let (dark, _) = hall_stage(0.0);
    let (scene, camera) = hall();
    let lit = World::from_scene(scene.clone());
    run_case("hall_lag", Measure::Lag { dark: 300, lit: 300 }, &lit, Some(&dark.world), &scene, None, &camera);
}

/// Linger: the 300th lit draw is the warm energy gate (picture and cells),
/// and the 120th dark draw is the linger picture and the cell dump.
#[test]
#[ignore = "lavapipe and a CPU path trace, minutes"]
fn the_night_hall_drops_the_old_lamp_within_120_frames() {
    let (lit, camera) = hall();
    let lit_world = World::from_scene(lit.clone());
    let (dark, _) = hall_stage(0.0);
    let dark_world = World::from_scene(dark.world.scene.clone());
    run_case(
        "hall_linger",
        Measure::Linger { warm: 300, dark: 120 },
        &dark_world,
        Some(&lit_world),
        &dark.world.scene,
        Some(&lit),
        &camera,
    );
}

struct EnvSet {
    key: &'static str,
    old: Option<String>,
}

impl EnvSet {
    fn set(key: &'static str, value: Option<&str>) -> Self {
        let old = std::env::var(key).ok();
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
        Self { key, old }
    }
}

impl Drop for EnvSet {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

#[test]
fn schedule_lands_on_the_approved_frames() {
    let linger = acceptance_linger(300, 120);
    let warm = linger.warm.expect("warm cells and picture are a boundary");
    assert_eq!(warm.picture_frame, 300);
    assert_eq!(warm.picture_index, 300, "warm picture is the 300th lit draw");
    assert_eq!(warm.direct_frame, 298);
    assert_eq!(warm.dump_frame, warm.picture_frame);
    assert_eq!(warm.dump_ready_frame, 305);
    assert_eq!(linger.graded.picture_frame, 420);
    assert_eq!(linger.graded.picture_index, 120, "linger picture is the 120th dark draw");
    assert_eq!(linger.graded.direct_frame, 422);
    assert_eq!(linger.graded.dump_frame, linger.graded.picture_frame);
    assert_eq!(linger.graded.dump_ready_frame, 425);
    assert_eq!(linger.lamp_off_frame, Some(301));
    assert_eq!(linger.steps.last().expect("flush").frame, 425);
    assert!(linger.ready.iter().any(|(frame, at)| *frame == 420 && *at == 425));
    assert!(!linger.ready.iter().any(|(frame, at)| *frame == 420 && *at == 424));
    assert!(warm.dump_ready_frame < linger.graded.picture_frame);
    assert_ne!(linger.graded.picture_index, 119);
    assert_ne!(linger.graded.picture_index, 121);
    assert_ne!(linger.graded.picture_index, 122);
    let rejected = rejected_linger_frames(300, 120);
    assert_eq!(rejected.lamp_off, 305);
    assert_eq!(rejected.dump_dark_index, 108);
    assert_eq!(rejected.bounce_dark_index, 122);
    assert_ne!(rejected.dump_dark_index, linger.graded.picture_index);

    let lag = acceptance_lag(300, 300);
    assert_eq!(lag.graded.picture_frame, 600);
    assert_eq!(lag.graded.picture_index, 300);
    assert_eq!(lag.graded.direct_frame, 598);
    assert_eq!(lag.graded.dump_frame, 600);
    assert_eq!(lag.graded.dump_ready_frame, 605);
    assert_eq!(lag.warm, None);
    assert_ne!(lag.graded.picture_index, 299);
    assert_ne!(lag.graded.picture_index, 301);
    assert_ne!(lag.graded.picture_index, 302);
    let rejected_lag = rejected_lag_frames(300, 300);
    assert_eq!(rejected_lag.dump_lit_index, 292);
    assert_eq!(rejected_lag.bounce_lit_index, 302);
    assert_ne!(rejected_lag.bounce_lit_index, lag.graded.picture_index);

    let energy = acceptance_energy(900);
    assert_eq!(energy.graded.picture_frame, 900);
    assert_eq!(energy.graded.picture_index, 900);
    assert_eq!(energy.graded.dump_frame, 900);
    assert_eq!(energy.graded.direct_frame, 898);
    assert_eq!(energy.graded.dump_ready_frame, 905);
    assert!(energy.warm.is_none());
}

#[test]
fn black_direct_is_an_exact_match_and_zero_denominator_cannot_pass() {
    let black = direct_oracle(0.0, 0.0).expect("black/black");
    assert!(matches!(black, DirectOracle::ExactBlack), "black/black became {black:?}");
    assert!(direct_oracle(1.0, 0.0).is_err(), "a lit picture against a black reference is not a ratio");
    let lit = direct_oracle(1.0, 1.0).expect("lit direct");
    assert!(matches!(lit, DirectOracle::Ratio(ratio) if (DIRECT_LOW..=DIRECT_HIGH).contains(&ratio)), "{lit:?}");
    assert!(linger_share(0.0, 0.0).is_err(), "zero/zero must not pass <= {LINGER}");
    assert!(linger_share(1.0, 0.0).is_err(), "a zero lit denominator is not a share");
    let share = linger_share(0.02, 1.0).expect("lit denominator");
    assert!(share <= LINGER, "{share}");
    let old = 0.0_f64 / 0.0_f64.max(1e-12);
    assert_eq!(old, 0.0);
    assert!(old <= LINGER, "the rejected division would pass the linger gate");
    let cells = SlotGrade { live: 0.0, reference: 0.0, n: 32, median: 0.0, under: 0.0, median_lit: 0.0 };
    assert!(slot_share(&cells).is_err(), "zero/zero cells must not pass <= {LINGER}");
}

#[test]
fn a_capped_reference_above_the_noise_target_is_blocked() {
    let reviewer = ReferenceFacts {
        present: true,
        noise: 0.032_146,
        spp: 672,
        seconds: 600.0,
        seconds_budget: 600.0,
        max_spp: 4096,
        noise_target: 0.02,
        hit_count: 160 * 90,
        linear_finite: true,
    };
    let err = reference_verdict(&reviewer).expect_err("noise 0.032146 is not a reference");
    assert!(err.contains("blocked"), "{err}");
    assert!(reference_verdict(&ReferenceFacts { noise: f32::NAN, ..reviewer }).is_err());
    assert!(reference_verdict(&ReferenceFacts { present: false, ..reviewer }).is_err());
    assert!(reference_verdict(&ReferenceFacts { linear_finite: false, ..reviewer }).is_err());
    assert!(reference_verdict(&ReferenceFacts {
        noise: 0.02,
        spp: 800,
        seconds: 12.0,
        ..reviewer
    })
    .is_ok());
    assert!(reference_verdict(&ReferenceFacts {
        noise: 0.03,
        spp: 4096,
        seconds: 10.0,
        ..reviewer
    })
    .expect_err("spp cap without the noise target")
    .contains("blocked"));
}

#[test]
fn acceptance_rejects_measurement_overrides_and_parses_sh_as_a_bool() {
    assert!(!parse_flag(Some("0")).expect("0"));
    assert!(parse_flag(Some("1")).expect("1"));
    assert!(!parse_flag(Some("false")).expect("false"));
    assert!(parse_flag(Some("true")).expect("true"));
    assert!(parse_flag(Some("")).is_err());
    assert!(parse_flag(Some("maybe")).is_err());
    assert!(parse_flag(None).is_err(), "acceptance does not treat a missing knob as a mode");
    assert!(acceptance_refusal(&[]).is_ok());
    for key in [
        "CACHE_ENERGY_WARM",
        "CACHE_ENERGY_FRAMES",
        "CACHE_ENERGY_TRAIL",
        "CACHE_ENERGY_SH",
        "CACHE_ENERGY_PGM",
        "CACHE_ENERGY_CSV",
        "GENOS_LIGHTING_PRESET",
        "GENOS_LIGHTING_DUMP",
        "GENOS_LIGHTING_SURFACE_CACHE_SH_CELLS",
    ] {
        let err = acceptance_refusal(&[(key, "1")]).expect_err(key);
        assert!(!err.contains("non-acceptance"), "{key}: {err}");
    }
    let research = acceptance_refusal(&[("CACHE_ENERGY_RESEARCH", "1")]).expect_err("research");
    assert!(research.contains("non-acceptance"), "{research}");
    let acceptance = Path::new("/tmp/reference-cache");
    let research_dir = Path::new("/tmp/research-non-acceptance");
    assert!(research_plan(0.05, 4096, 600.0, acceptance, research_dir).is_err());
    assert!(research_plan(0.02, 64, 600.0, acceptance, research_dir).is_err());
    assert!(research_plan(0.02, 4096, f32::INFINITY, acceptance, research_dir).is_err());
    assert!(research_plan(0.02, 4096, 600.0, acceptance, acceptance).is_err());
    assert!(research_plan(0.02, 4096, 600.0, acceptance, &acceptance.join("research-non-acceptance")).is_err());
    assert!(research_plan(0.02, 4096, 600.0, acceptance, research_dir).is_ok());
    let (scene, camera) = closed_box();
    let err = research_continue(&scene, &camera, 64, f32::INFINITY, acceptance, research_dir)
        .expect_err("an unbounded research trace is refused before it runs");
    assert!(err.contains("finite"), "{err}");
}

#[test]
fn a_preset_env_clears_a_struct_sh_flag_and_acceptance_refuses_it() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let _preset = EnvSet::set("GENOS_LIGHTING_PRESET", Some("high"));
    let cleared = EnvSet::set("GENOS_LIGHTING_SURFACE_CACHE_SH_CELLS", None);
    let mut settings = LightingSettings::preset(LightingPreset::High);
    settings.surface_cache.sh_cells = true;
    settings.apply_env().expect("apply_env");
    assert!(
        !settings.surface_cache.sh_cells,
        "GENOS_LIGHTING_PRESET replaces the struct and clears sh_cells when the key is unset"
    );
    drop(cleared);
    let _sh_on = EnvSet::set("GENOS_LIGHTING_SURFACE_CACHE_SH_CELLS", Some("1"));
    settings.apply_env().expect("apply_env overlay");
    assert!(settings.surface_cache.sh_cells, "the SH key overlays the preset when it is set");
    assert!(acceptance_refusal(&[("GENOS_LIGHTING_PRESET", "high")]).is_err());
}

#[test]
fn keep_fail_keeps_schedule_and_provenance() {
    let dir = std::env::temp_dir().join(format!("cache-energy-keep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("artifact dir");
    let prior = "{\n  \"verdict\": \"running\",\n  \"name\": \"hall\",\n  \"detail\": \"tracing\",\n  \"warm_picture_frame\": 300, \"warm_dump_ready\": 305, \"graded_picture_frame\": 420, \"graded_picture_index\": 120, \"graded_direct_frame\": 422, \"graded_dump_ready\": 425, \"lamp_off_frame\": 301,\n  \"source_commit\": \"d8142c375dc515d73a041d485c37cb44bc2fe628\",\n  \"cache_key\": \"0123456789abcdef\",\n  \"yaw_bits\": 3196059648,\n  \"eye_bits\": [1095183565, 1070386381, 1101643776],\n  \"files\": \"f420_layout.json\"\n}\n";
    std::fs::write(dir.join("result.json"), prior).expect("prior result");
    let caught = std::panic::catch_unwind(|| keep_fail(&dir, "draw failed: device lost"));
    assert!(
        caught.is_err(),
        "keep_fail aborts after writing the failure"
    );
    let body = std::fs::read_to_string(dir.join("result.json")).expect("result.json");
    assert!(body.contains("\"verdict\": \"fail\""), "{body}");
    assert!(body.contains("draw failed: device lost"), "{body}");
    assert!(!body.contains("\"detail\": \"tracing\""), "{body}");
    for field in [
        "\"name\": \"hall\"",
        "\"warm_picture_frame\": 300",
        "\"warm_dump_ready\": 305",
        "\"graded_picture_frame\": 420",
        "\"graded_picture_index\": 120",
        "\"graded_direct_frame\": 422",
        "\"graded_dump_ready\": 425",
        "\"lamp_off_frame\": 301",
        "\"source_commit\": \"d8142c375dc515d73a041d485c37cb44bc2fe628\"",
        "\"cache_key\": \"0123456789abcdef\"",
        "\"yaw_bits\": 3196059648",
        "\"eye_bits\": [1095183565, 1070386381, 1101643776]",
        "\"files\": \"f420_layout.json\"",
    ] {
        assert!(body.contains(field), "lost {field} in {body}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_later_failure_keeps_the_schedule_the_harness_writes() {
    let dir = std::env::temp_dir().join(format!("cache-energy-sched-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("artifact dir");
    let schedule = acceptance_linger(300, 120);
    std::fs::write(
        dir.join("result.json"),
        measured_json("hall_linger", &schedule, "tracing", "running", ""),
    )
    .expect("prior result");
    let caught = std::panic::catch_unwind(|| keep_fail(&dir, "renderer: device lost"));
    assert!(caught.is_err(), "keep_fail aborts");
    let body = std::fs::read_to_string(dir.join("result.json")).expect("result.json");
    assert!(body.contains("\"verdict\": \"fail\""), "{body}");
    assert!(body.contains("renderer: device lost"), "{body}");
    assert!(body.contains("\"name\": \"hall_linger\""), "{body}");
    assert!(body.contains("\"warm_picture_frame\": 300"), "{body}");
    assert!(body.contains("\"warm_dump_ready\": 305"), "{body}");
    assert!(body.contains("\"graded_picture_frame\": 420"), "{body}");
    assert!(body.contains("\"graded_picture_index\": 120"), "{body}");
    assert!(body.contains("\"graded_direct_frame\": 422"), "{body}");
    assert!(body.contains("\"graded_dump_ready\": 425"), "{body}");
    assert!(body.contains("\"lamp_off_frame\": 301"), "{body}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn precompute_matches_acceptance_setup_except_seconds() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let _seconds = EnvSet::set("CACHE_ENERGY_PRECOMPUTE_SECONDS", Some("2400"));
    let _gpu = EnvSet::set("GENOS_REFERENCE_GPU", None);
    let (scene, camera) = hall();
    let acceptance = acceptance_setup(&scene, &camera, 64);
    assert_eq!(acceptance.seconds.to_bits(), TRACE_SECONDS.to_bits());
    assert_eq!(acceptance.yaw.to_bits(), (-0.3554f32).to_bits());
    assert_eq!(acceptance.pitch.to_bits(), 0.08f32.to_bits());
    assert_eq!(
        acceptance.eye.map(f32::to_bits),
        [14.5f32, 1.7, 23.5].map(f32::to_bits)
    );
    let cli_yaw = genos_stress::building::View::Hall.pose().2;
    assert_ne!(
        acceptance.yaw.to_bits(),
        cli_yaw.to_bits(),
        "harness yaw is not --view hall"
    );
    assert_eq!(acceptance.width, WIDTH);
    assert_eq!(acceptance.height, HEIGHT);
    assert_eq!(acceptance.spp, 32);
    assert_eq!(acceptance.max_spp, MAX_SPP);
    assert_eq!(acceptance.noise_target.to_bits(), NOISE_TARGET.to_bits());
    assert_eq!(acceptance.max_bounces, 64);
    assert!(!acceptance.triangles);
    assert!(!acceptance.specular);
    assert_eq!(acceptance.materials.len(), 0);
    assert!(acceptance_refusal(&[("CACHE_ENERGY_SH", "0")]).is_err());
    assert!(acceptance_refusal(&[("GENOS_LIGHTING_SURFACE_CACHE_SH_CELLS", "0")]).is_err());
    let produced =
        precompute_setup(&scene, &camera, 64, PRECOMPUTE_SECONDS_CAP).expect("precompute setup");
    assert_eq!(produced.seconds.to_bits(), PRECOMPUTE_SECONDS_CAP.to_bits());
    shares_acceptance_trace(&acceptance, &produced).expect("setup drifted");
    let key = genos_debug::reference::cache_key(&acceptance);
    assert_eq!(key, genos_debug::reference::cache_key(&produced), "seconds leaked into the cache key");
    eprintln!(
        "hall lit64 key {key:016x} yaw_bits {} pitch_bits {} eye_bits {:?}",
        acceptance.yaw.to_bits(),
        acceptance.pitch.to_bits(),
        acceptance.eye.map(f32::to_bits)
    );
    let at_cap = precompute_setup(&scene, &camera, 64, 2400.0).expect("2400");
    assert_eq!(at_cap.seconds.to_bits(), 2400.0f32.to_bits());
    assert!(
        precompute_setup(&scene, &camera, 64, 2400.1).is_err(),
        "above 2400"
    );
    assert!(precompute_setup(&scene, &camera, 64, f32::INFINITY).is_err());
    assert!(precompute_setup(&scene, &camera, 64, f32::NAN).is_err());
    assert!(precompute_setup(&scene, &camera, 64, 0.0).is_err());
    assert!(precompute_setup(&scene, &camera, 64, -5.0).is_err());
}

fn ref_names(dir: &Path) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(read) = std::fs::read_dir(dir) {
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let ext = Path::new(&name).extension().and_then(|ext| ext.to_str()).unwrap_or("");
            if ext.eq_ignore_ascii_case("ref") || name.contains(".provenance.") {
                names.push(name);
            }
        }
    }
    names.sort();
    names
}

fn fixture_reference(noise: f32, spp: u32, hit: bool, finite: bool) -> genos_debug::Reference {
    let n = (WIDTH * HEIGHT) as usize;
    let pixel = if finite {
        [0.2_f32, 0.1, 0.05]
    } else {
        [f32::NAN, 0.0, 0.0]
    };
    genos_debug::Reference {
        image: genos_debug::Image {
            width: WIDTH,
            height: HEIGHT,
            rgb: vec![0; n * 3],
        },
        linear: vec![pixel; n],
        hit: vec![hit; n],
        spp,
        seconds: 12.0,
        noise,
        specular: None,
    }
}

fn write_fixture(path: &Path, reference: &genos_debug::Reference) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("isolated dir");
    }
    genos_debug::reference::write_cached(path, reference).expect("fixture ref");
}

#[test]
fn a_bad_precompute_cannot_enter_the_acceptance_cache() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let _gpu = EnvSet::set("GENOS_REFERENCE_GPU", None);
    let (scene, camera) = hall();
    let acceptance = acceptance_setup(&scene, &camera, 64);
    let mut long = acceptance.clone();
    long.seconds = PRECOMPUTE_SECONDS_CAP;
    let commit = "d8142c375dc515d73a041d485c37cb44bc2fe628";
    let root = std::env::temp_dir().join(format!("cache-energy-promote-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mut case_index = 0u32;
    let mut reject = |reference: &genos_debug::Reference,
                      produced: &genos_debug::reference::RefSetup,
                      commit: &str,
                      bytes: Option<&[u8]>| {
        case_index += 1;
        let dir = root.join(case_index.to_string());
        let isolated = dir.join("isolated.ref");
        std::fs::create_dir_all(&dir).expect("case dir");
        match bytes {
            Some(bytes) => std::fs::write(&isolated, bytes).expect("bad file"),
            None => write_fixture(&isolated, reference),
        }
        let acceptance_dir = dir.join("acceptance");
        let result = promote_isolated(&Promotion {
            reference,
            produced,
            acceptance: &acceptance,
            commit,
            expected_commit: "d8142c375dc515d73a041d485c37cb44bc2fe628",
            isolated: &isolated,
            acceptance_dir: &acceptance_dir,
        });
        assert!(result.is_err(), "case {case_index} promoted: {result:?}");
        assert!(
            ref_names(&acceptance_dir).is_empty(),
            "case {case_index} wrote {:?}",
            ref_names(&acceptance_dir)
        );
    };
    let noisy = fixture_reference(0.032_146, 672, true, true);
    reject(&noisy, &long, commit, None);
    let capped = fixture_reference(0.03, 4096, true, true);
    reject(&capped, &long, commit, None);
    let nan_noise = fixture_reference(f32::NAN, 128, true, true);
    reject(&nan_noise, &long, commit, None);
    let nan_pixel = fixture_reference(0.01, 128, true, false);
    reject(&nan_pixel, &long, commit, None);
    let empty = fixture_reference(0.01, 128, false, true);
    reject(&empty, &long, commit, None);
    let good = fixture_reference(0.01, 128, true, true);
    reject(&good, &long, commit, Some(&[]));
    let mut yaw = long.clone();
    yaw.yaw = -0.2;
    reject(&good, &yaw, commit, None);
    reject(
        &good,
        &long,
        "0000000000000000000000000000000000000000",
        None,
    );
    let mut over = long.clone();
    over.seconds = 2400.1;
    reject(&good, &over, commit, None);
    let mut triangles = long.clone();
    triangles.triangles = true;
    reject(&good, &triangles, commit, None);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_converged_precompute_promotes_the_same_key() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let _gpu = EnvSet::set("GENOS_REFERENCE_GPU", None);
    let (scene, camera) = hall();
    let acceptance = acceptance_setup(&scene, &camera, 64);
    let mut produced = acceptance.clone();
    produced.seconds = PRECOMPUTE_SECONDS_CAP;
    let commit = "d8142c375dc515d73a041d485c37cb44bc2fe628";
    let good = fixture_reference(0.02, 4096, true, true);
    let root = std::env::temp_dir().join(format!("cache-energy-promote-ok-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let isolated = root.join("isolated.ref");
    write_fixture(&isolated, &good);
    let acceptance_dir = root.join("acceptance");
    let key = genos_debug::reference::cache_key(&acceptance);
    let dest = promote_isolated(&Promotion {
        reference: &good,
        produced: &produced,
        acceptance: &acceptance,
        commit,
        expected_commit: commit,
        isolated: &isolated,
        acceptance_dir: &acceptance_dir,
    })
    .expect("converged final pass promotes");
    let expected = acceptance_dir.join(format!("{key:016x}.ref"));
    assert_eq!(dest, expected);
    let loaded = genos_debug::reference::read_cached(&dest, WIDTH, HEIGHT).expect("promoted file");
    assert_eq!(loaded.noise.to_bits(), good.noise.to_bits());
    assert_eq!(loaded.spp, 4096);
    assert!(loaded.hit.iter().any(|hit| *hit));
    let note = std::fs::read_to_string(acceptance_dir.join(format!("{key:016x}.provenance.json")))
        .expect("provenance");
    assert!(note.contains(commit), "{note}");
    assert!(note.contains(&format!("{key:016x}")), "{note}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn precompute_seconds_are_bounded_and_an_untrusted_file_is_refused() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let _clear = EnvSet::set("CACHE_ENERGY_PRECOMPUTE_SECONDS", None);
    assert!(
        precompute_seconds_from_env().is_err(),
        "a missing bound must not trace"
    );
    let _over = EnvSet::set("CACHE_ENERGY_PRECOMPUTE_SECONDS", Some("2401"));
    assert!(
        precompute_seconds_from_env().is_err(),
        "2401 is above the bound"
    );
    let _at_cap = EnvSet::set("CACHE_ENERGY_PRECOMPUTE_SECONDS", Some("2400"));
    assert_eq!(
        precompute_seconds_from_env().expect("2400").to_bits(),
        2400.0f32.to_bits()
    );
    let (scene, camera) = closed_box();
    let acceptance = acceptance_setup(&scene, &camera, 64);
    assert_eq!(acceptance.seconds.to_bits(), TRACE_SECONDS.to_bits());
    let mut produced = acceptance.clone();
    produced.seconds = 2400.0;
    let key = genos_debug::reference::cache_key(&acceptance);
    let commit = "d8142c375dc515d73a041d485c37cb44bc2fe628";
    let picture = fixture_reference(0.01, 32, true, true);
    let dir = std::env::temp_dir().join(format!("cache-energy-trust-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join("cached.ref");
    std::fs::write(&path, b"old").expect("old file");
    let missing_note =
        isolated_trust(&path, "", commit, &produced, key).expect_err("no provenance");
    assert!(missing_note.contains("untrusted"), "{missing_note}");
    let other = promotion_note(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        key,
        &produced,
        &picture,
        false,
    );
    let wrong_commit =
        isolated_trust(&path, &other, commit, &produced, key).expect_err("other commit");
    assert!(wrong_commit.contains("untrusted"), "{wrong_commit}");
    let note = promotion_note(commit, key, &produced, &picture, false);
    isolated_trust(&path, &note, commit, &produced, key).expect("same commit and pose");
    isolated_trust(&dir.join("absent.ref"), "", commit, &produced, key).expect("nothing to trust");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "CPU path trace of the lit hall, up to 2400s, not acceptance"]
fn precompute_hall_lit64() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let seconds = precompute_seconds_from_env().expect("CACHE_ENERGY_PRECOMPUTE_SECONDS");
    precompute_hall_lit64_offline(seconds).expect("precompute");
}

fn dir_files(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut rows = Vec::new();
    if let Ok(read) = std::fs::read_dir(dir) {
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let bytes = std::fs::read(entry.path()).unwrap_or_default();
            rows.push((name, bytes));
        }
    }
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    rows
}

#[test]
fn floor_sampling_budget_is_not_the_acceptance_identity() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let _gpu = EnvSet::set("GENOS_REFERENCE_GPU", None);
    let (scene, camera) = open_floor();
    let acceptance = acceptance_setup(&scene, &camera, 64);
    assert_eq!(acceptance.max_spp, MAX_SPP);
    assert_eq!(acceptance.seconds.to_bits(), TRACE_SECONDS.to_bits());
    assert_eq!(acceptance.noise_target.to_bits(), NOISE_TARGET.to_bits());
    assert_eq!(acceptance.spp, 32);
    assert_eq!(acceptance.max_bounces, 64);
    assert!(!acceptance.triangles);
    assert!(!acceptance.specular);
    assert_eq!(acceptance.materials.len(), 0);
    assert_eq!(acceptance.width, WIDTH);
    assert_eq!(acceptance.height, HEIGHT);
    assert_eq!(
        acceptance.eye.map(f32::to_bits),
        [0.0f32, 1.7, 6.0].map(f32::to_bits)
    );
    assert_eq!(acceptance.yaw.to_bits(), 0.0f32.to_bits());
    assert_eq!(acceptance.pitch.to_bits(), (-0.12f32).to_bits());
    let produced = floor_precompute_setup(
        &scene,
        &camera,
        64,
        OFFLINE_MAX_SPP,
        PRECOMPUTE_SECONDS_CAP,
    )
    .expect("explicit floor budget");
    assert_eq!(produced.max_spp, OFFLINE_MAX_SPP);
    assert_eq!(produced.seconds.to_bits(), PRECOMPUTE_SECONDS_CAP.to_bits());
    assert_eq!(produced.spp, acceptance.spp);
    assert_eq!(produced.noise_target.to_bits(), acceptance.noise_target.to_bits());
    assert_eq!(produced.max_bounces, acceptance.max_bounces);
    assert_eq!(produced.eye.map(f32::to_bits), acceptance.eye.map(f32::to_bits));
    shares_acceptance_identity(&acceptance, &produced).expect("budget stays off the identity");
    let budget = shares_acceptance_trace(&acceptance, &produced)
        .expect_err("max spp is a sampling budget, not a second identity");
    assert!(budget.contains("sampling budget"), "{budget}");
    let key = genos_debug::reference::cache_key(&acceptance);
    assert_eq!(
        key,
        genos_debug::reference::cache_key(&produced),
        "sampling budget leaked into the cache key"
    );
    let direct = genos_debug::reference::cache_key(&acceptance_setup(&scene, &camera, 0));
    assert_eq!(format!("{direct:016x}"), "e1d4a8f081cd9e2b");
    assert_eq!(format!("{key:016x}"), "dfe032d1638e605b");
    eprintln!("open floor b0 key {direct:016x} b64 key {key:016x}");
}

#[test]
fn a_floor_budget_does_not_ignore_pose_bounces_or_estimator() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let _gpu = EnvSet::set("GENOS_REFERENCE_GPU", None);
    let (scene, camera) = open_floor();
    let acceptance = acceptance_setup(&scene, &camera, 64);
    let produced = floor_precompute_setup(&scene, &camera, 64, OFFLINE_MAX_SPP, 10.0)
        .expect("explicit floor budget");
    let mut yaw = produced.clone();
    yaw.yaw = -0.2;
    assert!(shares_acceptance_identity(&acceptance, &yaw)
        .expect_err("yaw")
        .contains("yaw"));
    let mut eye = produced.clone();
    eye.eye[0] = 1.0;
    assert!(shares_acceptance_identity(&acceptance, &eye)
        .expect_err("eye")
        .contains("eye"));
    let mut pitch = produced.clone();
    pitch.pitch = 0.2;
    assert!(shares_acceptance_identity(&acceptance, &pitch)
        .expect_err("pitch")
        .contains("pitch"));
    let mut size = produced.clone();
    size.width = 80;
    assert!(shares_acceptance_identity(&acceptance, &size)
        .expect_err("size")
        .contains("size"));
    let mut bounces = produced.clone();
    bounces.max_bounces = 3;
    assert!(shares_acceptance_identity(&acceptance, &bounces)
        .expect_err("bounces")
        .contains("estimator"));
    let mut noise = produced.clone();
    noise.noise_target = 0.05;
    assert!(shares_acceptance_identity(&acceptance, &noise)
        .expect_err("noise target")
        .contains("estimator"));
    let mut batch = produced.clone();
    batch.spp = 8;
    assert!(shares_acceptance_identity(&acceptance, &batch)
        .expect_err("batch")
        .contains("estimator"));
    let mut triangles = produced.clone();
    triangles.triangles = true;
    assert!(shares_acceptance_identity(&acceptance, &triangles)
        .expect_err("triangles")
        .contains("shape"));
    let mut lobe = produced.clone();
    lobe.specular = true;
    assert!(shares_acceptance_identity(&acceptance, &lobe).is_err());
    let mut paint = produced.clone();
    paint.materials.push(Some(genos_debug::reference::SolidMaterial {
        roughness: 0.4,
        metallic: 0.0,
    }));
    assert!(shares_acceptance_identity(&acceptance, &paint).is_err());
    let (other_scene, other_camera) = closed_box();
    let other = floor_precompute_setup(&other_scene, &other_camera, 64, OFFLINE_MAX_SPP, 10.0)
        .expect("closed box still has an acceptance setup");
    assert!(shares_acceptance_identity(&acceptance, &other)
        .expect_err("scene")
        .contains("scene"));

    let (hall_scene, hall_camera) = hall();
    let hall_acceptance = acceptance_setup(&hall_scene, &hall_camera, 64);
    let hall_produced =
        precompute_setup(&hall_scene, &hall_camera, 64, PRECOMPUTE_SECONDS_CAP).expect("hall");
    assert_eq!(hall_produced.max_spp, MAX_SPP);
    assert_ne!(hall_produced.seconds.to_bits(), hall_acceptance.seconds.to_bits());
    shares_acceptance_trace(&hall_acceptance, &hall_produced).expect("hall changes seconds only");
    shares_acceptance_identity(&hall_acceptance, &hall_produced).expect("hall identity");
}

#[test]
fn floor_offline_budget_rejects_out_of_range_and_overflow() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let (scene, camera) = open_floor();
    for spp in [0, OFFLINE_MAX_SPP + 1, u32::MAX] {
        assert!(
            floor_precompute_setup(&scene, &camera, 64, spp, 10.0).is_err(),
            "spp {spp}"
        );
    }
    for seconds in [0.0, -1.0, 2400.1, f32::INFINITY, f32::NAN] {
        assert!(
            floor_precompute_setup(&scene, &camera, 64, OFFLINE_MAX_SPP, seconds).is_err(),
            "seconds {seconds}"
        );
    }
    let low = floor_precompute_setup(&scene, &camera, 64, 1, PRECOMPUTE_SECONDS_CAP)
        .expect("1 spp is inside the ceiling");
    assert_eq!(low.max_spp, 1);
    assert_eq!(low.noise_target.to_bits(), NOISE_TARGET.to_bits());
    assert_eq!(low.seconds.to_bits(), PRECOMPUTE_SECONDS_CAP.to_bits());
    let _clear_spp = EnvSet::set("CACHE_ENERGY_FLOOR_MAX_SPP", None);
    let _clear_seconds = EnvSet::set("CACHE_ENERGY_FLOOR_SECONDS", None);
    assert!(floor_budgets_from_env().is_err(), "a missing budget must not trace");
    for bad in [
        "262145",
        "1048576",
        "4294967296",
        "99999999999999999999",
        "-1",
        "+262144",
        "1.5",
        "0",
        "",
        "nan",
    ] {
        let _spp = EnvSet::set("CACHE_ENERGY_FLOOR_MAX_SPP", Some(bad));
        let _seconds = EnvSet::set("CACHE_ENERGY_FLOOR_SECONDS", Some("2400"));
        let err = floor_budgets_from_env().expect_err(bad);
        assert!(!err.contains("tracing"), "{bad}: {err}");
    }
    for bad in ["2401", "0", "-5", "inf", "nan", "1e99", ""] {
        let _spp = EnvSet::set("CACHE_ENERGY_FLOOR_MAX_SPP", Some("262144"));
        let _seconds = EnvSet::set("CACHE_ENERGY_FLOOR_SECONDS", Some(bad));
        assert!(floor_budgets_from_env().is_err(), "{bad}");
    }
    let _spp = EnvSet::set("CACHE_ENERGY_FLOOR_MAX_SPP", Some("262144"));
    let _seconds = EnvSet::set("CACHE_ENERGY_FLOOR_SECONDS", Some("2400"));
    let (spp, seconds) = floor_budgets_from_env().expect("ceiling");
    assert_eq!(spp, OFFLINE_MAX_SPP);
    assert_eq!(seconds.to_bits(), PRECOMPUTE_SECONDS_CAP.to_bits());
    let acceptance = acceptance_setup(&scene, &camera, 64);
    assert_eq!(acceptance.max_spp, MAX_SPP);
    assert_eq!(acceptance.seconds.to_bits(), TRACE_SECONDS.to_bits());
    assert_eq!(acceptance.noise_target.to_bits(), NOISE_TARGET.to_bits());
    assert!(acceptance_refusal(&[
        ("CACHE_ENERGY_FLOOR_MAX_SPP", "262144"),
        ("CACHE_ENERGY_FLOOR_SECONDS", "2400"),
    ])
    .is_ok());
}

#[test]
fn a_converged_floor_above_4096_spp_promotes_only_inside_the_offline_cap() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let _gpu = EnvSet::set("GENOS_REFERENCE_GPU", None);
    let (scene, camera) = open_floor();
    let acceptance = acceptance_setup(&scene, &camera, 64);
    let mut produced = acceptance.clone();
    produced.max_spp = 8_192;
    produced.seconds = 30.0;
    let commit = "d8142c375dc515d73a041d485c37cb44bc2fe628";
    let good = fixture_reference(0.02, 8_192, true, true);
    let root = std::env::temp_dir().join(format!("cache-energy-floor-ok-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let isolated = root.join("isolated.ref");
    write_fixture(&isolated, &good);
    let acceptance_dir = root.join("acceptance");
    let key = genos_debug::reference::cache_key(&acceptance);
    let dest = promote_floor_isolated(&Promotion {
        reference: &good,
        produced: &produced,
        acceptance: &acceptance,
        commit,
        expected_commit: commit,
        isolated: &isolated,
        acceptance_dir: &acceptance_dir,
    })
    .expect("8192 spp inside the explicit cap promotes");
    assert_eq!(dest, acceptance_dir.join(format!("{key:016x}.ref")));
    let loaded = genos_debug::reference::read_cached(&dest, WIDTH, HEIGHT).expect("promoted");
    assert_eq!(loaded.spp, 8_192);
    assert_eq!(loaded.noise.to_bits(), good.noise.to_bits());
    let note = std::fs::read_to_string(acceptance_dir.join(format!("{key:016x}.provenance.json")))
        .expect("provenance");
    assert!(note.contains("\"acceptance_max_spp\": 4096"), "{note}");
    assert!(note.contains("\"offline_max_spp\": 8192"), "{note}");
    assert!(note.contains("\"offline_spp_ceiling\": 262144"), "{note}");
    assert!(note.contains("\"spp\": 8192"), "{note}");
    assert!(note.contains("\"acceptance_seconds\": 600"), "{note}");
    assert!(note.contains(commit), "{note}");
    assert!(note.contains("\"scene\": \"floor\""), "{note}");

    let mut at_acceptance_cap = produced.clone();
    at_acceptance_cap.max_spp = MAX_SPP;
    let above = fixture_reference(0.01, 4_097, true, true);
    let blocked = root.join("blocked");
    std::fs::create_dir_all(&blocked).expect("blocked");
    let kept = blocked.join(format!("{key:016x}.ref"));
    std::fs::write(&kept, b"already-valid").expect("seed");
    let before = dir_files(&blocked);
    let isolated_above = root.join("above.ref");
    write_fixture(&isolated_above, &above);
    let err = promote_floor_isolated(&Promotion {
        reference: &above,
        produced: &at_acceptance_cap,
        acceptance: &acceptance,
        commit,
        expected_commit: commit,
        isolated: &isolated_above,
        acceptance_dir: &blocked,
    })
    .expect_err("spp above 4096 needs an explicit cap above 4096");
    assert!(err.contains("blocked"), "{err}");
    assert_eq!(dir_files(&blocked), before);

    let mut over_ceiling = produced.clone();
    over_ceiling.max_spp = OFFLINE_MAX_SPP + 1;
    let err = promote_floor_isolated(&Promotion {
        reference: &good,
        produced: &over_ceiling,
        acceptance: &acceptance,
        commit,
        expected_commit: commit,
        isolated: &isolated,
        acceptance_dir: &blocked,
    })
    .expect_err("cap above 262144");
    assert!(err.contains("262144") || err.contains("outside"), "{err}");
    assert_eq!(dir_files(&blocked), before);

    let (hall_scene, hall_camera) = hall();
    let hall_acceptance = acceptance_setup(&hall_scene, &hall_camera, 64);
    let mut hall_produced = hall_acceptance.clone();
    hall_produced.seconds = PRECOMPUTE_SECONDS_CAP;
    let hall_dir = root.join("hall");
    let hall_isolated = root.join("hall.ref");
    let hall_high = fixture_reference(0.01, 8_192, true, true);
    write_fixture(&hall_isolated, &hall_high);
    let hall_err = promote_isolated(&Promotion {
        reference: &hall_high,
        produced: &hall_produced,
        acceptance: &hall_acceptance,
        commit,
        expected_commit: commit,
        isolated: &hall_isolated,
        acceptance_dir: &hall_dir,
    })
    .expect_err("hall promotion still stops at 4096 spp");
    assert!(hall_err.contains("blocked"), "{hall_err}");
    assert!(dir_files(&hall_dir).is_empty(), "{:?}", dir_files(&hall_dir));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_noisy_floor_at_the_offline_cap_does_not_enter_the_cache() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let _gpu = EnvSet::set("GENOS_REFERENCE_GPU", None);
    let (scene, camera) = open_floor();
    let acceptance = acceptance_setup(&scene, &camera, 64);
    let mut produced = acceptance.clone();
    produced.max_spp = OFFLINE_MAX_SPP;
    produced.seconds = PRECOMPUTE_SECONDS_CAP;
    let noisy = fixture_reference(0.036_648_18, OFFLINE_MAX_SPP, true, true);
    let commit = "d8142c375dc515d73a041d485c37cb44bc2fe628";
    let root = std::env::temp_dir().join(format!("cache-energy-floor-noisy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let isolated = root.join("isolated.ref");
    write_fixture(&isolated, &noisy);
    let acceptance_dir = root.join("acceptance");
    std::fs::create_dir_all(&acceptance_dir).expect("cache");
    let key = genos_debug::reference::cache_key(&acceptance);
    std::fs::write(acceptance_dir.join(format!("{key:016x}.ref")), b"valid-noisy-4096").expect("seed");
    std::fs::write(acceptance_dir.join("hall-or-box.ref"), b"leave-me").expect("other");
    let before = dir_files(&acceptance_dir);
    let err = promote_floor_isolated(&Promotion {
        reference: &noisy,
        produced: &produced,
        acceptance: &acceptance,
        commit,
        expected_commit: commit,
        isolated: &isolated,
        acceptance_dir: &acceptance_dir,
    })
    .expect_err("noise above 0.02 at the offline cap stays blocked");
    assert!(err.contains("blocked"), "{err}");
    assert!(err.contains("0.02") || err.contains("0.020"), "{err}");
    assert_eq!(dir_files(&acceptance_dir), before);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_bad_floor_precompute_preserves_the_acceptance_cache() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let _gpu = EnvSet::set("GENOS_REFERENCE_GPU", None);
    let (scene, camera) = open_floor();
    let acceptance = acceptance_setup(&scene, &camera, 64);
    let mut produced = acceptance.clone();
    produced.max_spp = OFFLINE_MAX_SPP;
    produced.seconds = 12.0;
    let commit = "d8142c375dc515d73a041d485c37cb44bc2fe628";
    let good = fixture_reference(0.01, 5_000, true, true);
    let root = std::env::temp_dir().join(format!("cache-energy-floor-bad-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let acceptance_dir = root.join("acceptance");
    std::fs::create_dir_all(&acceptance_dir).expect("cache");
    let key = genos_debug::reference::cache_key(&acceptance);
    std::fs::write(acceptance_dir.join(format!("{key:016x}.ref")), b"valid-cache").expect("seed");
    std::fs::write(acceptance_dir.join("box.ref"), b"box-cache").expect("other");
    let before = dir_files(&acceptance_dir);
    let mut case_index = 0u32;
    let mut reject = |reference: &genos_debug::Reference,
                      produced: &genos_debug::reference::RefSetup,
                      commit: &str,
                      bytes: Option<&[u8]>| {
        case_index += 1;
        let isolated = root.join(format!("isolated-{case_index}.ref"));
        match bytes {
            Some(bytes) => std::fs::write(&isolated, bytes).expect("bad file"),
            None => write_fixture(&isolated, reference),
        }
        let result = promote_floor_isolated(&Promotion {
            reference,
            produced,
            acceptance: &acceptance,
            commit,
            expected_commit: "d8142c375dc515d73a041d485c37cb44bc2fe628",
            isolated: &isolated,
            acceptance_dir: &acceptance_dir,
        });
        assert!(result.is_err(), "case {case_index} promoted: {result:?}");
        assert_eq!(
            dir_files(&acceptance_dir),
            before,
            "case {case_index} changed the cache"
        );
    };
    reject(&fixture_reference(f32::NAN, 128, true, true), &produced, commit, None);
    reject(&fixture_reference(0.01, 128, true, false), &produced, commit, None);
    reject(&fixture_reference(0.01, 128, false, true), &produced, commit, None);
    reject(&good, &produced, commit, Some(&[]));
    let mut yaw = produced.clone();
    yaw.yaw = 0.4;
    reject(&good, &yaw, commit, None);
    let mut bounces = produced.clone();
    bounces.max_bounces = 2;
    reject(&good, &bounces, commit, None);
    let mut batch = produced.clone();
    batch.spp = 16;
    reject(&good, &batch, commit, None);
    let mut noise = produced.clone();
    noise.noise_target = 0.2;
    reject(&good, &noise, commit, None);
    reject(
        &good,
        &produced,
        "0000000000000000000000000000000000000000",
        None,
    );
    let mut seconds = produced.clone();
    seconds.seconds = 2400.1;
    reject(&good, &seconds, commit, None);
    let mut triangles = produced.clone();
    triangles.triangles = true;
    reject(&good, &triangles, commit, None);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn floor_provenance_is_metadata_and_an_untrusted_file_is_refused() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let (scene, camera) = open_floor();
    let acceptance = acceptance_setup(&scene, &camera, 64);
    let mut produced = acceptance.clone();
    produced.max_spp = OFFLINE_MAX_SPP;
    produced.seconds = 20.0;
    let commit = "d8142c375dc515d73a041d485c37cb44bc2fe628";
    let picture = fixture_reference(0.01, 64, true, true);
    let key = genos_debug::reference::cache_key(&acceptance);
    let dir = std::env::temp_dir().join(format!("cache-energy-floor-trust-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join("cached.ref");
    std::fs::write(&path, b"old").expect("old file");
    let missing = floor_isolated_trust(&path, "", commit, &produced, key).expect_err("no note");
    assert!(missing.contains("untrusted"), "{missing}");
    let hall_note = promotion_note(commit, key, &produced, &picture, false);
    let hallish = floor_isolated_trust(&path, &hall_note, commit, &produced, key)
        .expect_err("hall metadata is not a floor trace");
    assert!(hallish.contains("untrusted"), "{hallish}");
    let other = floor_promotion_note(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        key,
        &produced,
        &picture,
        false,
        Some(picture.seconds),
    );
    let wrong = floor_isolated_trust(&path, &other, commit, &produced, key).expect_err("commit");
    assert!(wrong.contains("untrusted"), "{wrong}");
    let note = floor_promotion_note(commit, key, &produced, &picture, false, Some(picture.seconds));
    floor_isolated_trust(&path, &note, commit, &produced, key).expect("matching metadata");
    let mut other_budget = produced.clone();
    other_budget.max_spp = 8_192;
    let drifted = floor_isolated_trust(&path, &note, commit, &other_budget, key)
        .expect_err("a different spp budget is a different trace");
    assert!(drifted.contains("untrusted"), "{drifted}");
    floor_isolated_trust(&dir.join("absent.ref"), "", commit, &produced, key)
        .expect("nothing to trust");
    assert!(
        !note.contains("sha256") && !note.contains("signature"),
        "provenance stays metadata: {note}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn picture_bounce_and_direct_ratios_are_exact() {
    let n = (WIDTH * HEIGHT) as usize;
    let mut full_hit = vec![false; n];
    let mut direct_hit = vec![false; n];
    let mut full_linear = vec![[0.0f32; 3]; n];
    let mut direct_linear = vec![[0.0f32; 3]; n];
    full_hit[0] = true;
    full_hit[1] = true;
    full_hit[3] = true;
    direct_hit[0] = true;
    direct_hit[1] = true;
    direct_hit[3] = true;
    full_linear[0] = [1.0, 1.0, 1.0];
    direct_linear[1] = [1.0, 1.0, 1.0];
    let mut live_bounce = vec![None; n];
    let mut live_direct = vec![None; n];
    live_bounce[0] = Some(2.0);
    live_bounce[1] = Some(7.0);
    live_bounce[2] = Some(100.0);
    live_direct[0] = Some(4.0);
    live_direct[1] = Some(1.0);
    live_direct[2] = Some(100.0);
    let image = |linear: Vec<[f32; 3]>, hit: Vec<bool>| genos_debug::Reference {
        image: genos_debug::Image {
            width: WIDTH,
            height: HEIGHT,
            rgb: vec![0; n * 3],
        },
        linear,
        hit,
        spp: 32,
        seconds: 1.0,
        noise: 0.0,
        specular: None,
    };
    let direct = image(direct_linear, direct_hit);
    let full = image(full_linear, full_hit);
    let picture = picture_sums(&live_bounce, &live_direct, &direct, &full, &direct)
        .expect("picture sums");
    let white = luma([1.0, 1.0, 1.0]);
    assert_eq!(picture.bounce_n, 2, "a miss and a missing sample stay out");
    assert_eq!(picture.direct_n, 2);
    assert_eq!(picture.ref_bounce, white, "a darker full pixel clamps at zero");
    assert_eq!(picture.live_bounce, 9.0);
    assert_eq!(picture.ref_direct, white);
    assert_eq!(picture.live_direct, 5.0);
    let bounce = linger_share(picture.live_bounce, picture.ref_bounce).expect("bounce ratio");
    assert_eq!(bounce, 9.0 / white);
    let direct_ratio = direct_oracle(picture.live_direct, picture.ref_direct).expect("direct");
    assert_eq!(direct_ratio, DirectOracle::Ratio(5.0 / white));
}

#[test]
fn ordinary_acceptance_stays_at_4096_and_accepts_a_converged_higher_spp() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let _spp = EnvSet::set("CACHE_ENERGY_FLOOR_MAX_SPP", Some("262144"));
    let _seconds = EnvSet::set("CACHE_ENERGY_FLOOR_SECONDS", Some("2400"));
    let _hall = EnvSet::set("CACHE_ENERGY_PRECOMPUTE_SECONDS", Some("2400"));
    let (scene, camera) = open_floor();
    let acceptance = acceptance_setup(&scene, &camera, 64);
    assert_eq!(acceptance.max_spp, MAX_SPP);
    assert_eq!(acceptance.seconds.to_bits(), TRACE_SECONDS.to_bits());
    assert_eq!(acceptance.noise_target.to_bits(), NOISE_TARGET.to_bits());
    assert_eq!(acceptance.spp, 32);
    let facts = facts_of(&fixture_reference(0.01, OFFLINE_MAX_SPP, true, true), TRACE_SECONDS);
    assert_eq!(facts.max_spp, MAX_SPP);
    assert!(reference_verdict(&facts).is_ok(), "a converged file above 4096 still passes");
    let noisy = ReferenceFacts {
        noise: 0.036_648_18,
        spp: MAX_SPP,
        seconds: 6.282,
        ..facts
    };
    assert!(reference_verdict(&noisy).expect_err("noisy 4096").contains("blocked"));
    let noisy_cap = ReferenceFacts {
        noise: 0.03,
        spp: OFFLINE_MAX_SPP,
        seconds: 20.0,
        ..facts
    };
    assert!(reference_verdict(&noisy_cap).expect_err("noisy offline cap").contains("blocked"));
    assert!(acceptance_refusal(&[("CACHE_ENERGY_SH", "1")]).is_err());
}

#[test]
fn a_decimal_prefix_of_the_offline_ceiling_is_refused_and_the_isolated_pair_stays() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let (scene, camera) = open_floor();
    let acceptance = acceptance_setup(&scene, &camera, 64);
    let mut stored = acceptance.clone();
    stored.max_spp = OFFLINE_MAX_SPP;
    stored.seconds = 24.9;
    let commit = "5a22737fc92f6b177083fb77995410f65281bedc";
    let picture = fixture_reference(0.026_066_182, OFFLINE_MAX_SPP, true, true);
    let key = genos_debug::reference::cache_key(&acceptance);
    let root = std::env::temp_dir().join(format!(
        "cache-energy-floor-prefix-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("dir");
    let path = root.join(format!("{key:016x}.ref"));
    write_fixture(&path, &picture);
    let note_path = root.join("provenance.json");
    let note = floor_promotion_note(commit, key, &stored, &picture, false, Some(picture.seconds));
    std::fs::write(&note_path, &note).expect("note");
    let before_ref = std::fs::read(&path).expect("ref bytes");
    let before_note = std::fs::read(&note_path).expect("note bytes");
    for cap in [1_u32, 16, 163, 1638, 2, 26, 262, 2621, 26214, 16_384] {
        let mut requested = stored.clone();
        requested.max_spp = cap;
        let err = floor_isolated_trust(&path, &note, commit, &requested, key)
            .expect_err("a decimal prefix or the previous 16384 budget is a different integer");
        assert!(err.contains("untrusted"), "cap {cap}: {err}");
        assert_eq!(std::fs::read(&path).expect("ref"), before_ref, "cap {cap}");
        assert_eq!(
            std::fs::read(&note_path).expect("note"),
            before_note,
            "cap {cap} rewrote the sidecar"
        );
    }
    floor_isolated_trust(&path, &note, commit, &stored, key).expect("262144 matches 262144");
    assert_eq!(std::fs::read(&path).expect("ref"), before_ref);
    assert_eq!(std::fs::read(&note_path).expect("note"), before_note);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn negative_noise_is_outside_the_band_for_hall_and_floor() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let _gpu = EnvSet::set("GENOS_REFERENCE_GPU", None);
    let commit = "d8142c375dc515d73a041d485c37cb44bc2fe628";
    let root = std::env::temp_dir().join(format!(
        "cache-energy-negative-noise-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let negative = fixture_reference(-0.01, 128, true, true);
    let (scene, camera) = open_floor();
    let acceptance = acceptance_setup(&scene, &camera, 64);
    let mut produced = acceptance.clone();
    produced.max_spp = 8_192;
    produced.seconds = 30.0;
    let isolated = root.join("isolated.ref");
    write_fixture(&isolated, &negative);
    let acceptance_dir = root.join("floor-cache");
    std::fs::create_dir_all(&acceptance_dir).expect("cache");
    let key = genos_debug::reference::cache_key(&acceptance);
    std::fs::write(acceptance_dir.join(format!("{key:016x}.ref")), b"kept-ref").expect("seed");
    std::fs::write(
        acceptance_dir.join(format!("{key:016x}.provenance.json")),
        b"kept-note",
    )
    .expect("seed");
    let before = dir_files(&acceptance_dir);
    let err = promote_floor_isolated(&Promotion {
        reference: &negative,
        produced: &produced,
        acceptance: &acceptance,
        commit,
        expected_commit: commit,
        isolated: &isolated,
        acceptance_dir: &acceptance_dir,
    })
    .expect_err("negative floor noise");
    assert!(err.contains("blocked"), "{err}");
    assert_eq!(dir_files(&acceptance_dir), before, "{err}");
    let zero = fixture_reference(0.0, 128, true, true);
    let zero_path = root.join("zero.ref");
    write_fixture(&zero_path, &zero);
    promote_floor_isolated(&Promotion {
        reference: &zero,
        produced: &produced,
        acceptance: &acceptance,
        commit,
        expected_commit: commit,
        isolated: &zero_path,
        acceptance_dir: &root.join("zero-cache"),
    })
    .expect("noise 0 stays inside 0..=0.02");
    let (hall_scene, hall_camera) = hall();
    let hall_acceptance = acceptance_setup(&hall_scene, &hall_camera, 64);
    let mut hall_produced = hall_acceptance.clone();
    hall_produced.seconds = 30.0;
    let hall_dir = root.join("hall-cache");
    std::fs::create_dir_all(&hall_dir).expect("hall");
    let hall_key = genos_debug::reference::cache_key(&hall_acceptance);
    std::fs::write(hall_dir.join(format!("{hall_key:016x}.ref")), b"hall-ref").expect("seed");
    std::fs::write(
        hall_dir.join(format!("{hall_key:016x}.provenance.json")),
        b"hall-note",
    )
    .expect("seed");
    let hall_before = dir_files(&hall_dir);
    let hall_isolated = root.join("hall.ref");
    write_fixture(&hall_isolated, &negative);
    let hall_err = promote_isolated(&Promotion {
        reference: &negative,
        produced: &hall_produced,
        acceptance: &hall_acceptance,
        commit,
        expected_commit: commit,
        isolated: &hall_isolated,
        acceptance_dir: &hall_dir,
    })
    .expect_err("negative hall noise");
    assert!(hall_err.contains("blocked"), "{hall_err}");
    assert_eq!(dir_files(&hall_dir), hall_before, "{hall_err}");
    let facts = facts_of(&negative, TRACE_SECONDS);
    assert!(reference_verdict(&facts)
        .expect_err("acceptance rejects negative noise")
        .contains("blocked"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_pre_publication_failure_preserves_the_acceptance_pair() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let _gpu = EnvSet::set("GENOS_REFERENCE_GPU", None);
    let commit = "d8142c375dc515d73a041d485c37cb44bc2fe628";
    let root = std::env::temp_dir().join(format!("cache-energy-stage-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let good = fixture_reference(0.01, 128, true, true);
    let isolated = root.join("isolated.ref");
    write_fixture(&isolated, &good);
    let (scene, camera) = open_floor();
    let acceptance = acceptance_setup(&scene, &camera, 64);
    let mut produced = acceptance.clone();
    produced.max_spp = 8_192;
    produced.seconds = 30.0;
    let floor_dir = root.join("floor");
    let floor_key = genos_debug::reference::cache_key(&acceptance);
    let (floor_marker, floor_note) = seed_directory_dest(&floor_dir, floor_key);
    let floor_before = dir_files(&floor_dir);
    let err = promote_floor_isolated(&Promotion {
        reference: &good,
        produced: &produced,
        acceptance: &acceptance,
        commit,
        expected_commit: commit,
        isolated: &isolated,
        acceptance_dir: &floor_dir,
    })
    .expect_err("floor replace must fail before the live name changes");
    assert!(err.contains("reference replace failed"), "{err}");
    assert!(!err.contains("blocked"), "{err}");
    assert_eq!(dir_files(&floor_dir), floor_before, "{err}");
    assert_eq!(std::fs::read(&floor_marker).expect("marker"), b"inside");
    assert_eq!(std::fs::read(&floor_note).expect("note"), b"existing-note-bytes");
    assert!(floor_dir.join(format!("{floor_key:016x}.ref")).is_dir());
    let (hall_scene, hall_camera) = hall();
    let hall_acceptance = acceptance_setup(&hall_scene, &hall_camera, 64);
    let mut hall_produced = hall_acceptance.clone();
    hall_produced.seconds = 30.0;
    let hall_dir = root.join("hall");
    let hall_key = genos_debug::reference::cache_key(&hall_acceptance);
    let (hall_marker, hall_note) = seed_directory_dest(&hall_dir, hall_key);
    let hall_before = dir_files(&hall_dir);
    let hall_err = promote_isolated(&Promotion {
        reference: &good,
        produced: &hall_produced,
        acceptance: &hall_acceptance,
        commit,
        expected_commit: commit,
        isolated: &isolated,
        acceptance_dir: &hall_dir,
    })
    .expect_err("hall replace must fail before the live name changes");
    assert!(hall_err.contains("reference replace failed"), "{hall_err}");
    assert!(!hall_err.contains("blocked"), "{hall_err}");
    assert_eq!(dir_files(&hall_dir), hall_before, "{hall_err}");
    assert_eq!(std::fs::read(&hall_marker).expect("marker"), b"inside");
    assert_eq!(std::fs::read(&hall_note).expect("note"), b"existing-note-bytes");
    assert!(hall_dir.join(format!("{hall_key:016x}.ref")).is_dir());
    let _ = std::fs::remove_dir_all(&root);
}

/// The live `.ref` name is a directory, so the first rename cannot replace it.
/// This does not depend on the process being unable to write a mode-0555 directory.
fn seed_directory_dest(dir: &Path, key: u64) -> (PathBuf, PathBuf) {
    std::fs::create_dir_all(dir).expect("cache");
    let dest = dir.join(format!("{key:016x}.ref"));
    std::fs::create_dir(&dest).expect("destination shape");
    let marker = dest.join("keep");
    std::fs::write(&marker, b"inside").expect("marker");
    let note = dir.join(format!("{key:016x}.provenance.json"));
    std::fs::write(&note, b"existing-note-bytes").expect("note");
    (marker, note)
}

fn note_field(note: &str, field: &str) -> Option<String> {
    json_fields(note)
        .into_iter()
        .find(|(key, _)| key == field)
        .map(|(_, value)| value)
}

#[test]
fn render_cached_reuse_line_includes_the_sticky_cap() {
    let line = cached_reuse_line(
        "floor precompute",
        "5a22737fc92f6b177083fb77995410f65281bedc",
        0xdfe0_32d1_638e_605b,
    );
    assert!(line.contains("setup cap"), "{line}");
    assert!(line.contains("capped noisy"), "{line}");
    assert!(!line.contains("only when noise"), "{line}");
    let hall = cached_reuse_line("precompute", "d8142c375dc515d73a041d485c37cb44bc2fe628", 1);
    assert!(hall.contains("capped noisy"), "{hall}");
    assert!(!hall.contains("only when noise"), "{hall}");
}

#[test]
fn a_reused_floor_keeps_trace_elapsed_and_records_promotion() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let (scene, camera) = open_floor();
    let acceptance = acceptance_setup(&scene, &camera, 64);
    let mut produced = acceptance.clone();
    produced.max_spp = OFFLINE_MAX_SPP;
    produced.seconds = PRECOMPUTE_SECONDS_CAP;
    let commit = "5a22737fc92f6b177083fb77995410f65281bedc";
    let key = genos_debug::reference::cache_key(&acceptance);
    let prior = floor_promotion_note(
        commit,
        key,
        &produced,
        &fixture_reference(0.026_066_182, OFFLINE_MAX_SPP, true, true),
        false,
        Some(24.5),
    );
    assert_eq!(
        recorded_trace_seconds(true, 0.0, &prior).map(f32::to_bits),
        Some(24.5_f32.to_bits()),
        "reload must keep the sidecar clock"
    );
    assert_eq!(
        recorded_trace_seconds(true, 0.0, "{\"scene\": \"floor\"}"),
        None,
        "a missing clock is not zero"
    );
    assert_eq!(
        recorded_trace_seconds(false, 6.25, ""),
        Some(6.25),
        "a fresh trace keeps its own clock, including a real zero"
    );
    assert_eq!(recorded_trace_seconds(false, 0.0, &prior), Some(0.0));
    let root = std::env::temp_dir().join(format!(
        "cache-energy-floor-elapsed-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let note_path = root.join("provenance.json");
    let mut loaded = fixture_reference(0.01, 8_192, true, true);
    loaded.seconds = 0.0;
    write_floor_isolated_sidecar(
        &note_path,
        commit,
        key,
        &produced,
        &loaded,
        true,
        recorded_trace_seconds(true, loaded.seconds, &prior),
    )
    .expect("sidecar");
    let kept = std::fs::read_to_string(&note_path).expect("note");
    assert_eq!(note_field(&kept, "seconds").as_deref(), Some("24.5"), "{kept}");
    assert_eq!(note_field(&kept, "promoted").as_deref(), Some("true"), "{kept}");
    write_floor_isolated_sidecar(
        &note_path,
        commit,
        key,
        &produced,
        &loaded,
        false,
        None,
    )
    .expect("missing clock");
    let missing = std::fs::read_to_string(&note_path).expect("note");
    assert_eq!(note_field(&missing, "seconds").as_deref(), Some("null"), "{missing}");
    assert_ne!(note_field(&missing, "seconds").as_deref(), Some("0"));
    assert_eq!(note_field(&missing, "promoted").as_deref(), Some("false"), "{missing}");
    let fresh = fixture_reference(0.01, 128, true, true);
    let hall_kept = hall_isolated_note(commit, key, &produced, &fresh, true);
    assert!(hall_kept.contains("\"promoted\": true"), "{hall_kept}");
    let hall_blocked = hall_isolated_note(commit, key, &produced, &fresh, false);
    assert!(hall_blocked.contains("\"promoted\": false"), "{hall_blocked}");
    let isolated = root.join("isolated.ref");
    write_fixture(&isolated, &loaded);
    loaded.seconds = 24.5;
    let acceptance_dir = root.join("acceptance");
    let dest = promote_floor_isolated(&Promotion {
        reference: &loaded,
        produced: &produced,
        acceptance: &acceptance,
        commit,
        expected_commit: commit,
        isolated: &isolated,
        acceptance_dir: &acceptance_dir,
    })
    .expect("promote records the restored clock");
    let cache_note = std::fs::read_to_string(acceptance_dir.join(format!("{key:016x}.provenance.json")))
        .expect("acceptance note");
    assert_eq!(note_field(&cache_note, "seconds").as_deref(), Some("24.5"), "{cache_note}");
    assert_eq!(note_field(&cache_note, "promoted").as_deref(), Some("true"), "{cache_note}");
    assert_eq!(dest, acceptance_dir.join(format!("{key:016x}.ref")));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "CPU path trace of the open floor, at most 262144 spp and 2400 s, not acceptance"]
fn precompute_floor_b64() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let (max_spp, seconds) = floor_budgets_from_env().expect("CACHE_ENERGY_FLOOR_MAX_SPP and CACHE_ENERGY_FLOOR_SECONDS");
    precompute_floor_b64_offline(max_spp, seconds).expect("precompute");
}

#[test]
#[ignore = "lavapipe draw, about 10 frames, no path trace"]
fn a_dump_file_is_visible_on_draw_n_plus_5_and_names_frame_n() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let dump_n = 4u64;
    let dir = std::env::temp_dir().join(format!("cache-energy-dump-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dump dir");
    let dump_dir = dir.to_str().expect("utf8 dump dir");
    let _dump = EnvSet::set("GENOS_LIGHTING_DUMP", Some(&dump_n.to_string()));
    let _dump_dir = EnvSet::set("GENOS_LIGHTING_DUMP_DIR", Some(dump_dir));
    let _preset = EnvSet::set("GENOS_LIGHTING_PRESET", None);
    let _sh = EnvSet::set("GENOS_LIGHTING_SURFACE_CACHE_SH_CELLS", None);
    let _gpu = EnvSet::set("GENOS_REFERENCE_GPU", None);
    let (scene, camera) = closed_box();
    let world = World::from_scene(scene);
    let mut window = Window::open_proof(WIDTH, HEIGHT).expect("proof window");
    let size = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, size.width, size.height).expect("renderer");
    let mut owner = genos_window::Lifecycle::new(window, renderer, permit);
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let mut settings = LightingSettings::preset(LightingPreset::High);
    settings.surface_cache.sh_cells = true;
    renderer.set_lighting_settings(settings);
    let layout = dir.join(format!("f{dump_n}_layout.json"));
    for draw in 1..=dump_n + 4 {
        let _ = window.pump();
        let pixels = renderer
            .draw(&world, &camera, true)
            .expect("draw")
            .expect("pixels");
        assert!(!pixels.is_empty(), "draw {draw}");
        assert!(
            !layout.exists(),
            "f{dump_n} was visible on draw {draw}, before N+5"
        );
    }
    let _ = window.pump();
    let pixels = renderer
        .draw(&world, &camera, true)
        .expect("draw")
        .expect("pixels");
    assert!(!pixels.is_empty(), "draw {}", dump_n + 5);
    let text = std::fs::read_to_string(&layout)
        .unwrap_or_else(|_| format!("missing at draw {} ({})", dump_n + 5, dir_list(&dir)));
    assert!(text.contains(&format!("\"frame\": {dump_n}")), "{text}");
    assert!(
        dir.join(format!("f{dump_n}_keys.bin")).is_file(),
        "{}",
        dir_list(&dir)
    );
}

