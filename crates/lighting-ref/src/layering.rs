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
//! Planted pictures prove each comparison both ways. A live lamp toggle is
//! not invented: `--case baseline` prints `NOT-IMPLEMENTED`.

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
    /// Today's engine: nothing to measure yet.
    Baseline,
}

impl Case {
    /// `good`, `broken`, or `baseline`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "good" => Some(Self::Good),
            "broken" => Some(Self::Broken),
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
    /// Relative error or pop magnitude.
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
    /// `good`, `broken`, or `baseline`.
    pub case_name: &'static str,
    /// `fixture` or `absent`.
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

/// Grade one case.
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
    }
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
    let settled = settle_index(series)?;
    let observed = u32::try_from(series.frames.len()).map_err(|_| "too many frames".to_string())?;
    let pop = pop_magnitude(series)?;
    let dim = dim_error(series)?;
    let same = blur_relative(layer_reference, layer_frame, &MetricParams::default())?;
    let mismatches = mismatch_count(reads);
    Ok(Report {
        case_name: case.name(),
        source: "fixture",
        metrics: vec![
            judge_settle(settled, observed),
            judge_ratio("pop_magnitude", pop, POP_LIMIT),
            judge_ratio("dim_error", dim, SETTLE_LINE),
            judge_ratio("layer_same_frame", same, SETTLE_LINE),
            judge_bytes(layer_bytes),
            judge_count(mismatches),
        ],
    })
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

fn judge_settle(settled_at: Option<u32>, observed: u32) -> Metric {
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
        detail: None,
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

fn dim_error(series: &Series) -> Result<f64, String> {
    let last = series
        .frames
        .last()
        .ok_or_else(|| "dim needs a frame".to_string())?;
    blur_relative(&series.reference, last, &MetricParams::default())
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

#[cfg(test)]
mod tests {
    use super::{evaluate, summary_line, write_report, Case, MetricValue, Status};

    fn graded(case: Case) -> super::Report {
        match evaluate(case) {
            Ok(report) => report,
            Err(err) => panic!("evaluate: {err}"),
        }
    }

    #[test]
    fn good_fixture_passes_every_check() {
        let report = graded(Case::Good);
        assert_eq!(report.exit_code(), 0);
        assert_eq!(report.metrics.len(), 6);
        for metric in &report.metrics {
            assert_eq!(metric.status, Status::Pass, "{}", metric.name);
            assert_ne!(metric.value, MetricValue::Missing, "{}", metric.name);
        }
    }

    #[test]
    fn broken_fixture_fails_every_check() {
        let report = graded(Case::Broken);
        assert_eq!(report.exit_code(), 1);
        assert_eq!(report.metrics.len(), 6);
        for metric in &report.metrics {
            assert_eq!(metric.status, Status::Fail, "{}", metric.name);
            assert_ne!(metric.value, MetricValue::Missing, "{}", metric.name);
        }
    }

    #[test]
    fn baseline_marks_every_row_not_implemented() {
        let report = graded(Case::Baseline);
        assert_eq!(report.exit_code(), 0);
        assert_eq!(report.source, "absent");
        for metric in &report.metrics {
            assert_eq!(metric.status, Status::NotImplemented, "{}", metric.name);
            assert_eq!(metric.value, MetricValue::Missing, "{}", metric.name);
            assert!(metric.detail.is_some(), "{}", metric.name);
        }
    }

    #[test]
    fn summary_line_is_four_fields() {
        for case in [Case::Good, Case::Broken, Case::Baseline] {
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
}
