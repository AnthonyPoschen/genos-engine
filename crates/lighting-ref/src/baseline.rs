//! Candidate metrics stored beside a commit's reference pictures.
//!
//! A later run passes that commit as `BASE` and reads this file. Missing keys
//! were not measured. A missing file means regressions are not judged.

use std::collections::BTreeMap;
use std::path::Path;

use crate::metrics::{measure, FrameMetrics, MetricParams};
use crate::picture::Picture;
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

/// Metrics a later `--base` can regress against before any live picture exists.
///
/// Raw error is the reference's own noise. Blurred error is that noise after
/// the same box the score uses. The luminance ratio error is 0: a matching
/// picture has the reference's mean brightness. Flicker and convergence are
/// omitted; a still reference has no live sequence.
pub fn reference_metrics(picture: &Picture, noise: f64) -> Result<FrameMetrics, String> {
    let mut metrics = measure(picture, picture, None, &[], &MetricParams::default())?;
    let noise = if noise.is_finite() {
        noise.max(0.0)
    } else {
        0.0
    };
    let tail = crate::config::Config::builtin().noise_tail;
    let kernel = f64::from(crate::metrics::blur_radius(picture.height) * 2 + 1);
    metrics.mean_rel = Some(noise);
    metrics.p99_rel = Some(noise * tail);
    if metrics.mean_rel_dark.is_some() {
        metrics.mean_rel_dark = Some(noise);
    }
    metrics.flip_mean = Some(noise.min(1.0));
    metrics.flip_p99 = Some((noise * tail).min(1.0));
    metrics.blur_rel = Some(noise / kernel);
    metrics.luma_ratio = Some(1.0);
    metrics.luma_delta = Some(0.0);
    metrics.flicker = None;
    metrics.converge_1 = None;
    metrics.converge_2 = None;
    metrics.converge_4 = None;
    metrics.converge_8 = None;
    metrics.unfilled = Some(0);
    Ok(metrics)
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

/// Write `updates` into `path`, keeping every checkpoint the file already
/// names that this call does not replace.
pub fn upsert_metrics(
    path: &Path,
    commit: &str,
    updates: &[(&str, &str, &FrameMetrics)],
) -> Result<(), String> {
    let mut kept = Vec::new();
    if path.is_file() {
        let text =
            std::fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
        for object in nested_objects(&text) {
            let scene = string_field(object, "scene").unwrap_or_default();
            let checkpoint = string_field(object, "checkpoint").unwrap_or_default();
            let replaced = updates.iter().any(|(next_scene, next_checkpoint, _)| {
                *next_scene == scene && *next_checkpoint == checkpoint
            });
            if !replaced {
                kept.push(object.trim().trim_end_matches(',').trim().to_string());
            }
        }
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("{}: {err}", parent.display()))?;
        }
    }
    let mut body = String::from("{\n");
    body.push_str(&format!("  \"commit\": \"{}\",\n", escape(commit)));
    body.push_str("  \"checkpoints\": [\n");
    let total = kept.len() + updates.len();
    for (i, object) in kept.iter().enumerate() {
        let comma = if i + 1 == total { "" } else { "," };
        body.push_str("    ");
        body.push_str(object);
        body.push_str(comma);
        body.push('\n');
    }
    for (i, (scene, checkpoint, metrics)) in updates.iter().enumerate() {
        let comma = if kept.len() + i + 1 == total { "" } else { "," };
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

    #[test]
    fn a_reference_floor_stores_the_noise_and_a_ratio_of_one() {
        let picture = Picture::solid(4, 180, [0.4, 0.4, 0.4]);
        let metrics = reference_metrics(&picture, 0.31).unwrap();
        assert!((metrics.mean_rel.unwrap() - 0.31).abs() < 1.0e-9);
        assert!((metrics.p99_rel.unwrap() - 1.24).abs() < 1.0e-9);
        assert!((metrics.blur_rel.unwrap() - 0.31 / 9.0).abs() < 1.0e-9);
        assert!((metrics.luma_delta.unwrap()).abs() < 1.0e-9);
        assert!(metrics.flicker.is_none());
        let path = std::env::temp_dir().join(format!("lref-floor-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        write_metrics(&path, "abc", &[("direct-only", "hall", &metrics)]).unwrap();
        let file = read_metrics(&path).unwrap();
        assert_eq!(file.rows[0].checkpoint, "hall");
        let blur = file.rows[0].baseline.get(MetricId::BlurRel).unwrap();
        assert!((blur - 0.31 / 9.0).abs() < 1.0e-4, "{blur}");
        assert!(
            file.rows[0]
                .baseline
                .get(MetricId::LumaDelta)
                .unwrap()
                .abs()
                < 1.0e-9
        );
        assert!(file.rows[0].baseline.get(MetricId::Flicker).is_none());
        let _ = std::fs::remove_file(&path);
    }
}
