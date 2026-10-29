//! JSON dump the GPU stage diffs against.
//!
//! `reference.json` holds the scene, the config, the brute-force image, the
//! multi-seed mean image, the per-frame mean luminance, and one seed's
//! reservoirs. A later stage writes the same schema. [`diff_dumps`] scores
//! `ref_err_near`, and when the candidate includes them, `grid_worst`,
//! `frames_to_ref`, `frame_ms`, and the reservoirs.

use genos_json::{array, encode, float, int, object, parse, string, Value};
use genos_math::Vec3;

use crate::metrics::{frames_to_ref, grid_worst, ref_err_luma, ref_err_near, GateLimits};
use crate::reservoir::{reservoir_w, Reservoir, Weighting, LAMP_EMPTY};
use crate::restir::{Config, History, Proposal};
use crate::shade::{Aabb, Lamp, Receiver, Scene};

const KIND: &str = "genos-restir-reference";
const VERSION: i64 = 1;

/// One finished run, ready to encode.
#[derive(Clone, Debug)]
pub struct Dump {
    pub scene: Scene,
    pub config: Config,
    pub seeds: Vec<u64>,
    pub frames: u32,
    pub reservoir_seed: u64,
    pub brute: Vec<Vec3>,
    pub estimate: Vec<Vec3>,
    pub frame_luminance: Vec<Vec<f32>>,
    pub reservoirs: Vec<Reservoir>,
    pub frame_ms: Option<f32>,
}

/// What [`diff_dumps`] found. `pass` is `reasons` empty.
#[derive(Clone, Debug)]
pub struct Diff {
    pub ref_err_near: f32,
    pub grid_worst: Option<f32>,
    pub frames_to_ref: Option<u32>,
    pub frame_ms: Option<f32>,
    pub reservoir_mismatches: u32,
    pub max_w_rel: f32,
    pub reasons: Vec<String>,
}

impl Diff {
    #[must_use]
    pub fn pass(&self) -> bool {
        self.reasons.is_empty()
    }
}

/// Scores `candidate` against `reference.brute` and, when present, the sequence and reservoirs.
#[must_use]
pub fn diff_dumps(reference: &Dump, candidate: &Dump, limits: &GateLimits) -> Diff {
    let mut reasons = Vec::new();
    if !scene_matches(&reference.scene, &candidate.scene) {
        reasons.push("scene does not match the reference".to_string());
    }
    if candidate.estimate.len() != reference.brute.len() {
        reasons.push("estimate length does not match brute".to_string());
    }
    let error = ref_err_near(&candidate.estimate, &reference.brute, &reference.scene);
    if error > limits.ref_err_near {
        reasons.push(format!(
            "ref_err_near {error:.4} above {:.4}",
            limits.ref_err_near
        ));
    }
    let (grid, frames) = sequence_gates(candidate, reference, limits, &mut reasons);
    let frame_ms = candidate.frame_ms;
    if let Some(ms) = frame_ms {
        if !ms.is_finite() || ms > limits.frame_ms {
            reasons.push(format!("frame_ms {ms:.3} above {:.3}", limits.frame_ms));
        }
    }
    let (mismatches, max_w) =
        reservoir_delta(&reference.reservoirs, &candidate.reservoirs, &mut reasons);
    Diff {
        ref_err_near: error,
        grid_worst: grid,
        frames_to_ref: frames,
        frame_ms,
        reservoir_mismatches: mismatches,
        max_w_rel: max_w,
        reasons,
    }
}

