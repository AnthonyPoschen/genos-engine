//! GI pass times and work counters for one recorded window.
//!
//! The renderer fills [`GiSample`] while `--gi-metrics` is on. This module
//! turns that window into p50 and p99, and into the CSV files under
//! `./genos-debug/gi/`.

#![expect(
    clippy::missing_errors_doc,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::must_use_candidate,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![cfg_attr(
    test,
    expect(clippy::panic, reason = "a test aborts when a fixture fails")
)]
#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")
)]

use std::fmt::Write;
use std::path::Path;

use genos_mcp::json::Value;
use genos_render::GiSample;

use crate::value::{num, obj, text};

pub const FRAME_HEADER: &str =
    "frame,direct_ms,atrous_ms,compose_ms,trace_ms,filter_ms,gather_ms,resolve_ms,lighting_ms,rays,cache_entries_refreshed,lamps_shaded\n";

pub const SUMMARY_HEADER: &str = "name,p50,p99\n";

/// Nearest rank. `p` is 0..1. An empty window is 0.
pub fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let last = sorted.len() - 1;
    sorted[nearest_rank(last, p).min(last)]
}

/// `round((last) * p)` without a lossy cast. `last` is the last index.
fn nearest_rank(last: usize, p: f64) -> usize {
    let Ok(last_u) = u32::try_from(last) else {
        return last;
    };
    let target = f64::from(last_u) * p;
    let mut index = 0usize;
    while index < last {
        let next = index + 1;
        let Ok(next_u) = u32::try_from(next) else {
            break;
        };
        if f64::from(next_u) - 0.5 > target {
            break;
        }
        index = next;
    }
    index
}

fn count_f64(value: u64) -> f64 {
    let Ok(value) = u32::try_from(value) else {
        return f64::from(u32::MAX);
    };
    f64::from(value)
}

fn len_f64(len: usize) -> f64 {
    let Ok(len) = u64::try_from(len) else {
        return f64::from(u32::MAX);
    };
    count_f64(len)
}

pub fn summary_csv(samples: &[GiSample]) -> String {
    let mut out = String::from(SUMMARY_HEADER);
    let mut push = |name: &str, values: Vec<f64>, whole: bool| {
        let p50 = percentile(&values, 0.50);
        let p99 = percentile(&values, 0.99);
        if whole {
            let _ = writeln!(out, "{name},{p50:.0},{p99:.0}");
        } else {
            let _ = writeln!(out, "{name},{p50:.4},{p99:.4}");
        }
    };
    push("direct_ms", column(samples, |s| s.direct_ms), false);
    push("atrous_ms", column(samples, |s| s.atrous_ms), false);
    push("compose_ms", column(samples, |s| s.compose_ms), false);
    push("trace_ms", column(samples, |s| s.trace_ms), false);
    push("filter_ms", column(samples, |s| s.filter_ms), false);
    push("gather_ms", column(samples, |s| s.gather_ms), false);
    push("resolve_ms", column(samples, |s| s.resolve_ms), false);
    push("lighting_ms", column(samples, |s| s.lighting_ms), false);
    push("rays", column(samples, |s| f64::from(s.rays)), true);
    push(
        "cache_entries_refreshed",
        column(samples, |s| f64::from(s.cache_entries_refreshed)),
        true,
    );
    push(
        "lamps_shaded",
        column(samples, |s| f64::from(s.lamps_shaded)),
        true,
    );
    out
}

pub fn frames_csv(samples: &[GiSample]) -> String {
    let mut out = String::from(FRAME_HEADER);
    for sample in samples {
        out.push_str(&frame_row(sample));
    }
    out
}

pub fn frame_row(sample: &GiSample) -> String {
    format!(
        "{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{},{},{}\n",
        sample.seq,
        sample.direct_ms,
        sample.atrous_ms,
        sample.compose_ms,
        sample.trace_ms,
        sample.filter_ms,
        sample.gather_ms,
        sample.resolve_ms,
        sample.lighting_ms,
        sample.rays,
        sample.cache_entries_refreshed,
        sample.lamps_shaded
    )
}

/// Letters, digits, hyphen, and underscore. Keeps the file inside the GI directory.
pub fn scenario_name(name: &str) -> Result<&str, String> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("scenario name is letters, digits, hyphen, or underscore".into());
    }
    Ok(name)
}

