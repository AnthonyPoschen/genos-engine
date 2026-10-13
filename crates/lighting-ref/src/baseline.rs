//! Candidate metrics stored beside a commit's reference pictures.
//!
//! A later run passes that commit as `BASE` and reads this file. Missing keys
//! were not measured. A missing file means regressions are not judged.

use std::collections::BTreeMap;
use std::path::Path;

use crate::metrics::FrameMetrics;
use crate::verdict::{Baseline, MetricId};

#[derive(Clone, Debug, PartialEq)]
pub struct MetricsFile {
    pub commit: String,
    pub rows: Vec<MetricRow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MetricRow {
    pub scene: String,
    pub checkpoint: String,
    pub baseline: Baseline,
}

pub fn write_metrics(
    path: &Path,
    commit: &str,
    rows: &[(&str, &str, &FrameMetrics)],
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("{}: {err}", parent.display()))?;
        }
    }
    let mut body = String::from("{\n");
    body.push_str(&format!("  \"commit\": \"{}\",\n", escape(commit)));
    body.push_str("  \"checkpoints\": [\n");
    for (i, (scene, checkpoint, metrics)) in rows.iter().enumerate() {
        let comma = if i + 1 == rows.len() { "" } else { "," };
        body.push_str("    {\n");
        body.push_str(&format!("      \"scene\": \"{}\",\n", escape(scene)));
        body.push_str(&format!("      \"checkpoint\": \"{}\"", escape(checkpoint)));
        let saved = Baseline::from_metrics(metrics);
        for id in MetricId::all() {
            if let Some(value) = saved.get(*id) {
                body.push_str(&format!(",\n      \"{}\": {}", id.key(), fmt(value)));
            }
        }
        body.push_str(&format!("\n    }}{comma}\n"));
    }
    body.push_str("  ]\n}\n");
    std::fs::write(path, body).map_err(|err| format!("{}: {err}", path.display()))
}

pub fn read_metrics(path: &Path) -> Result<MetricsFile, String> {
    let text = std::fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
    let commit = string_field(&text, "commit").unwrap_or_default();
    let mut rows = Vec::new();
    for object in nested_objects(&text) {
        let Some(scene) = string_field(object, "scene") else {
            continue;
        };
        let Some(checkpoint) = string_field(object, "checkpoint") else {
            continue;
        };
        let mut values = BTreeMap::new();
        for id in MetricId::all() {
            if let Some(value) = number_field(object, id.key()) {
                values.insert(id.key().to_string(), value);
            }
        }
        rows.push(MetricRow {
            scene,
            checkpoint,
            baseline: Baseline { values },
        });
    }
    Ok(MetricsFile { commit, rows })
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

fn fmt(value: f64) -> String {
    if !value.is_finite() {
        return "null".to_string();
    }
    let mut s = format!("{value:.6}");
    while s.contains('.') && s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.push('0');
    }
    s
}

/// Objects nested one level under the root object.
fn nested_objects(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'{' => {
                if depth == 1 {
                    start = i;
                }
                depth += 1;
            }
            b'}' => {
                depth -= 1;
                if depth == 1 && start < text.len() {
                    out.push(&text[start..=i]);
                }
            }
            _ => {}
        }
        i += 1;
    }
    out
}

fn string_field(object: &str, key: &str) -> Option<String> {
    let rest = after_key(object, key)?;
    let rest = rest.trim_start();
    if !rest.starts_with('"') {
        return None;
    }
    let rest = &rest[1..];
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(n) = chars.next() {
                out.push(n);
            }
            continue;
        }
        if c == '"' {
            break;
        }
        out.push(c);
    }
    Some(out)
}

fn number_field(object: &str, key: &str) -> Option<f64> {
    let rest = after_key(object, key)?.trim_start();
    if rest.starts_with("null") {
        return None;
    }
    let end = rest
        .find(|c: char| c == ',' || c == '}' || c == '\n' || c.is_whitespace())
        .unwrap_or(rest.len());
    rest[..end].trim().parse().ok()
}

fn after_key<'a>(object: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("\"{key}\"");
    let idx = object.find(&pat)?;
    let rest = &object[idx + pat.len()..];
    let rest = rest.trim_start();
    rest.strip_prefix(':')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{measure, MetricParams};
    use crate::picture::Picture;

    #[test]
    fn metrics_round_trip_and_a_missing_value_stays_missing() {
        let picture = Picture::solid(4, 2, [0.4, 0.4, 0.4]);
        let mut metrics = measure(
            &picture,
            &picture.scaled(1.2),
            None,
            &[],
            &MetricParams::default(),
        )
        .unwrap();
        metrics.flicker = None;
        let path = std::env::temp_dir().join(format!("lref-metrics-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        write_metrics(&path, "abc", &[("stress-still", "hall", &metrics)]).unwrap();
        let file = read_metrics(&path).unwrap();
        assert_eq!(file.commit, "abc");
        assert_eq!(file.rows.len(), 1);
        assert_eq!(file.rows[0].scene, "stress-still");
        assert_eq!(file.rows[0].checkpoint, "hall");
        let mean = file.rows[0].baseline.get(MetricId::MeanRel).unwrap();
        assert!((mean - 0.2).abs() < 1.0e-4, "{mean}");
        assert!(file.rows[0].baseline.get(MetricId::Flicker).is_none());
        let _ = std::fs::remove_file(&path);
    }
}