fn sequence_gates(
    candidate: &Dump,
    reference: &Dump,
    limits: &GateLimits,
    reasons: &mut Vec<String>,
) -> (Option<f32>, Option<u32>) {
    if candidate.frame_luminance.is_empty() {
        return (None, None);
    }
    let width = reference.scene.width;
    let grid = grid_worst(&candidate.frame_luminance, width);
    if grid > limits.grid_worst {
        reasons.push(format!(
            "grid_worst {grid:.4} above {:.4}",
            limits.grid_worst
        ));
    }
    let errors: Vec<f32> = candidate
        .frame_luminance
        .iter()
        .map(|frame| ref_err_luma(frame, &reference.brute, &reference.scene))
        .collect();
    let frames = frames_to_ref(&errors);
    let cap = u32::try_from(errors.len()).unwrap_or(0);
    if frames == 0 || frames > limits.frames_to_ref || frames > cap {
        reasons.push(format!(
            "frames_to_ref {frames} above {} (sequence {cap})",
            limits.frames_to_ref
        ));
    }
    (Some(grid), Some(frames))
}

fn reservoir_delta(
    reference: &[Reservoir],
    candidate: &[Reservoir],
    reasons: &mut Vec<String>,
) -> (u32, f32) {
    if candidate.is_empty() {
        return (0, 0.0);
    }
    if candidate.len() != reference.len() {
        reasons.push("reservoir count does not match".to_string());
        return (u32::try_from(candidate.len()).unwrap_or(u32::MAX), 1.0);
    }
    let mut mismatches = 0u32;
    let mut max_w = 0.0f32;
    for (left, right) in reference.iter().zip(candidate.iter()) {
        if left.lamp != right.lamp || left.is_empty() != right.is_empty() {
            mismatches = mismatches.saturating_add(1);
        }
        let scale = reservoir_w(*left)
            .abs()
            .max(reservoir_w(*right).abs())
            .max(1.0e-6);
        max_w = max_w.max((reservoir_w(*left) - reservoir_w(*right)).abs() / scale);
    }
    if mismatches > 0 {
        reasons.push(format!("{mismatches} reservoir lamps differ"));
    }
    if max_w > 1.0e-3 {
        reasons.push(format!("reservoir W relative error {max_w:.4}"));
    }
    (mismatches, max_w)
}

fn scene_matches(left: &Scene, right: &Scene) -> bool {
    left.width == right.width
        && near3(left.camera, right.camera)
        && left.lamps.len() == right.lamps.len()
        && left.receivers.len() == right.receivers.len()
        && left.occluders.len() == right.occluders.len()
        && left
            .lamps
            .iter()
            .zip(&right.lamps)
            .all(|(a, b)| lamp_near(a, b))
        && left
            .receivers
            .iter()
            .zip(&right.receivers)
            .all(|(a, b)| receiver_near(a, b))
        && left
            .occluders
            .iter()
            .zip(&right.occluders)
            .all(|(a, b)| box_near(a, b))
}

fn lamp_near(left: &Lamp, right: &Lamp) -> bool {
    near3(left.position, right.position) && near3(left.intensity, right.intensity)
}

fn receiver_near(left: &Receiver, right: &Receiver) -> bool {
    near3(left.position, right.position)
        && near3(left.normal, right.normal)
        && near3(left.albedo, right.albedo)
}

fn box_near(left: &Aabb, right: &Aabb) -> bool {
    near3(left.min, right.min) && near3(left.max, right.max)
}

fn near3(left: Vec3, right: Vec3) -> bool {
    (left.x - right.x).abs() <= 1.0e-4
        && (left.y - right.y).abs() <= 1.0e-4
        && (left.z - right.z).abs() <= 1.0e-4
}

/// Encodes a dump. The keys stay in this order.
#[expect(
    clippy::module_name_repetitions,
    reason = "encode_dump is the name the binary and the diff call"
)]
#[must_use]
pub fn encode_dump(dump: &Dump) -> String {
    let value = object([
        ("kind", string(KIND)),
        ("version", int(VERSION)),
        ("scene", scene_value(&dump.scene)),
        ("config", config_value(&dump.config)),
        (
            "seeds",
            array(dump.seeds.iter().copied().map(seed_value).collect()),
        ),
        ("frames", int(i64::from(dump.frames))),
        ("reservoir_seed", seed_value(dump.reservoir_seed)),
        ("brute", colours(&dump.brute)),
        ("estimate", colours(&dump.estimate)),
        ("frame_luminance", frames_value(&dump.frame_luminance)),
        ("reservoirs", reservoirs_value(&dump.reservoirs)),
        ("frame_ms", ms_value(dump.frame_ms)),
    ]);
    encode(&value)
}

