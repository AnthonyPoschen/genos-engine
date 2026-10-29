//! Multi-seed run that fills the four stage-C gates.
//!
//! The mean image over seeds is what `ref_err_near`, `grid_worst`, and
//! `frames_to_ref` see. The gate scene has no occluder, so a target proposal
//! holds the luminance still. `Scene::shadow` is a separate bias check.
//! `frame_ms` times one `Session::step` on the same scene. One seed's
//! reservoirs are stored so a GPU that adopts the PCG stream can be diffed
//! lamp-for-lamp.

use std::time::Instant;

use genos_math::Vec3;

use crate::dump::Dump;
use crate::metrics::{frames_to_ref, grid_worst, ref_err_near, GateLimits};
use crate::reservoir::m_as_f32;
use crate::restir::{Config, Session};
use crate::shade::{brute, luminance, Scene};

/// Inputs for [`evaluate`].
#[derive(Clone, Debug)]
pub struct EvalSpec {
    pub scene: Scene,
    pub config: Config,
    pub seeds: Vec<u64>,
    pub frames: u32,
    pub limits: GateLimits,
}

impl EvalSpec {
    /// The unoccluded gate scene, 64 seeds, 8 frames, reference ceilings.
    #[must_use]
    pub fn gate() -> Self {
        Self {
            scene: Scene::gate(),
            config: Config::gate(),
            seeds: (1..=64).collect(),
            frames: 8,
            limits: GateLimits::reference(),
        }
    }
}

/// Gate numbers for one [`evaluate`].
#[derive(Clone, Debug)]
pub struct Report {
    pub ref_err_near: f32,
    pub grid_worst: f32,
    pub frames_to_ref: u32,
    pub frame_ms: f32,
    pub variance_early: f32,
    pub variance_late: f32,
    pub variance_ratio: f32,
    pub frame_ref_err_near: Vec<f32>,
    pub pass: bool,
    pub limits: GateLimits,
}

/// Runs the spec. The dump's `estimate` is the last frame's multi-seed mean.
///
/// # Errors
///
/// Returns a string when the spec has no seeds or frames, or a session refuses the scene.
pub fn evaluate(spec: &EvalSpec) -> Result<(Report, Dump), String> {
    if spec.seeds.is_empty() || spec.frames == 0 {
        return Err("seeds and frames must be positive".to_string());
    }
    let frame_count = usize::try_from(spec.frames).unwrap_or(0);
    let receivers = spec.scene.receivers.len();
    let brute_image = brute_image(&spec.scene);
    let watch = brightest(&brute_image);
    let mut sums = vec![vec![Vec3::ZERO; receivers]; frame_count];
    let mut early = Vec::new();
    let mut late = Vec::new();
    let mut reservoirs = Vec::new();
    let reservoir_seed = spec.seeds.first().copied().unwrap_or(1);
    for seed in &spec.seeds {
        let mut session = Session::new(spec.scene.clone(), spec.config, *seed)?;
        for (frame, sum) in sums.iter_mut().enumerate() {
            let radiance = session.step();
            accumulate(sum, &radiance);
            let sample = radiance.get(watch).copied().unwrap_or(Vec3::ZERO);
            if frame == 0 {
                early.push(luminance(sample));
            }
            if frame + 1 == frame_count {
                late.push(luminance(sample));
                if *seed == reservoir_seed {
                    reservoirs = session.shaded().to_vec();
                }
            }
        }
    }
    let scale = m_as_f32(u32::try_from(spec.seeds.len()).unwrap_or(1));
    let means = scale_frames(&sums, scale);
    let frame_luminance = luma_frames(&means);
    let frame_ref_err_near: Vec<f32> = means
        .iter()
        .map(|frame| ref_err_near(frame, &brute_image, &spec.scene))
        .collect();
    let final_image = means.last().cloned().unwrap_or_default();
    let error = frame_ref_err_near.last().copied().unwrap_or(0.0);
    let grid = grid_worst(&frame_luminance, spec.scene.width);
    let settle = frames_to_ref(&frame_ref_err_near);
    let frame_ms = time_one_step(spec)?;
    let variance_early = variance(&early);
    let variance_late = variance(&late);
    let variance_ratio = if variance_early > 1.0e-12 {
        variance_late / variance_early
    } else {
        1.0
    };
    let pass = error <= spec.limits.ref_err_near
        && grid <= spec.limits.grid_worst
        && settle >= 1
        && settle <= spec.limits.frames_to_ref
        && settle <= spec.frames
        && frame_ms.is_finite()
        && frame_ms <= spec.limits.frame_ms;
    let report = Report {
        ref_err_near: error,
        grid_worst: grid,
        frames_to_ref: settle,
        frame_ms,
        variance_early,
        variance_late,
        variance_ratio,
        frame_ref_err_near,
        pass,
        limits: spec.limits,
    };
    let dump = Dump {
        scene: spec.scene.clone(),
        config: spec.config,
        seeds: spec.seeds.clone(),
        frames: spec.frames,
        reservoir_seed,
        brute: brute_image,
        estimate: final_image,
        frame_luminance,
        reservoirs,
        frame_ms: Some(frame_ms),
    };
    Ok((report, dump))
}

