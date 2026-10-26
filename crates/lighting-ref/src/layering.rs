#![cfg_attr(
    test,
    expect(
        clippy::panic,
        reason = "a fixture test aborts when the grader itself errors"
    )
)]
#![cfg_attr(
    test,
    expect(
        clippy::unwrap_used,
        reason = "a fixture test aborts when the grader itself errors"
    )
)]

//! Picture grades for lamp-layer settling.
//!
//! The comparisons are the ones in `docs/systems/lamp-layering-design.md`.
//! Planted pictures prove each comparison both ways. `--capture` scores a
//! directory of `frame-NNNN.png` files from the engine. Without that
//! directory, `--case baseline` prints `NOT-IMPLEMENTED`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::metrics::{blur_relative, MetricParams};
use crate::picture::Picture;

/// Default directory, relative to the working directory.
pub const DEFAULT_OUT: &str = "genos-debug/gi/layering";

/// Blurred relative error must stay at or under this line. Same line as the
/// lighting reference check.
pub const SETTLE_LINE: f64 = 0.05;

/// Peak minus mean of the per-pixel relative step.
pub const POP_LIMIT: f64 = 0.05;

/// Max `|observed mean step − τ step| / |start − end|`. τ = 50 ms.
pub const FADE_LIMIT: f64 = 0.05;

/// Worst 1% of per-pixel temporal standard deviation on the steady hold,
/// in linear luma.
pub const HOLD_FLICKER_LIMIT: f64 = 0.02;

/// High preset: 200 ms at a 1/60 s step.
pub const HIGH_SETTLE_FRAMES: u32 = 12;

/// High per-lamp sample ceiling, in bytes.
pub const HIGH_LAYER_BYTES: u64 = 1_048_576;

/// Reads that disagree with the published value.
pub const FINISHED_LIMIT: u32 = 0;

/// Fast-blend time constant for High and Medium.
const TAU_SECONDS: f64 = 0.050;

/// Fixed scene step. Same clock as `light_settle.rhai`.
const FRAME_DT: f64 = 1.0 / 60.0;

const HOLD_FRAMES: usize = 4;
const READ_EPS: f64 = 1.0e-3;
const SPAN_FLOOR: f64 = 1.0e-4;
const TILE: u32 = 8;
const GOOD_BYTES: u64 = 262_144;
const BROKEN_BYTES: u64 = 2_097_152;

/// Which planted run to grade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    /// Every comparison passes.
    Good,
    /// Every comparison fails.
    Broken,
    /// A uniform jump. `pop_magnitude` passes. `fade_deviation` fails.
    Instant,
    /// A steady picture with a pixel that shimmers. `hold_flicker` fails.
    Noisy,
    /// Today's engine, with no capture directory.
    Baseline,
}

impl Case {
    /// `good`, `broken`, `instant`, `noisy`, or `baseline`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "good" => Some(Self::Good),
            "broken" => Some(Self::Broken),
            "instant" => Some(Self::Instant),
            "noisy" => Some(Self::Noisy),
            "baseline" => Some(Self::Baseline),
            _ => None,
        }
    }

    /// File stem and the `case` field.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Good => "good",
            Self::Broken => "broken",
            Self::Instant => "instant",
            Self::Noisy => "noisy",
            Self::Baseline => "baseline",
        }
    }
}

/// One row of the summary.
#[derive(Clone, Debug, PartialEq)]
pub struct Metric {
    /// Stable name.
    pub name: &'static str,
    /// Missing when the row is `NOT-IMPLEMENTED`.
    pub value: MetricValue,
    /// High-preset limit.
    pub threshold: MetricValue,
    /// Pass, fail, or not implemented.
    pub status: Status,
    /// Why a row has no value. Omitted from the text line.
    pub detail: Option<&'static str>,
}

/// A measured number, or no measurement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MetricValue {
    /// No capture and no counter.
    Missing,
    /// Frame index.
    Frames(u32),
    /// Relative error, fade deviation, pop, or flicker.
    Ratio(f64),
    /// Sample bytes.
    Bytes(u64),
    /// Mismatched reads.
    Count(u32),
}

