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
//! Each frame is scored against the image for that frame's requested lamp
//! weight. Planted pictures prove each comparison both ways. `--capture`
//! reads `frame-NNNN.png`, `weights.txt`, and the on/off expected images.
//! Without that directory, `--case baseline` prints `NOT-IMPLEMENTED`.

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

/// Max relative mean-luma miss against the requested-weight image.
/// `|mean(Y) − mean(Yexpected)| / max(mean(Yexpected), 0.02)`.
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

const HOLD_FRAMES: usize = 4;
const READ_EPS: f64 = 1.0e-3;
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
    /// A step was requested. The picture instead fades with τ = 50 ms.
    /// `pop_magnitude` passes. `fade_deviation` fails.
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
        Case::Good => {
            let (frames, expected) = matched_step()?;
            grade(
                case,
                &frames,
                &expected,
                &grey(0.5),
                &grey(0.5),
                GOOD_BYTES,
                &good_reads(),
            )
        }
        Case::Broken => {
            let (frames, expected) = broken_pair()?;
            grade(
                case,
                &frames,
                &expected,
                &grey(1.0),
                &grey(0.5),
                BROKEN_BYTES,
                &broken_reads(),
            )
        }
        Case::Instant => {
            let (frames, expected) = lagging_step()?;
            grade(
                case,
                &frames,
                &expected,
                &grey(0.5),
                &grey(0.5),
                GOOD_BYTES,
                &good_reads(),
            )
        }
        Case::Noisy => {
            let (frames, expected) = noisy_pair()?;
            grade(
                case,
                &frames,
                &expected,
                &grey(0.5),
                &grey(0.5),
                GOOD_BYTES,
                &good_reads(),
            )
        }
    }
}