fn brute_image(scene: &Scene) -> Vec<Vec3> {
    scene
        .receivers
        .iter()
        .map(|receiver| brute(receiver, scene))
        .collect()
}

fn brightest(image: &[Vec3]) -> usize {
    let mut best = 0usize;
    let mut best_luma = -1.0f32;
    for (index, rgb) in image.iter().enumerate() {
        let value = luminance(*rgb);
        if value > best_luma {
            best = index;
            best_luma = value;
        }
    }
    best
}

fn accumulate(bucket: &mut [Vec3], sample: &[Vec3]) {
    for (sum, value) in bucket.iter_mut().zip(sample) {
        *sum += *value;
    }
}

fn scale_frames(sums: &[Vec<Vec3>], scale: f32) -> Vec<Vec<Vec3>> {
    sums.iter()
        .map(|frame| frame.iter().map(|value| *value / scale).collect())
        .collect()
}

fn luma_frames(frames: &[Vec<Vec3>]) -> Vec<Vec<f32>> {
    frames
        .iter()
        .map(|frame| frame.iter().copied().map(luminance).collect())
        .collect()
}

fn time_one_step(spec: &EvalSpec) -> Result<f32, String> {
    let seed = spec.seeds.first().copied().unwrap_or(1);
    let mut session = Session::new(spec.scene.clone(), spec.config, seed)?;
    let _ = session.step();
    let started = Instant::now();
    let _ = session.step();
    let millis = started.elapsed().as_secs_f64() * 1000.0;
    Ok(narrow_ms(millis))
}

fn variance(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let scale = m_as_f32(u32::try_from(values.len()).unwrap_or(1));
    let mean = values.iter().sum::<f32>() / scale;
    values
        .iter()
        .map(|value| (value - mean) * (value - mean))
        .sum::<f32>()
        / scale
}

#[expect(
    clippy::as_conversions,
    reason = "Instant reports f64 seconds; the gate stores f32 milliseconds"
)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "Instant reports f64 seconds; the gate stores f32 milliseconds"
)]
fn narrow_ms(value: f64) -> f32 {
    value as f32
}