/// Row outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// At or under the threshold.
    Pass,
    /// Over the threshold, or the series never settled.
    Fail,
    /// The engine does not produce this input yet.
    NotImplemented,
}

impl Status {
    /// Token printed in the summary and the JSON.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::NotImplemented => "NOT-IMPLEMENTED",
        }
    }
}

/// One case, ready to print and write.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    /// `good`, `broken`, `instant`, `noisy`, or `baseline`.
    pub case_name: &'static str,
    /// `fixture`, `capture`, or `absent`.
    pub source: &'static str,
    /// Rows in print order.
    pub metrics: Vec<Metric>,
}

impl Report {
    /// 1 when any row failed. `NOT-IMPLEMENTED` is not a failure.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        i32::from(
            self.metrics
                .iter()
                .any(|metric| metric.status == Status::Fail),
        )
    }
}

/// Grade one planted case.
///
/// # Errors
///
/// Planted pictures that disagree in size.
#[must_use = "the report is the grade"]
pub fn evaluate(case: Case) -> Result<Report, String> {
    match case {
        Case::Baseline => Ok(baseline_report()),
        Case::Good => grade(
            case,
            &fade_series(),
            &grey(0.5),
            &grey(0.5),
            GOOD_BYTES,
            &good_reads(),
        ),
        Case::Broken => grade(
            case,
            &broken_series(),
            &grey(1.0),
            &grey(0.5),
            BROKEN_BYTES,
            &broken_reads(),
        ),
        Case::Instant => grade(
            case,
            &instant_series(),
            &grey(0.5),
            &grey(0.5),
            GOOD_BYTES,
            &good_reads(),
        ),
        Case::Noisy => grade(
            case,
            &noisy_series(),
            &grey(0.5),
            &grey(0.5),
            GOOD_BYTES,
            &good_reads(),
        ),
    }
}

/// Grade `frame-NNNN.png` files written by the engine.
///
/// `DIR` or `DIR/frames` holds `frame-0000.png` upward, with no gaps.
/// `reference.png` beside those frames scores `dim_error`. Without it the
/// settle clock uses the mean of the last four frames, and `dim_error` stays
/// `NOT-IMPLEMENTED`. Layer rows stay `NOT-IMPLEMENTED`.
///
/// # Errors
///
/// The directory is missing, a PNG does not load, or the frames disagree in size.
#[must_use = "the report is the grade"]
pub fn evaluate_capture(dir: &Path) -> Result<Report, String> {
    let frame_dir = locate_frames(dir)?;
    let frames = load_frames(&frame_dir)?;
    let reference = load_reference(&frame_dir)?;
    let (reference, score_dim, settle_detail) = match reference {
        Some(picture) => (picture, true, None),
        None => (
            tail_average(&frames)?,
            false,
            Some("held end, not a path trace"),
        ),
    };
    let series = Series { reference, frames };
    let mut metrics = picture_metrics(&series, score_dim, settle_detail)?;
    metrics.push(missing(
        "layer_same_frame",
        MetricValue::Ratio(SETTLE_LINE),
        "no lamp layer on the lamp",
    ));
    metrics.push(missing(
        "layer_bytes",
        MetricValue::Bytes(HIGH_LAYER_BYTES),
        "no lamp-layer pool",
    ));
    metrics.push(missing(
        "finished_read",
        MetricValue::Count(FINISHED_LIMIT),
        "no publish counter",
    ));
    Ok(Report {
        case_name: Case::Baseline.name(),
        source: "capture",
        metrics,
    })
}

/// `name value threshold STATUS`
#[must_use]
pub fn summary_line(metric: &Metric) -> String {
    format!(
        "{} {} {} {}",
        metric.name,
        token(&metric.value),
        token(&metric.threshold),
        metric.status.as_str()
    )
}

