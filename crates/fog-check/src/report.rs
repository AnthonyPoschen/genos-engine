//! Summary lines and the JSON file.

use std::path::{Path, PathBuf};

use crate::{Fixture, Suite};

/// Pass, fail, or not implemented. A missing feature is never a pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// The measured value is within the threshold.
    Pass,
    /// The measured value missed the threshold.
    Fail,
    /// No measurement. A supplied zero is ignored.
    NotImplemented,
}

impl Status {
    /// The token printed on the summary line.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::NotImplemented => "NOT-IMPLEMENTED",
        }
    }
}

/// One metric. `value` is `None` when the feature is absent.
#[derive(Clone, Debug, PartialEq)]
pub struct Metric {
    /// Stable name. No spaces.
    pub name: &'static str,
    /// Measured value. `None` prints as `-`.
    pub value: Option<f64>,
    /// Lower is better. A value equal to the threshold passes.
    pub threshold: f64,
    /// Verdict.
    pub status: Status,
    /// Empty when the metric was graded.
    pub note: &'static str,
}

/// One harness run.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    /// Suite that was requested.
    pub suite: Suite,
    /// Fixture that was requested.
    pub fixture: Fixture,
    /// Metrics in print order.
    pub metrics: Vec<Metric>,
}

/// Lower-is-better verdict.
///
/// `present == false` is `NOT-IMPLEMENTED` even when `value` is `Some(0.0)`.
/// A negative measurement fails. `NaN` fails.
#[must_use]
pub fn judge(present: bool, value: Option<f64>, threshold: f64) -> Status {
    if !present {
        return Status::NotImplemented;
    }
    match value {
        Some(value) if value >= 0.0 && value <= threshold => Status::Pass,
        Some(_) => Status::Fail,
        None => Status::NotImplemented,
    }
}

/// 0 all passed, 1 any failure, 2 no failure and at least one gap.
#[must_use]
pub fn exit_code(report: &Report) -> i32 {
    if report
        .metrics
        .iter()
        .any(|metric| metric.status == Status::Fail)
    {
        1
    } else if report
        .metrics
        .iter()
        .any(|metric| metric.status == Status::NotImplemented)
    {
        2
    } else {
        0
    }
}

/// One compact line per metric: name, value, threshold, status.
#[must_use]
pub fn format_lines(report: &Report) -> String {
    let mut text = String::new();
    for metric in &report.metrics {
        text.push_str(&format_line(metric));
        text.push('\n');
    }
    text
}

fn format_line(metric: &Metric) -> String {
    let value = match metric.value {
        Some(value) => format!("{value:.6e}"),
        None => "-".to_string(),
    };
    format!(
        "{} {value} {:.6e} {}",
        metric.name,
        metric.threshold,
        metric.status.as_str()
    )
}

/// Write `<dir>/metrics.json`. Creates `dir` when it is missing.
///
/// # Errors
///
/// Returns an error when the directory or the file cannot be written.
pub fn write_metrics(dir: &Path, report: &Report) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    let path = dir.join("metrics.json");
    std::fs::write(&path, to_json(report)).map_err(|err| format!("{}: {err}", path.display()))?;
    Ok(path)
}

fn to_json(report: &Report) -> String {
    let mut body = String::from("{\n");
    body.push_str(&format!(
        "  \"suite\": {},\n  \"fixture\": {},\n  \"metrics\": [\n",
        json_string(report.suite.as_str()),
        json_string(report.fixture.as_str())
    ));
    for (index, metric) in report.metrics.iter().enumerate() {
        let comma = if index + 1 == report.metrics.len() {
            ""
        } else {
            ","
        };
        let value = match metric.value {
            Some(value) => format!("{value:.6e}"),
            None => "null".to_string(),
        };
        body.push_str(&format!(
            "    {{\"name\": {}, \"value\": {value}, \"threshold\": {:.6e}, \"status\": {}, \"note\": {}}}{comma}\n",
            json_string(metric.name),
            metric.threshold,
            json_string(metric.status.as_str()),
            json_string(metric.note),
        ));
    }
    body.push_str("  ]\n}\n");
    body
}

fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}
