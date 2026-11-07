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
//! 0.02 noise, hits the 600 s or 4096 spp cap, or is nonfinite or missing is
//! blocked. A reconstructed linger from a rejected run is not a pass. The
//! oracle's `energy_mean` is the full picture, direct light included, and is
//! a separate report.
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

fn keep_fail(dir: &Path, detail: &str) -> ! {
    keep(
        dir,
        &format!("{{\n  \"verdict\": \"fail\",\n  \"detail\": \"{}\"\n}}\n", escape(detail)),
    );
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
    if facts.noise > facts.noise_target {
        let capped = facts.seconds >= facts.seconds_budget || facts.spp >= facts.max_spp;
        let why = if capped { "capped" } else { "above the noise target" };
        return Err(format!(
            "blocked: reference noise {:.6} > {:.3} ({why}, spp {}, seconds {:.3})",
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

fn trace_acceptance(scene: &Scene, camera: &Camera, bounces: u32) -> Result<genos_debug::Reference, String> {
    let mut setup = genos_debug::reference::RefSetup::from_camera(scene, camera, WIDTH, HEIGHT, 32);
    setup.max_bounces = bounces;
    setup.noise_target = NOISE_TARGET;
    setup.max_spp = MAX_SPP;
    setup.seconds = TRACE_SECONDS;
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

fn measured_json(name: &str, schedule: &LampSchedule, detail: &str, verdict: &str) -> String {
    format!(
        "{{\n  \"verdict\": \"{verdict}\",\n  \"name\": \"{name}\",\n  \"detail\": \"{}\",\n  {},\n  \"files\": \"{}\"\n}}\n",
        escape(detail),
        schedule_json(schedule),
        escape(&dir_list(&artifact_dir(name)))
    )
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
) -> (TracePair, genos_debug::Reference) {
    let blocked = |err: String| -> ! {
        keep(artifacts, &measured_json(name, schedule, &err, "fail"));
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
    keep(&artifacts, &measured_json(name, &schedule, "tracing", "running"));
    let (bounce, direct_reference) = load_traces(name, &schedule, &artifacts, camera, graded_scene, lit_scene);
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
    let (renderer, permit) = match Renderer::open(window.display, window.surface, size.width, size.height) {
        Ok(open) => open,
        Err(err) => keep_fail(&artifacts, &format!("renderer: {err}")),
    };
    let mut owner = genos_window::Lifecycle::new(window, renderer, permit);
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let mut settings = LightingSettings::preset(LightingPreset::High);
    settings.surface_cache.sh_cells = true;
    renderer.set_lighting_settings(settings);
    let mut host = FrameHost { window, renderer, camera, artifacts: &artifacts };
    let warm = WarmGate { scene: lit_scene, b0: Some(&bounce.b0), b64: Some(&bounce.b64) };
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
    let blotch = picture.blotch.map_or_else(|| "none".to_string(), |value| format!("{value:.3}"));
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
    keep(&artifacts, &measured_json(name, &schedule, &detail, "measured"));
    eprintln!("cache_energy {name}: {detail}");
    match measure {
        Measure::Linger { .. } => {
            if !drawn.warm_captured {
                keep_fail(&artifacts, "linger finished without a warm picture");
            }
            if let Err(err) = linger_holds(&picture, &cells.coarse) {
                keep_fail(&artifacts, &err);
            }
        }
        Measure::Energy { .. } => {
            if let Err(err) = energy_holds(&picture, &cells.coarse) {
                keep_fail(&artifacts, &err);
            }
            if drawn.settled_at.is_none() {
                let cost = drawn.entries.iter().sum::<f64>() / drawn.entries.len().max(1) as f64;
                keep_fail(
                    &artifacts,
                    &format!("a still scene never settled ({cost:.0} entries a frame, draining {})", drawn.draining),
                );
            }
        }
        Measure::Lag { .. } => {
            if let Err(err) = energy_holds(&picture, &cells.coarse) {
                keep_fail(&artifacts, &err);
            }
        }
    }
    keep(&artifacts, &measured_json(name, &schedule, &detail, "pass"));
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