/// Reads a dump.
///
/// # Errors
///
/// Returns a string when the text is not this schema.
#[expect(
    clippy::module_name_repetitions,
    reason = "parse_dump is the name the binary and the diff call"
)]
pub fn parse_dump(text: &str) -> Result<Dump, String> {
    let value = parse(text)?;
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .ok_or("missing kind")?;
    if kind != KIND {
        return Err(format!("kind {kind}"));
    }
    let scene = parse_scene(value.get("scene").ok_or("missing scene")?)?;
    let config = parse_config(value.get("config").ok_or("missing config")?)?;
    let seeds = parse_seeds(value.get("seeds").ok_or("missing seeds")?)?;
    let frames = u32_field(&value, "frames")?;
    let reservoir_seed = parse_seed(
        value
            .get("reservoir_seed")
            .ok_or("missing reservoir_seed")?,
    )?;
    let brute = parse_colours(value.get("brute").ok_or("missing brute")?)?;
    let estimate = parse_colours(value.get("estimate").ok_or("missing estimate")?)?;
    let frame_luminance = parse_frames(
        value
            .get("frame_luminance")
            .ok_or("missing frame_luminance")?,
    )?;
    let reservoirs = parse_reservoirs(value.get("reservoirs").ok_or("missing reservoirs")?)?;
    let frame_ms = parse_ms(value.get("frame_ms"))?;
    Ok(Dump {
        scene,
        config,
        seeds,
        frames,
        reservoir_seed,
        brute,
        estimate,
        frame_luminance,
        reservoirs,
        frame_ms,
    })
}

fn scene_value(scene: &Scene) -> Value {
    object([
        ("width", int(i64::try_from(scene.width).unwrap_or(0))),
        ("camera", vec3_value(scene.camera)),
        ("lamps", array(scene.lamps.iter().map(lamp_value).collect())),
        (
            "receivers",
            array(scene.receivers.iter().map(receiver_value).collect()),
        ),
        (
            "occluders",
            array(scene.occluders.iter().map(box_value).collect()),
        ),
    ])
}

fn lamp_value(lamp: &Lamp) -> Value {
    object([
        ("position", vec3_value(lamp.position)),
        ("intensity", vec3_value(lamp.intensity)),
    ])
}

fn receiver_value(receiver: &Receiver) -> Value {
    object([
        ("position", vec3_value(receiver.position)),
        ("normal", vec3_value(receiver.normal)),
        ("albedo", vec3_value(receiver.albedo)),
    ])
}

fn box_value(box_: &Aabb) -> Value {
    object([("min", vec3_value(box_.min)), ("max", vec3_value(box_.max))])
}

fn config_value(config: &Config) -> Value {
    object([
        ("candidates", int(i64::from(config.candidates))),
        (
            "spatial_neighbors",
            int(i64::from(config.spatial_neighbors)),
        ),
        ("spatial_radius", int(i64::from(config.spatial_radius))),
        ("temporal", genos_json::bool(config.temporal)),
        ("weighting", string(weight_name(config.weighting))),
        ("proposal", string(proposal_name(config.proposal))),
        ("history", string(history_name(config.history))),
        ("m_clamp", int(i64::from(config.m_clamp))),
    ])
}

fn weight_name(weighting: Weighting) -> &'static str {
    match weighting {
        Weighting::Balance => "balance",
        Weighting::Section3 => "section3",
    }
}