/// Text form of a report, one JSON object.
#[must_use]
pub fn encode_report(report: &Report) -> String {
    let errors = genos_json::array(
        report
            .frame_ref_err_near
            .iter()
            .map(|value| genos_json::float(f64::from(*value)))
            .collect(),
    );
    let value = genos_json::object([
        ("kind", genos_json::string("genos-restir-metrics")),
        ("version", genos_json::int(1)),
        (
            "ref_err_near",
            genos_json::float(f64::from(report.ref_err_near)),
        ),
        (
            "grid_worst",
            genos_json::float(f64::from(report.grid_worst)),
        ),
        (
            "frames_to_ref",
            genos_json::int(i64::from(report.frames_to_ref)),
        ),
        ("frame_ms", genos_json::float(f64::from(report.frame_ms))),
        (
            "variance_early",
            genos_json::float(f64::from(report.variance_early)),
        ),
        (
            "variance_late",
            genos_json::float(f64::from(report.variance_late)),
        ),
        (
            "variance_ratio",
            genos_json::float(f64::from(report.variance_ratio)),
        ),
        ("frame_ref_err_near", errors),
        ("pass", genos_json::bool(report.pass)),
        (
            "limits",
            genos_json::object([
                (
                    "ref_err_near",
                    genos_json::float(f64::from(report.limits.ref_err_near)),
                ),
                (
                    "grid_worst",
                    genos_json::float(f64::from(report.limits.grid_worst)),
                ),
                (
                    "frames_to_ref",
                    genos_json::int(i64::from(report.limits.frames_to_ref)),
                ),
                (
                    "frame_ms",
                    genos_json::float(f64::from(report.limits.frame_ms)),
                ),
            ]),
        ),
    ]);
    genos_json::encode(&value)
}

/// Writes `metrics.json` and `reference.json`, replacing whatever was there.
///
/// # Errors
///
/// Returns a string when the directory or either file cannot be written.
pub fn write_run(
    dir: &std::path::Path,
    report: &Report,
    dump: &crate::dump::Dump,
) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|err| format!("create {}: {err}", dir.display()))?;
    let metrics = dir.join("metrics.json");
    let reference = dir.join("reference.json");
    std::fs::write(&metrics, encode_report(report))
        .map_err(|err| format!("write {}: {err}", metrics.display()))?;
    std::fs::write(&reference, crate::dump::encode_dump(dump))
        .map_err(|err| format!("write {}: {err}", reference.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{evaluate, EvalSpec};
    use crate::dump::{diff_dumps, encode_dump, parse_dump};
    use crate::metrics::GateLimits;
    use crate::restir::Config;
    use crate::shade::Scene;

    #[test]
    fn the_gate_scene_passes_its_own_ceilings() -> Result<(), String> {
        let (report, dump) = evaluate(&EvalSpec::gate())?;
        assert!(
            report.pass,
            "ref_err_near {:.4} grid_worst {:.4} frames_to_ref {} frame_ms {:.3} variance_ratio {:.3} per frame {:?}",
            report.ref_err_near,
            report.grid_worst,
            report.frames_to_ref,
            report.frame_ms,
            report.variance_ratio,
            report.frame_ref_err_near
        );
        let text = encode_dump(&dump);
        let round = parse_dump(&text)?;
        let diff = diff_dumps(&dump, &round, &GateLimits::reference());
        assert!(diff.pass(), "{:?}", diff.reasons);
        let mut broken = round;
        for rgb in &mut broken.estimate {
            *rgb = genos_math::Vec3::new(rgb.x + 1.0, rgb.y + 1.0, rgb.z + 1.0);
        }
        let bad = diff_dumps(&dump, &broken, &GateLimits::reference());
        assert!(!bad.pass(), "a shifted image must fail");
        Ok(())
    }

    #[test]
    fn a_shadowed_mean_stays_near_the_brute_sum() -> Result<(), String> {
        let spec = EvalSpec {
            scene: Scene::shadow(),
            config: Config::gate(),
            seeds: (1..=128).collect(),
            frames: 4,
            limits: GateLimits::reference(),
        };
        let (report, _) = evaluate(&spec)?;
        assert!(
            report.ref_err_near < 0.05,
            "shadow ref_err_near {:.4} per frame {:?}",
            report.ref_err_near,
            report.frame_ref_err_near
        );
        Ok(())
    }
}