/// Pretty JSON for `{case}.json`.
#[must_use]
pub fn to_json(report: &Report) -> String {
    let mut body = String::from("{\n");
    body.push_str(&format!("  \"case\": \"{}\",\n", report.case_name));
    body.push_str(&format!("  \"source\": \"{}\",\n", report.source));
    body.push_str("  \"preset\": \"high\",\n");
    body.push_str("  \"metrics\": [\n");
    for (index, metric) in report.metrics.iter().enumerate() {
        let comma = if index + 1 == report.metrics.len() {
            ""
        } else {
            ","
        };
        body.push_str("    {");
        body.push_str(&format!("\"name\": \"{}\"", metric.name));
        body.push_str(&format!(", \"value\": {}", value_json(&metric.value)));
        body.push_str(&format!(
            ", \"threshold\": {}",
            value_json(&metric.threshold)
        ));
        body.push_str(&format!(", \"status\": \"{}\"", metric.status.as_str()));
        if let Some(detail) = metric.detail {
            body.push_str(&format!(", \"detail\": \"{detail}\""));
        }
        body.push_str(&format!("}}{comma}\n"));
    }
    body.push_str("  ]\n}\n");
    body
}

/// Write `{dir}/{case}.json`. Creates `dir`.
///
/// # Errors
///
/// Directory creation or the write failed.
pub fn write_report(dir: &Path, report: &Report) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|err| err.to_string())?;
    let path = dir.join(format!("{}.json", report.case_name));
    fs::write(&path, to_json(report)).map_err(|err| err.to_string())?;
    Ok(path)
}

struct Series {
    reference: Picture,
    frames: Vec<Picture>,
}

struct ProbeRead {
    published: f32,
    observed: f32,
}

fn grade(
    case: Case,
    series: &Series,
    layer_frame: &Picture,
    layer_reference: &Picture,
    layer_bytes: u64,
    reads: &[ProbeRead],
) -> Result<Report, String> {
    let mut metrics = picture_metrics(series, true, None)?;
    let same = blur_relative(layer_reference, layer_frame, &MetricParams::default())?;
    metrics.push(judge_ratio("layer_same_frame", same, SETTLE_LINE));
    metrics.push(judge_bytes(layer_bytes));
    metrics.push(judge_count(mismatch_count(reads)));
    Ok(Report {
        case_name: case.name(),
        source: "fixture",
        metrics,
    })
}

fn picture_metrics(
    series: &Series,
    score_dim: bool,
    settle_detail: Option<&'static str>,
) -> Result<Vec<Metric>, String> {
    same_series(series)?;
    let settled_at = settle_index(series)?;
    let observed = u32::try_from(series.frames.len()).map_err(|_| "too many frames".to_string())?;
    let asymptote = mean_luma(&series.reference)?;
    let curve = fade_curve(&series.frames, asymptote)?;
    let origin = curve.origin;
    let settled = settled_at.map(|frame| frame.saturating_sub(origin));
    let observed = observed.saturating_sub(origin);
    let flicker = hold_flicker(&series.frames, settled_at)?;
    let mut metrics = vec![
        judge_settle(settled, observed, settle_detail),
        judge_ratio("pop_magnitude", pop_magnitude(series)?, POP_LIMIT),
        judge_fade(curve.deviation),
        judge_ratio("hold_flicker", flicker, HOLD_FLICKER_LIMIT),
    ];
    if score_dim {
        metrics.push(judge_ratio("dim_error", dim_error(series)?, SETTLE_LINE));
    } else {
        metrics.push(missing(
            "dim_error",
            MetricValue::Ratio(SETTLE_LINE),
            "no path-traced compare",
        ));
    }
    Ok(metrics)
}

fn baseline_report() -> Report {
    Report {
        case_name: Case::Baseline.name(),
        source: "absent",
        metrics: vec![
            missing(
                "toggle_settle_frames",
                MetricValue::Frames(HIGH_SETTLE_FRAMES),
                "no lamp-toggle capture",
            ),
            missing(
                "pop_magnitude",
                MetricValue::Ratio(POP_LIMIT),
                "no lamp-toggle capture",
            ),
            missing(
                "fade_deviation",
                MetricValue::Ratio(FADE_LIMIT),
                "no lamp-toggle capture",
            ),
            missing(
                "hold_flicker",
                MetricValue::Ratio(HOLD_FLICKER_LIMIT),
                "no lamp-toggle capture",
            ),
            missing(
                "dim_error",
                MetricValue::Ratio(SETTLE_LINE),
                "no path-traced compare",
            ),
            missing(
                "layer_same_frame",
                MetricValue::Ratio(SETTLE_LINE),
                "no lamp layer on the lamp",
            ),
            missing(
                "layer_bytes",
                MetricValue::Bytes(HIGH_LAYER_BYTES),
                "no lamp-layer pool",
            ),
            missing(
                "finished_read",
                MetricValue::Count(FINISHED_LIMIT),
                "no publish counter",
            ),
        ],
    }
}