fn proposal_name(proposal: Proposal) -> &'static str {
    match proposal {
        Proposal::Uniform => "uniform",
        Proposal::Target => "target",
    }
}

fn history_name(history: History) -> &'static str {
    match history {
        History::Temporal => "temporal",
        History::Final => "final",
    }
}

fn colours(values: &[Vec3]) -> Value {
    array(values.iter().copied().map(vec3_value).collect())
}

fn frames_value(frames: &[Vec<f32>]) -> Value {
    array(
        frames
            .iter()
            .map(|frame| {
                array(
                    frame
                        .iter()
                        .copied()
                        .map(|value| float(f64::from(value)))
                        .collect(),
                )
            })
            .collect(),
    )
}

fn reservoirs_value(reservoirs: &[Reservoir]) -> Value {
    array(reservoirs.iter().copied().map(reservoir_value).collect())
}

fn reservoir_value(reservoir: Reservoir) -> Value {
    let lamp = if reservoir.lamp == LAMP_EMPTY {
        int(-1)
    } else {
        int(i64::from(reservoir.lamp))
    };
    object([
        ("lamp", lamp),
        ("w_sum", float(f64::from(reservoir.w_sum))),
        ("m", int(i64::from(reservoir.m))),
        ("target", float(f64::from(reservoir.target))),
    ])
}

fn vec3_value(value: Vec3) -> Value {
    array(vec![
        float(f64::from(value.x)),
        float(f64::from(value.y)),
        float(f64::from(value.z)),
    ])
}

fn seed_value(seed: u64) -> Value {
    int(i64::try_from(seed).unwrap_or(0))
}

fn ms_value(ms: Option<f32>) -> Value {
    match ms {
        Some(value) => float(f64::from(value)),
        None => Value::Null,
    }
}

fn parse_scene(value: &Value) -> Result<Scene, String> {
    let width = usize_field(value, "width")?;
    let camera = parse_vec3(value.get("camera").ok_or("missing camera")?)?;
    let lamps = parse_list(value.get("lamps").ok_or("missing lamps")?, parse_lamp)?;
    let receivers = parse_list(
        value.get("receivers").ok_or("missing receivers")?,
        parse_receiver,
    )?;
    let occluders = parse_list(
        value.get("occluders").ok_or("missing occluders")?,
        parse_box,
    )?;
    Ok(Scene {
        lamps,
        receivers,
        occluders,
        width,
        camera,
    })
}

fn parse_lamp(value: &Value) -> Result<Lamp, String> {
    Ok(Lamp {
        position: parse_vec3(value.get("position").ok_or("missing position")?)?,
        intensity: parse_vec3(value.get("intensity").ok_or("missing intensity")?)?,
    })
}

fn parse_receiver(value: &Value) -> Result<Receiver, String> {
    Ok(Receiver {
        position: parse_vec3(value.get("position").ok_or("missing position")?)?,
        normal: parse_vec3(value.get("normal").ok_or("missing normal")?)?,
        albedo: parse_vec3(value.get("albedo").ok_or("missing albedo")?)?,
    })
}

fn parse_box(value: &Value) -> Result<Aabb, String> {
    Ok(Aabb {
        min: parse_vec3(value.get("min").ok_or("missing min")?)?,
        max: parse_vec3(value.get("max").ok_or("missing max")?)?,
    })
}