pub fn write_summary(
    dir: &Path,
    name: &str,
    samples: &[GiSample],
) -> Result<std::path::PathBuf, String> {
    let name = scenario_name(name)?;
    std::fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    let path = dir.join(format!("{name}.csv"));
    std::fs::write(&path, summary_csv(samples))
        .map_err(|err| format!("{}: {err}", path.display()))?;
    let frames = dir.join(format!("{name}-frames.csv"));
    std::fs::write(&frames, frames_csv(samples))
        .map_err(|err| format!("{}: {err}", frames.display()))?;
    Ok(path)
}

pub fn append_frames(dir: &Path, samples: &[GiSample]) -> Result<(), String> {
    if samples.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    let path = dir.join("frames.csv");
    let exists = path.is_file();
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| format!("{}: {err}", path.display()))?;
    if !exists {
        std::io::Write::write_all(&mut file, FRAME_HEADER.as_bytes())
            .map_err(|err| format!("{}: {err}", path.display()))?;
    }
    for sample in samples {
        std::io::Write::write_all(&mut file, frame_row(sample).as_bytes())
            .map_err(|err| format!("{}: {err}", path.display()))?;
    }
    Ok(())
}

pub fn truncate_frames(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    let path = dir.join("frames.csv");
    std::fs::write(&path, FRAME_HEADER).map_err(|err| format!("{}: {err}", path.display()))
}

fn column(samples: &[GiSample], pick: impl Fn(&GiSample) -> f64) -> Vec<f64> {
    samples.iter().map(pick).collect()
}

pub fn sample_value(sample: &GiSample) -> Value {
    obj(vec![
        ("frame", num(count_f64(sample.seq))),
        ("direct_ms", num(sample.direct_ms)),
        ("atrous_ms", num(sample.atrous_ms)),
        ("compose_ms", num(sample.compose_ms)),
        ("trace_ms", num(sample.trace_ms)),
        ("filter_ms", num(sample.filter_ms)),
        ("gather_ms", num(sample.gather_ms)),
        ("resolve_ms", num(sample.resolve_ms)),
        ("lighting_ms", num(sample.lighting_ms)),
        ("rays", num(f64::from(sample.rays))),
        (
            "cache_entries_refreshed",
            num(f64::from(sample.cache_entries_refreshed)),
        ),
        ("lamps_shaded", num(f64::from(sample.lamps_shaded))),
    ])
}

pub fn summary_value(samples: &[GiSample]) -> Value {
    let pair = |pick: fn(&GiSample) -> f64| {
        let values = column(samples, pick);
        obj(vec![
            ("p50", num(percentile(&values, 0.50))),
            ("p99", num(percentile(&values, 0.99))),
        ])
    };
    let count = |pick: fn(&GiSample) -> u32| {
        let values: Vec<f64> = samples.iter().map(|s| f64::from(pick(s))).collect();
        obj(vec![
            ("p50", num(percentile(&values, 0.50))),
            ("p99", num(percentile(&values, 0.99))),
        ])
    };
    obj(vec![
        ("direct_ms", pair(|s| s.direct_ms)),
        ("atrous_ms", pair(|s| s.atrous_ms)),
        ("compose_ms", pair(|s| s.compose_ms)),
        ("trace_ms", pair(|s| s.trace_ms)),
        ("filter_ms", pair(|s| s.filter_ms)),
        ("gather_ms", pair(|s| s.gather_ms)),
        ("resolve_ms", pair(|s| s.resolve_ms)),
        ("lighting_ms", pair(|s| s.lighting_ms)),
        ("rays", count(|s| s.rays)),
        (
            "cache_entries_refreshed",
            count(|s| s.cache_entries_refreshed),
        ),
        ("lamps_shaded", count(|s| s.lamps_shaded)),
    ])
}

/// JSON for `gi_metrics` before a file is written.
pub fn report_value(enabled: bool, samples: &[GiSample]) -> Value {
    let latest = samples.last().map_or(Value::Null, sample_value);
    obj(vec![
        ("enabled", Value::Bool(enabled)),
        ("recorded", num(len_f64(samples.len()))),
        ("latest", latest),
        (
            "summary",
            if samples.is_empty() {
                Value::Null
            } else {
                summary_value(samples)
            },
        ),
    ])
}