fn missing(name: &'static str, threshold: MetricValue, detail: &'static str) -> Metric {
    Metric {
        name,
        value: MetricValue::Missing,
        threshold,
        status: Status::NotImplemented,
        detail: Some(detail),
    }
}

fn judge_settle(settled_at: Option<u32>, observed: u32, detail: Option<&'static str>) -> Metric {
    let (value, status) = match settled_at {
        Some(frame) if frame <= HIGH_SETTLE_FRAMES => (frame, Status::Pass),
        Some(frame) => (frame, Status::Fail),
        None => (observed, Status::Fail),
    };
    Metric {
        name: "toggle_settle_frames",
        value: MetricValue::Frames(value),
        threshold: MetricValue::Frames(HIGH_SETTLE_FRAMES),
        status,
        detail,
    }
}

fn judge_ratio(name: &'static str, value: f64, threshold: f64) -> Metric {
    let status = if value.is_finite() && value <= threshold {
        Status::Pass
    } else {
        Status::Fail
    };
    Metric {
        name,
        value: MetricValue::Ratio(value),
        threshold: MetricValue::Ratio(threshold),
        status,
        detail: None,
    }
}

fn judge_fade(value: Option<f64>) -> Metric {
    match value {
        Some(value) => judge_ratio("fade_deviation", value, FADE_LIMIT),
        None => missing(
            "fade_deviation",
            MetricValue::Ratio(FADE_LIMIT),
            "mean luma did not move",
        ),
    }
}

fn judge_bytes(value: u64) -> Metric {
    Metric {
        name: "layer_bytes",
        value: MetricValue::Bytes(value),
        threshold: MetricValue::Bytes(HIGH_LAYER_BYTES),
        status: if value <= HIGH_LAYER_BYTES {
            Status::Pass
        } else {
            Status::Fail
        },
        detail: None,
    }
}

fn judge_count(value: u32) -> Metric {
    Metric {
        name: "finished_read",
        value: MetricValue::Count(value),
        threshold: MetricValue::Count(FINISHED_LIMIT),
        status: if value == FINISHED_LIMIT {
            Status::Pass
        } else {
            Status::Fail
        },
        detail: None,
    }
}

fn settle_index(series: &Series) -> Result<Option<u32>, String> {
    let len = series.frames.len();
    if len < HOLD_FRAMES {
        return Ok(None);
    }
    let last = len - HOLD_FRAMES;
    for start in 0..=last {
        if window_settled(series, start)? {
            let frame = u32::try_from(start).map_err(|_| "frame index overflow".to_string())?;
            return Ok(Some(frame));
        }
    }
    Ok(None)
}

fn window_settled(series: &Series, start: usize) -> Result<bool, String> {
    for offset in 0..HOLD_FRAMES {
        let error = blur_relative(
            &series.reference,
            &series.frames[start + offset],
            &MetricParams::default(),
        )?;
        if error > SETTLE_LINE {
            return Ok(false);
        }
    }
    Ok(true)
}

fn pop_magnitude(series: &Series) -> Result<f64, String> {
    if series.frames.len() < 2 {
        return Err("pop needs two frames".to_string());
    }
    let floor = MetricParams::default().rel_floor;
    let mut worst = 0.0_f64;
    for pair in series.frames.windows(2) {
        let prev = &pair[0];
        let next = &pair[1];
        if prev.width != next.width || prev.height != next.height {
            return Err("pop frames differ in size".to_string());
        }
        let count = u32::try_from(prev.pixels()).map_err(|_| "picture too large".to_string())?;
        if count == 0 {
            return Err("pop picture is empty".to_string());
        }
        let mut sum = 0.0_f64;
        let mut peak = 0.0_f64;
        for index in 0..prev.pixels() {
            let before = f64::from(prev.luma_at(index));
            let after = f64::from(next.luma_at(index));
            let step = (after - before).abs() / before.max(floor);
            sum += step;
            if step > peak {
                peak = step;
            }
        }
        let excess = (peak - (sum / f64::from(count))).max(0.0);
        if excess > worst {
            worst = excess;
        }
    }
    Ok(worst)
}