fn parse_config(value: &Value) -> Result<Config, String> {
    let weighting = match value.get("weighting").and_then(Value::as_str).unwrap_or("") {
        "balance" => Weighting::Balance,
        "section3" => Weighting::Section3,
        other => return Err(format!("weighting {other}")),
    };
    let proposal = match value.get("proposal").and_then(Value::as_str).unwrap_or("") {
        "uniform" => Proposal::Uniform,
        "target" => Proposal::Target,
        other => return Err(format!("proposal {other}")),
    };
    let history = match value.get("history").and_then(Value::as_str).unwrap_or("") {
        "temporal" => History::Temporal,
        "final" => History::Final,
        other => return Err(format!("history {other}")),
    };
    Ok(Config {
        candidates: u32_field(value, "candidates")?,
        spatial_neighbors: u32_field(value, "spatial_neighbors")?,
        spatial_radius: u32_field(value, "spatial_radius")?,
        temporal: value
            .get("temporal")
            .and_then(Value::as_bool)
            .ok_or("temporal")?,
        weighting,
        proposal,
        history,
        m_clamp: value
            .get("m_clamp")
            .map_or(Ok(crate::restir::DEFAULT_M_CLAMP), |_| {
                u32_field(value, "m_clamp")
            })?,
    })
}

fn parse_seeds(value: &Value) -> Result<Vec<u64>, String> {
    let items = value.as_array().ok_or("seeds")?;
    items.iter().map(parse_seed).collect()
}

fn parse_seed(value: &Value) -> Result<u64, String> {
    let raw = value.as_i64().ok_or("seed")?;
    u64::try_from(raw).map_err(|_| "seed out of range".to_string())
}

fn parse_colours(value: &Value) -> Result<Vec<Vec3>, String> {
    parse_list(value, parse_vec3)
}

fn parse_frames(value: &Value) -> Result<Vec<Vec<f32>>, String> {
    let items = value.as_array().ok_or("frame_luminance")?;
    items
        .iter()
        .map(|frame| {
            let row = frame.as_array().ok_or("frame")?;
            row.iter().map(parse_f32).collect()
        })
        .collect()
}

fn parse_reservoirs(value: &Value) -> Result<Vec<Reservoir>, String> {
    parse_list(value, parse_reservoir)
}

fn parse_reservoir(value: &Value) -> Result<Reservoir, String> {
    let raw = value.get("lamp").and_then(Value::as_i64).ok_or("lamp")?;
    let lamp = if raw < 0 {
        LAMP_EMPTY
    } else {
        u32::try_from(raw).map_err(|_| "lamp out of range".to_string())?
    };
    Ok(Reservoir {
        lamp,
        w_sum: parse_f32(value.get("w_sum").ok_or("w_sum")?)?,
        m: u32_field(value, "m")?,
        target: parse_f32(value.get("target").ok_or("target")?)?,
    })
}

fn parse_list<T>(
    value: &Value,
    each: impl Fn(&Value) -> Result<T, String>,
) -> Result<Vec<T>, String> {
    let items = value.as_array().ok_or("expected a list")?;
    items.iter().map(each).collect()
}

fn parse_vec3(value: &Value) -> Result<Vec3, String> {
    let items = value.as_array().ok_or("expected 3 numbers")?;
    if items.len() != 3 {
        return Err("expected 3 numbers".to_string());
    }
    let x = parse_f32(items.first().ok_or("x")?)?;
    let y = parse_f32(items.get(1).ok_or("y")?)?;
    let z = parse_f32(items.get(2).ok_or("z")?)?;
    Ok(Vec3::new(x, y, z))
}

fn parse_ms(value: Option<&Value>) -> Result<Option<f32>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(other) => Ok(Some(parse_f32(other)?)),
    }
}

fn parse_f32(value: &Value) -> Result<f32, String> {
    let wide = value.as_f64().ok_or("expected a number")?;
    if !wide.is_finite() {
        return Err("non-finite number".to_string());
    }
    Ok(narrow(wide))
}

fn u32_field(value: &Value, key: &str) -> Result<u32, String> {
    let raw = value
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("missing {key}"))?;
    u32::try_from(raw).map_err(|_| format!("{key} out of range"))
}

fn usize_field(value: &Value, key: &str) -> Result<usize, String> {
    let raw = u32_field(value, key)?;
    Ok(usize::try_from(raw).unwrap_or(0))
}

fn narrow(value: f64) -> f32 {
    genos_math::f32_from_f64(value)
}