/// Grade `frame-NNNN.png` files written by the engine.
///
/// `DIR` or `DIR/frames` holds `frame-0000.png` upward, with no gaps.
/// `weights.txt` is one requested lamp weight per frame. `expected-on.png`
/// and `expected-off.png` (or the `expected-*-reference.png` names the
/// reference command writes) are the path-traced or analytic endpoints.
/// The expected image is `off + weight · (on − off)`. Without those files
/// the rows that need a target stay `NOT-IMPLEMENTED`. Layer rows stay
/// `NOT-IMPLEMENTED`.
///
/// # Errors
///
/// The directory is missing, a PNG does not load, the frames disagree in
/// size, or the weights and the endpoints disagree about whether a target
/// was supplied.
#[must_use = "the report is the grade"]
pub fn evaluate_capture(dir: &Path) -> Result<Report, String> {
    let frame_dir = locate_frames(dir)?;
    let frames = load_frames(&frame_dir)?;
    let weights = load_weights(&frame_dir)?;
    let endpoints = load_endpoints(&frame_dir)?;
    let mut metrics = match (weights, endpoints) {
        (Some(weights), Some((on, off))) => {
            if weights.len() != frames.len() {
                return Err(format!(
                    "weights.txt has {} rows, {} frames",
                    weights.len(),
                    frames.len()
                ));
            }
            let expected = images_at(&weights, &on, &off)?;
            picture_metrics(&frames, &expected)?
        }
        (None, None) => frames_only(&frames)?,
        (Some(_), None) => {
            return Err(
                "weights.txt is present but expected-on.png and expected-off.png are not"
                    .to_string(),
            );
        }
        (None, Some(_)) => {
            return Err("expected images are present but weights.txt is not".to_string());
        }
    };
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

struct ProbeRead {
    published: f32,
    observed: f32,
}

fn grade(
    case: Case,
    frames: &[Picture],
    expected: &[Picture],
    layer_frame: &Picture,
    layer_reference: &Picture,
    layer_bytes: u64,
    reads: &[ProbeRead],
) -> Result<Report, String> {
    let mut metrics = picture_metrics(frames, expected)?;
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

fn picture_metrics(frames: &[Picture], expected: &[Picture]) -> Result<Vec<Metric>, String> {
    same_pictures(frames, expected)?;
    let settled_at = settle_index(frames, expected)?;
    let observed = u32::try_from(frames.len()).map_err(|_| "too many frames".to_string())?;
    let held_from = held_request_index(expected)?;
    let flicker = hold_flicker(frames, Some(held_from), settled_at)?;
    Ok(vec![
        judge_settle(settled_at, observed, None),
        judge_ratio("pop_magnitude", pop_magnitude(frames)?, POP_LIMIT),
        judge_ratio(
            "fade_deviation",
            fade_deviation(frames, expected)?,
            FADE_LIMIT,
        ),
        judge_ratio("hold_flicker", flicker, HOLD_FLICKER_LIMIT),
        judge_ratio("dim_error", dim_error(frames, expected)?, SETTLE_LINE),
    ])
}

fn frames_only(frames: &[Picture]) -> Result<Vec<Metric>, String> {
    if frames.is_empty() {
        return Err("capture has no frames".to_string());
    }
    Ok(vec![
        missing(
            "toggle_settle_frames",
            MetricValue::Frames(HIGH_SETTLE_FRAMES),
            "no requested-weight reference",
        ),
        judge_ratio("pop_magnitude", pop_magnitude(frames)?, POP_LIMIT),
        missing(
            "fade_deviation",
            MetricValue::Ratio(FADE_LIMIT),
            "no requested-weight reference",
        ),
        judge_ratio(
            "hold_flicker",
            hold_flicker(frames, None, None)?,
            HOLD_FLICKER_LIMIT,
        ),
        missing(
            "dim_error",
            MetricValue::Ratio(SETTLE_LINE),
            "no requested-weight reference",
        ),
    ])
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

fn settle_index(frames: &[Picture], expected: &[Picture]) -> Result<Option<u32>, String> {
    let len = frames.len();
    if len < HOLD_FRAMES {
        return Ok(None);
    }
    let last = len - HOLD_FRAMES;
    for start in 0..=last {
        if window_settled(frames, expected, start)? {
            let frame = u32::try_from(start).map_err(|_| "frame index overflow".to_string())?;
            return Ok(Some(frame));
        }
    }
    Ok(None)
}

fn window_settled(frames: &[Picture], expected: &[Picture], start: usize) -> Result<bool, String> {
    for offset in 0..HOLD_FRAMES {
        let error = blur_relative(
            &expected[start + offset],
            &frames[start + offset],
            &MetricParams::default(),
        )?;
        if error > SETTLE_LINE {
            return Ok(false);
        }
    }
    Ok(true)
}

fn pop_magnitude(frames: &[Picture]) -> Result<f64, String> {
    if frames.len() < 2 {
        return Err("pop needs two frames".to_string());
    }
    let floor = MetricParams::default().rel_floor;
    let mut worst = 0.0_f64;
    for pair in frames.windows(2) {
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

/// Max `|mean(Y) − mean(Yexpected)| / max(mean(Yexpected), 0.02)`.
///
/// The denominator is the expected mean, floored like the settle line. It is
/// not the net change of the capture, which shrinks when a picture overshoots
/// and used to push the score off the luma scale.
fn fade_deviation(frames: &[Picture], expected: &[Picture]) -> Result<f64, String> {
    if frames.is_empty() {
        return Err("fade needs a frame".to_string());
    }
    let floor = MetricParams::default().rel_floor;
    let mut worst = 0.0_f64;
    for (frame, want) in frames.iter().zip(expected) {
        let seen = mean_luma(frame)?;
        let asked = mean_luma(want)?;
        let miss = (seen - asked).abs() / asked.max(floor);
        if miss > worst {
            worst = miss;
        }
    }
    Ok(worst)
}

fn hold_flicker(
    frames: &[Picture],
    held_from: Option<usize>,
    settled_at: Option<u32>,
) -> Result<f64, String> {
    temporal_std_p99(hold_window(frames, held_from, settled_at)?)
}

fn hold_window(
    frames: &[Picture],
    held_from: Option<usize>,
    settled_at: Option<u32>,
) -> Result<&[Picture], String> {
    if frames.len() < 2 {
        return Err("flicker needs two frames".to_string());
    }
    let start = match settled_at {
        Some(frame) => {
            let settled = usize::try_from(frame).map_err(|_| "frame index overflow".to_string())?;
            settled.max(held_from.unwrap_or(0))
        }
        None => frames.len().saturating_sub(HOLD_FRAMES),
    };
    let start = start.min(frames.len() - 2);
    Ok(&frames[start..])
}

/// First frame whose requested image matches the last frame's request.
fn held_request_index(expected: &[Picture]) -> Result<usize, String> {
    let last = expected
        .last()
        .ok_or_else(|| "no expected frame".to_string())?;
    let mut index = expected.len() - 1;
    while index > 0 {
        let error = blur_relative(&expected[index - 1], last, &MetricParams::default())?;
        if error > SETTLE_LINE {
            break;
        }
        index -= 1;
    }
    Ok(index)
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

fn dim_error(frames: &[Picture], expected: &[Picture]) -> Result<f64, String> {
    let last = frames
        .last()
        .ok_or_else(|| "dim needs a frame".to_string())?;
    let want = expected
        .last()
        .ok_or_else(|| "dim needs an expected frame".to_string())?;
    blur_relative(want, last, &MetricParams::default())
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

fn same_pictures(frames: &[Picture], expected: &[Picture]) -> Result<(), String> {
    if frames.is_empty() {
        return Err("series has no frames".to_string());
    }
    if frames.len() != expected.len() {
        return Err("frame count does not match the requested weights".to_string());
    }
    let width = frames[0].width;
    let height = frames[0].height;
    for (frame, want) in frames.iter().zip(expected) {
        if frame.width != width
            || frame.height != height
            || want.width != width
            || want.height != height
        {
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

fn step_weights(count: usize) -> Vec<f32> {
    let mut weights = vec![0.0; count];
    if let Some(first) = weights.first_mut() {
        *first = 1.0;
    }
    weights
}

fn images_at(weights: &[f32], on: &Picture, off: &Picture) -> Result<Vec<Picture>, String> {
    if on.width != off.width || on.height != off.height {
        return Err("expected-on and expected-off differ in size".to_string());
    }
    let mut pictures = Vec::with_capacity(weights.len());
    for weight in weights {
        if !weight.is_finite() {
            return Err("weight is not finite".to_string());
        }
        pictures.push(lerp_picture(off, on, *weight)?);
    }
    Ok(pictures)
}

fn lerp_picture(off: &Picture, on: &Picture, weight: f32) -> Result<Picture, String> {
    let mut linear = Vec::with_capacity(off.linear.len());
    for (dark, lit) in off.linear.iter().zip(&on.linear) {
        linear.push(*dark + (*lit - *dark) * weight);
    }
    Picture::new(off.width, off.height, linear)
}

/// Requested step, and a picture that matches it on the same frame.
fn matched_step() -> Result<(Vec<Picture>, Vec<Picture>), String> {
    let expected = images_at(&step_weights(9), &grey(1.0), &grey(0.5))?;
    Ok((expected.clone(), expected))
}

/// Requested step. The picture instead fades with τ = 50 ms.
fn lagging_step() -> Result<(Vec<Picture>, Vec<Picture>), String> {
    let expected = images_at(&step_weights(17), &grey(1.0), &grey(0.5))?;
    let decay = (-(1.0_f32 / 60.0) / 0.050).exp();
    let mut remain = 1.0_f32;
    let mut frames = Vec::new();
    for _ in 0..expected.len() {
        frames.push(grey(0.5 + 0.5 * remain));
        remain *= decay;
    }
    Ok((frames, expected))
}

/// A speck flips in a picture that stays at the on image after a step request.
fn broken_pair() -> Result<(Vec<Picture>, Vec<Picture>), String> {
    let expected = images_at(&step_weights(20), &grey(1.0), &grey(0.5))?;
    let mut frames = Vec::new();
    for index in 0..expected.len() {
        let mut frame = grey(1.0);
        if index % 2 == 0 {
            frame.set_pixel(0, 0, [0.0, 0.0, 0.0]);
        }
        frames.push(frame);
    }
    Ok((frames, expected))
}

/// Steady request. One pixel alternates.
fn noisy_pair() -> Result<(Vec<Picture>, Vec<Picture>), String> {
    let expected = images_at(&[0.0; 16], &grey(1.0), &grey(0.5))?;
    let mut frames = Vec::new();
    for index in 0..expected.len() {
        let mut frame = grey(0.5);
        let y = if index % 2 == 0 { 0.0 } else { 1.0 };
        frame.set_pixel(0, 0, [y, y, y]);
        frames.push(frame);
    }
    Ok((frames, expected))
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

fn load_weights(frame_dir: &Path) -> Result<Option<Vec<f32>>, String> {
    let Some(path) = find_file(frame_dir, "weights.txt") else {
        return Ok(None);
    };
    let text = fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    let mut weights = Vec::new();
    for (line_no, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let weight: f32 = trimmed
            .parse()
            .map_err(|_| format!("{}:{}: not a weight", path.display(), line_no + 1))?;
        if !weight.is_finite() {
            return Err(format!(
                "{}:{}: weight is not finite",
                path.display(),
                line_no + 1
            ));
        }
        weights.push(weight);
    }
    if weights.is_empty() {
        return Err(format!("{}: no weights", path.display()));
    }
    Ok(Some(weights))
}

fn load_endpoints(frame_dir: &Path) -> Result<Option<(Picture, Picture)>, String> {
    let on = find_named(frame_dir, &["expected-on.png", "expected-on-reference.png"]);
    let off = find_named(
        frame_dir,
        &["expected-off.png", "expected-off-reference.png"],
    );
    match (on, off) {
        (None, None) => Ok(None),
        (Some(on_path), Some(off_path)) => Ok(Some((
            Picture::load_png(&on_path)?,
            Picture::load_png(&off_path)?,
        ))),
        _ => Err("expected-on and expected-off must both be present".to_string()),
    }
}

fn find_named(frame_dir: &Path, names: &[&str]) -> Option<PathBuf> {
    names.iter().find_map(|name| find_file(frame_dir, name))
}

fn find_file(frame_dir: &Path, name: &str) -> Option<PathBuf> {
    let direct = frame_dir.join(name);
    if direct.is_file() {
        return Some(direct);
    }
    frame_dir.parent().and_then(|parent| {
        let nested = parent.join(name);
        nested.is_file().then_some(nested)
    })
}

#[cfg(test)]
mod tests {
    use super::{
        evaluate, evaluate_capture, grey, images_at, picture_metrics, summary_line, write_report,
        Case, Metric, MetricValue, Report, Status,
    };

    fn graded(case: Case) -> Report {
        match evaluate(case) {
            Ok(report) => report,
            Err(err) => panic!("evaluate: {err}"),
        }
    }

    fn named_metrics<'a>(metrics: &'a [Metric], name: &str) -> &'a Metric {
        metrics
            .iter()
            .find(|metric| metric.name == name)
            .unwrap_or_else(|| panic!("missing {name}"))
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
    fn lagging_picture_fails_the_requested_step() {
        let report = graded(Case::Instant);
        assert_eq!(report.exit_code(), 1);
        let pop = named(&report, "pop_magnitude");
        assert_eq!(pop.status, Status::Pass, "{}", summary_line(pop));
        assert!(ratio(pop) < 1.0e-4, "{}", summary_line(pop));
        let settle = named(&report, "toggle_settle_frames");
        assert_eq!(settle.status, Status::Pass, "{}", summary_line(settle));
        let fade = named(&report, "fade_deviation");
        assert_eq!(fade.status, Status::Fail, "{}", summary_line(fade));
        assert!(ratio(fade) > 0.5, "{}", summary_line(fade));
        assert!(ratio(fade) < 2.0, "{}", summary_line(fade));
    }

    #[test]
    fn scripted_weights_are_the_target() {
        let weights = [1.0_f32, 0.75, 0.5, 0.25, 0.0, 0.0, 0.0, 0.0];
        let on = grey(0.5);
        let off = grey(0.25);
        let expected = images_at(&weights, &on, &off).unwrap();
        let matched = picture_metrics(&expected, &expected).unwrap();
        let fade = named_metrics(&matched, "fade_deviation");
        assert_eq!(fade.status, Status::Pass, "{}", summary_line(fade));
        assert!(ratio(fade) < 1.0e-4, "{}", summary_line(fade));

        let decay = (-(1.0_f32 / 60.0) / 0.050).exp();
        let mut remain = 1.0_f32;
        let mut frames = Vec::new();
        for _ in 0..weights.len() {
            frames.push(grey(0.25 + 0.25 * remain));
            remain *= decay;
        }
        let lagged = picture_metrics(&frames, &expected).unwrap();
        let miss = named_metrics(&lagged, "fade_deviation");
        assert_eq!(miss.status, Status::Fail, "{}", summary_line(miss));
        assert!(ratio(miss) > 0.05, "{}", summary_line(miss));
        assert!(ratio(miss) < 2.0, "{}", summary_line(miss));
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
    fn capture_of_a_matching_step_scores_the_requested_weight() {
        let dir =
            std::env::temp_dir().join(format!("genos-layering-cap-match-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let frames = dir.join("frames");
        std::fs::create_dir_all(&frames).unwrap();
        let on = grey(0.4);
        let off = grey(0.2);
        let weights = [1.0_f32, 0.0, 0.0, 0.0, 0.0, 0.0];
        let expected = images_at(&weights, &on, &off).unwrap();
        for (index, frame) in expected.iter().enumerate() {
            frame
                .save_png(&frames.join(format!("frame-{index:04}.png")))
                .unwrap();
        }
        on.save_png(&dir.join("expected-on.png")).unwrap();
        off.save_png(&dir.join("expected-off.png")).unwrap();
        let text = weights
            .iter()
            .map(f32::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(dir.join("weights.txt"), text + "\n").unwrap();
        let report = match evaluate_capture(&dir) {
            Ok(report) => report,
            Err(err) => panic!("capture: {err}"),
        };
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(report.source, "capture");
        for name in [
            "toggle_settle_frames",
            "pop_magnitude",
            "fade_deviation",
            "hold_flicker",
            "dim_error",
        ] {
            let metric = named(&report, name);
            assert_eq!(metric.status, Status::Pass, "{}", summary_line(metric));
        }
        assert_eq!(report.exit_code(), 0);
    }

    #[test]
    fn capture_without_a_target_leaves_the_picture_rows_open() {
        let dir = std::env::temp_dir().join(format!("genos-layering-cap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for index in 0..8 {
            grey(0.5)
                .save_png(&dir.join(format!("frame-{index:04}.png")))
                .unwrap();
        }
        let report = match evaluate_capture(&dir) {
            Ok(report) => report,
            Err(err) => panic!("capture: {err}"),
        };
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(report.source, "capture");
        for name in ["toggle_settle_frames", "fade_deviation", "dim_error"] {
            let metric = named(&report, name);
            assert_eq!(
                metric.status,
                Status::NotImplemented,
                "{}",
                summary_line(metric)
            );
            assert_eq!(metric.detail, Some("no requested-weight reference"));
        }
        for name in ["pop_magnitude", "hold_flicker"] {
            let metric = named(&report, name);
            assert_eq!(metric.status, Status::Pass, "{}", summary_line(metric));
        }
        assert_eq!(report.exit_code(), 0);
    }

    #[test]
    fn capture_rejects_a_half_specified_target() {
        let dir =
            std::env::temp_dir().join(format!("genos-layering-cap-half-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        grey(0.5).save_png(&dir.join("frame-0000.png")).unwrap();
        grey(0.2).save_png(&dir.join("frame-0001.png")).unwrap();
        std::fs::write(dir.join("weights.txt"), "1\n0\n").unwrap();
        let err = evaluate_capture(&dir).unwrap_err();
        assert!(err.contains("expected-on"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