struct FadeCurve {
    origin: u32,
    deviation: Option<f64>,
}

/// Mean-luma step against `end + (start − end) · exp(−t / τ)`.
///
/// `t` is zero on the frame before the largest mean step, so one frame of
/// command delay does not slide the curve. The score is the max
/// `|Δmean − Δcurve| / |start − end|`.
fn fade_curve(frames: &[Picture], asymptote: f64) -> Result<FadeCurve, String> {
    if frames.len() < 2 {
        return Err("fade needs two frames".to_string());
    }
    let mut means = Vec::with_capacity(frames.len());
    for frame in frames {
        means.push(mean_luma(frame)?);
    }
    let origin = largest_step(&means);
    let start = means[origin];
    let span = start - asymptote;
    if span.abs() < SPAN_FLOOR {
        let origin = u32::try_from(origin).map_err(|_| "frame index overflow".to_string())?;
        return Ok(FadeCurve {
            origin,
            deviation: None,
        });
    }
    let decay = (-FRAME_DT / TAU_SECONDS).exp();
    let mut weight = 1.0_f64;
    let mut worst = 0.0_f64;
    for index in (origin + 1)..means.len() {
        let observed = means[index] - means[index - 1];
        let expected = span * weight * (decay - 1.0);
        let deviation = (observed - expected).abs() / span.abs();
        if deviation > worst {
            worst = deviation;
        }
        weight *= decay;
    }
    let origin = u32::try_from(origin).map_err(|_| "frame index overflow".to_string())?;
    Ok(FadeCurve {
        origin,
        deviation: Some(worst),
    })
}

fn largest_step(means: &[f64]) -> usize {
    let mut at = 0_usize;
    let mut peak = 0.0_f64;
    for index in 1..means.len() {
        let step = (means[index] - means[index - 1]).abs();
        if step > peak {
            peak = step;
            at = index - 1;
        }
    }
    at
}

fn hold_flicker(frames: &[Picture], settled_at: Option<u32>) -> Result<f64, String> {
    temporal_std_p99(hold_window(frames, settled_at)?)
}

fn hold_window(frames: &[Picture], settled_at: Option<u32>) -> Result<&[Picture], String> {
    if frames.len() < 2 {
        return Err("flicker needs two frames".to_string());
    }
    let start = match settled_at {
        Some(frame) => usize::try_from(frame).map_err(|_| "frame index overflow".to_string())?,
        None => frames.len().saturating_sub(HOLD_FRAMES),
    };
    let start = start.min(frames.len() - 2);
    Ok(&frames[start..])
}

/// Population standard deviation per pixel, then the nearest-rank 99th percentile.
fn temporal_std_p99(frames: &[Picture]) -> Result<f64, String> {
    let first = frames
        .first()
        .ok_or_else(|| "flicker needs two frames".to_string())?;
    let count = u32::try_from(first.pixels()).map_err(|_| "picture too large".to_string())?;
    if count == 0 {
        return Err("flicker picture is empty".to_string());
    }
    let n_frames = u32::try_from(frames.len()).map_err(|_| "too many frames".to_string())?;
    let n_frames_f = f64::from(n_frames);
    let mut stds = Vec::with_capacity(first.pixels());
    for index in 0..first.pixels() {
        let mut mean = 0.0_f64;
        for frame in frames {
            if frame.width != first.width || frame.height != first.height {
                return Err("flicker frames differ in size".to_string());
            }
            mean += f64::from(frame.luma_at(index));
        }
        mean /= n_frames_f;
        let mut var = 0.0_f64;
        for frame in frames {
            let delta = f64::from(frame.luma_at(index)) - mean;
            var += delta * delta;
        }
        var /= n_frames_f;
        stds.push(var.sqrt());
    }
    Ok(worst_percent(&mut stds))
}