pub fn with_path(mut value: Value, path: &Path) -> Value {
    if let Value::Object(pairs) = &mut value {
        pairs.push(("path".into(), text(path.display().to_string())));
    }
    value
}

/// The command text when metrics were not turned on at launch.
pub fn disabled_error() -> String {
    "GI metrics are off. Pass --gi-metrics. GENOS_GI_METRICS applies only when that argument is omitted.".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(seq: u64, direct: f64, rays: u32) -> GiSample {
        GiSample {
            seq,
            direct_ms: direct,
            atrous_ms: direct + 1.0,
            compose_ms: 0.4,
            trace_ms: 0.5,
            filter_ms: 0.1,
            gather_ms: 0.05,
            resolve_ms: 0.55,
            lighting_ms: direct + 2.0,
            rays,
            cache_entries_refreshed: rays / 2,
            lamps_shaded: 3,
        }
    }

    #[test]
    fn p50_and_p99_use_nearest_rank() {
        let values = [1.0, 2.0, 3.0, 4.0, 100.0];
        assert!((percentile(&values, 0.50) - 3.0).abs() < 1e-9);
        assert!((percentile(&values, 0.99) - 100.0).abs() < 1e-9);
        assert!((percentile(&[], 0.50)).abs() < 1e-9);
    }

    #[test]
    fn summary_csv_names_every_pass_and_counter() {
        let samples = [sample(1, 1.0, 10), sample(2, 2.0, 20), sample(3, 3.0, 40)];
        let csv = summary_csv(&samples);
        assert!(csv.starts_with("name,p50,p99\n"));
        for name in [
            "direct_ms",
            "atrous_ms",
            "compose_ms",
            "trace_ms",
            "filter_ms",
            "gather_ms",
            "resolve_ms",
            "lighting_ms",
            "rays",
            "cache_entries_refreshed",
            "lamps_shaded",
        ] {
            assert!(csv.contains(name), "{name} missing from {csv}");
        }
        assert!(csv.contains("direct_ms,2.0000,3.0000\n"), "{csv}");
        assert!(csv.contains("rays,20,40\n"), "{csv}");
        let frames = frames_csv(&samples);
        assert!(frames.starts_with(FRAME_HEADER));
        assert!(frames.contains("1,1.0000,"));
        assert!(scenario_name("s1-spin").is_ok());
        assert!(scenario_name("../x").is_err());
        assert!(scenario_name("").is_err());
    }

    #[test]
    fn a_summary_file_stays_inside_the_gi_directory() {
        let dir = std::env::temp_dir().join(format!("genos-gi-metrics-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let samples = [sample(1, 1.5, 8)];
        let path = write_summary(&dir, "s6-settle", &samples).unwrap();
        assert_eq!(path, dir.join("s6-settle.csv"));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("direct_ms,1.5000,1.5000"));
        let frames = std::fs::read_to_string(dir.join("s6-settle-frames.csv")).unwrap();
        assert!(frames.contains("1,1.5000,"));
        append_frames(&dir, &samples).unwrap();
        append_frames(&dir, &samples).unwrap();
        let live = std::fs::read_to_string(dir.join("frames.csv")).unwrap();
        assert_eq!(live.matches("1,1.5000,").count(), 2);
        assert!(live.starts_with(FRAME_HEADER));
        truncate_frames(&dir).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("frames.csv")).unwrap(),
            FRAME_HEADER
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gate_scripts_parse() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../debug/lighting/scripts/gates");
        let engine = rhai::Engine::new();
        let names = [
            "s1-spin.rhai",
            "s2-strafe.rhai",
            "s3-approach.rhai",
            "s4-many-movers.rhai",
            "s5-aba.rhai",
            "s6-settle.rhai",
        ];
        for name in names {
            let path = dir.join(name);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("{}: {err}", path.display()));
            assert!(
                text.contains("gi_metrics"),
                "{name} does not record GI metrics"
            );
            assert!(text.contains("fixed_dt"), "{name} does not fix the step");
            engine
                .compile(&text)
                .unwrap_or_else(|err| panic!("{name}: {err}"));
        }
    }
}