fn worst_percent(values: &mut [f64]) -> f64 {
    let n = values.len();
    if n == 0 {
        return 0.0;
    }
    values.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let rank = n.saturating_mul(99).saturating_add(99) / 100;
    let index = rank.saturating_sub(1).min(n - 1);
    values[index]
}

fn dim_error(series: &Series) -> Result<f64, String> {
    let last = series
        .frames
        .last()
        .ok_or_else(|| "dim needs a frame".to_string())?;
    blur_relative(&series.reference, last, &MetricParams::default())
}

fn mean_luma(picture: &Picture) -> Result<f64, String> {
    let count = u32::try_from(picture.pixels()).map_err(|_| "picture too large".to_string())?;
    if count == 0 {
        return Err("picture is empty".to_string());
    }
    let mut sum = 0.0_f64;
    for index in 0..picture.pixels() {
        sum += f64::from(picture.luma_at(index));
    }
    Ok(sum / f64::from(count))
}

fn same_series(series: &Series) -> Result<(), String> {
    if series.frames.is_empty() {
        return Err("series has no frames".to_string());
    }
    for frame in &series.frames {
        if frame.width != series.reference.width || frame.height != series.reference.height {
            return Err("frames differ in size".to_string());
        }
    }
    Ok(())
}

fn mismatch_count(reads: &[ProbeRead]) -> u32 {
    let mut count = 0_u32;
    for read in reads {
        let delta = f64::from(read.observed) - f64::from(read.published);
        if delta.abs() > READ_EPS {
            count = count.saturating_add(1);
        }
    }
    count
}

fn token(value: &MetricValue) -> String {
    match value {
        MetricValue::Missing => "NOT-IMPLEMENTED".to_string(),
        MetricValue::Frames(n) | MetricValue::Count(n) => n.to_string(),
        MetricValue::Bytes(n) => n.to_string(),
        MetricValue::Ratio(n) => format!("{n:.4}"),
    }
}

fn value_json(value: &MetricValue) -> String {
    match value {
        MetricValue::Missing => "null".to_string(),
        MetricValue::Frames(_)
        | MetricValue::Ratio(_)
        | MetricValue::Bytes(_)
        | MetricValue::Count(_) => token(value),
    }
}

fn grey(y: f32) -> Picture {
    Picture::solid(TILE, TILE, [y, y, y])
}

fn tail_average(frames: &[Picture]) -> Result<Picture, String> {
    if frames.is_empty() {
        return Err("capture has no frames".to_string());
    }
    let start = frames.len().saturating_sub(HOLD_FRAMES);
    Picture::average(&frames[start..])
}

/// Unit white to half intensity. τ = 50 ms, step = 1/60 s.
fn fade_series() -> Series {
    let decay = (-(1.0_f32 / 60.0) / 0.050).exp();
    let mut remain = 1.0_f32;
    let mut frames = Vec::new();
    for _ in 0..17 {
        frames.push(grey(0.5 + 0.5 * remain));
        remain *= decay;
    }
    Series {
        reference: grey(0.5),
        frames,
    }
}

/// One frame jumps the whole picture from 1 to 0.5 and then holds.
fn instant_series() -> Series {
    let mut frames = vec![grey(1.0)];
    for _ in 0..8 {
        frames.push(grey(0.5));
    }
    Series {
        reference: grey(0.5),
        frames,
    }
}

/// The picture matches the reference, except one pixel alternates.
fn noisy_series() -> Series {
    let mut frames = Vec::new();
    for index in 0..16_u32 {
        let mut frame = grey(0.5);
        let y = if index % 2 == 0 { 0.0 } else { 1.0 };
        frame.set_pixel(0, 0, [y, y, y]);
        frames.push(frame);
    }
    Series {
        reference: grey(0.5),
        frames,
    }
}

/// A speck flips in a picture that never reaches the reference.
fn broken_series() -> Series {
    let mut frames = Vec::new();
    for index in 0..20_u32 {
        let mut frame = grey(1.0);
        if index % 2 == 0 {
            frame.set_pixel(0, 0, [0.0, 0.0, 0.0]);
        }
        frames.push(frame);
    }
    Series {
        reference: grey(0.5),
        frames,
    }
}

fn good_reads() -> Vec<ProbeRead> {
    vec![
        ProbeRead {
            published: 0.5,
            observed: 0.5,
        },
        ProbeRead {
            published: 0.25,
            observed: 0.25,
        },
    ]
}

fn broken_reads() -> Vec<ProbeRead> {
    vec![ProbeRead {
        published: 0.5,
        observed: 0.9,
    }]
}

fn locate_frames(dir: &Path) -> Result<PathBuf, String> {
    if !dir.is_dir() {
        return Err(format!("{}: not a directory", dir.display()));
    }
    if dir_has_frame(dir)? {
        return Ok(dir.to_path_buf());
    }
    let nested = dir.join("frames");
    if nested.is_dir() && dir_has_frame(&nested)? {
        return Ok(nested);
    }
    Err(format!("no frame-*.png in {}", dir.display()))
}

fn dir_has_frame(dir: &Path) -> Result<bool, String> {
    let entries = fs::read_dir(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| err.to_string())?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if frame_index(&name).is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn load_frames(dir: &Path) -> Result<Vec<Picture>, String> {
    let mut indexed = Vec::new();
    let entries = fs::read_dir(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| err.to_string())?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Some(index) = frame_index(&name) else {
            continue;
        };
        indexed.push((index, entry.path()));
    }
    if indexed.is_empty() {
        return Err(format!("no frame-*.png in {}", dir.display()));
    }
    indexed.sort_by_key(|(index, _)| *index);
    let mut frames = Vec::with_capacity(indexed.len());
    for (expected, (index, path)) in indexed.iter().enumerate() {
        let expected_u = u32::try_from(expected).map_err(|_| "too many frames".to_string())?;
        if *index != expected_u {
            return Err(format!(
                "frame numbers skip {expected_u} in {} (saw {index})",
                dir.display()
            ));
        }
        frames.push(Picture::load_png(path)?);
    }
    Ok(frames)
}

fn frame_index(name: &str) -> Option<u32> {
    let rest = name.strip_prefix("frame-")?.strip_suffix(".png")?;
    if rest.is_empty() || !rest.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    rest.parse().ok()
}

fn load_reference(frame_dir: &Path) -> Result<Option<Picture>, String> {
    let mut paths = vec![frame_dir.join("reference.png")];
    if let Some(parent) = frame_dir.parent() {
        paths.push(parent.join("reference.png"));
    }
    for path in paths {
        if path.is_file() {
            return Ok(Some(Picture::load_png(&path)?));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::{
        evaluate, evaluate_capture, fade_series, summary_line, write_report, Case, Metric,
        MetricValue, Report, Status,
    };

    fn graded(case: Case) -> Report {
        match evaluate(case) {
            Ok(report) => report,
            Err(err) => panic!("evaluate: {err}"),
        }
    }

    fn named<'a>(report: &'a Report, name: &str) -> &'a Metric {
        report
            .metrics
            .iter()
            .find(|metric| metric.name == name)
            .unwrap_or_else(|| panic!("missing {name}"))
    }

    fn ratio(metric: &Metric) -> f64 {
        match metric.value {
            MetricValue::Ratio(value) => value,
            MetricValue::Missing
            | MetricValue::Frames(_)
            | MetricValue::Bytes(_)
            | MetricValue::Count(_) => {
                panic!("not a ratio: {metric:?}")
            }
        }
    }

    #[test]
    fn good_fixture_passes_every_check() {
        let report = graded(Case::Good);
        assert_eq!(report.exit_code(), 0);
        assert_eq!(report.metrics.len(), 8);
        for metric in &report.metrics {
            assert_eq!(metric.status, Status::Pass, "{}", metric.name);
            assert_ne!(metric.value, MetricValue::Missing, "{}", metric.name);
        }
    }

    #[test]
    fn broken_fixture_fails_every_check() {
        let report = graded(Case::Broken);
        assert_eq!(report.exit_code(), 1);
        assert_eq!(report.metrics.len(), 8);
        for metric in &report.metrics {
            assert_eq!(metric.status, Status::Fail, "{}", summary_line(metric));
            assert_ne!(metric.value, MetricValue::Missing, "{}", metric.name);
        }
    }

    #[test]
    fn instant_toggle_fails_the_fade_curve() {
        let report = graded(Case::Instant);
        assert_eq!(report.exit_code(), 1);
        let pop = named(&report, "pop_magnitude");
        assert_eq!(pop.status, Status::Pass, "{}", summary_line(pop));
        assert!(ratio(pop) < 1.0e-4, "{}", summary_line(pop));
        let fade = named(&report, "fade_deviation");
        assert_eq!(fade.status, Status::Fail, "{}", summary_line(fade));
        assert!(ratio(fade) > 0.5, "{}", summary_line(fade));
    }

    #[test]
    fn noisy_hold_fails_flicker() {
        let report = graded(Case::Noisy);
        assert_eq!(report.exit_code(), 1);
        let flicker = named(&report, "hold_flicker");
        assert_eq!(flicker.status, Status::Fail, "{}", summary_line(flicker));
        assert!(ratio(flicker) > 0.2, "{}", summary_line(flicker));
    }

    #[test]
    fn baseline_marks_every_row_not_implemented() {
        let report = graded(Case::Baseline);
        assert_eq!(report.exit_code(), 0);
        assert_eq!(report.source, "absent");
        assert_eq!(report.metrics.len(), 8);
        for metric in &report.metrics {
            assert_eq!(metric.status, Status::NotImplemented, "{}", metric.name);
            assert_eq!(metric.value, MetricValue::Missing, "{}", metric.name);
            assert!(metric.detail.is_some(), "{}", metric.name);
        }
    }

    #[test]
    fn summary_line_is_four_fields() {
        for case in [
            Case::Good,
            Case::Broken,
            Case::Instant,
            Case::Noisy,
            Case::Baseline,
        ] {
            for metric in &graded(case).metrics {
                let line = summary_line(metric);
                assert_eq!(line.split(' ').count(), 4, "{line}");
            }
        }
        let baseline = graded(Case::Baseline);
        let first = summary_line(&baseline.metrics[0]);
        assert_eq!(
            first,
            "toggle_settle_frames NOT-IMPLEMENTED 12 NOT-IMPLEMENTED"
        );
    }

    #[test]
    fn json_keeps_pass_and_fail_apart() {
        let dir = std::env::temp_dir().join("genos-layering-gate-test");
        let good = graded(Case::Good);
        let broken = graded(Case::Broken);
        let good_path = write_report(&dir, &good).unwrap();
        let broken_path = write_report(&dir, &broken).unwrap();
        let good_text = std::fs::read_to_string(good_path).unwrap();
        let broken_text = std::fs::read_to_string(broken_path).unwrap();
        assert!(good_text.contains("\"status\": \"PASS\""));
        assert!(!good_text.contains("\"status\": \"FAIL\""));
        assert!(broken_text.contains("\"status\": \"FAIL\""));
        assert!(!broken_text.contains("\"status\": \"PASS\""));
        assert!(!broken_text.contains("\"value\": null"));
    }

    #[test]
    fn capture_of_the_fade_fills_picture_rows() {
        let dir = std::env::temp_dir().join(format!("genos-layering-cap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let series = fade_series();
        for (index, frame) in series.frames.iter().enumerate() {
            let name = format!("frame-{index:04}.png");
            frame.save_png(&dir.join(&name)).unwrap();
        }
        let report = match evaluate_capture(&dir) {
            Ok(report) => report,
            Err(err) => panic!("capture: {err}"),
        };
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(report.source, "capture");
        assert_eq!(report.case_name, "baseline");
        for name in [
            "toggle_settle_frames",
            "pop_magnitude",
            "fade_deviation",
            "hold_flicker",
        ] {
            let metric = named(&report, name);
            assert_eq!(metric.status, Status::Pass, "{}", summary_line(metric));
        }
        for name in [
            "dim_error",
            "layer_same_frame",
            "layer_bytes",
            "finished_read",
        ] {
            let metric = named(&report, name);
            assert_eq!(
                metric.status,
                Status::NotImplemented,
                "{}",
                summary_line(metric)
            );
        }
        assert_eq!(report.exit_code(), 0);
    }
}
