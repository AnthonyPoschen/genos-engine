#![expect(clippy::as_conversions, reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md")]
#![expect(clippy::cast_precision_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::format_push_string, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::missing_errors_doc, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::must_use_candidate, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![cfg_attr(test, expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails"))]

//! Reference pictures on disk, versioned by the engine commit.
//!
//! The directory is whatever the caller passes. The default the driver uses is
//! [`crate::reference_dir`], under `./genos-debug/reference/<commit>/`.

use std::path::{Path, PathBuf};

use crate::picture::Picture;

#[derive(Clone, Debug, PartialEq)]
pub struct ReferenceRecord {
    pub scene: String,
    pub checkpoint: String,
    pub commit: String,
    pub method: String,
    pub width: u32,
    pub height: u32,
    pub spp: u32,
    pub noise: f32,
    pub seconds: f32,
    pub max_bounces: u32,
    pub png_bytes: u64,
    pub linear_bytes: u64,
    pub note: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkippedScene {
    pub scene: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub commit: String,
    pub profile: String,
    pub records: Vec<ReferenceRecord>,
    pub skipped: Vec<SkippedScene>,
}

impl Manifest {
    pub fn total_bytes(&self) -> u64 {
        self.records
            .iter()
            .map(|r| r.png_bytes + r.linear_bytes)
            .sum()
    }
}

pub fn checkpoint_dir(root: &Path, scene: &str, checkpoint: &str) -> PathBuf {
    root.join(crate::paths::check_name(scene))
        .join(crate::paths::check_name(checkpoint))
}

/// Write `reference.png` and `reference.f32`. Byte sizes come from the files.
pub fn write_reference(
    root: &Path,
    mut record: ReferenceRecord,
    picture: &Picture,
) -> Result<ReferenceRecord, String> {
    let dir = checkpoint_dir(root, &record.scene, &record.checkpoint);
    std::fs::create_dir_all(&dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    let png = dir.join("reference.png");
    let linear = dir.join("reference.f32");
    picture.save_png(&png)?;
    picture.save_linear(&linear)?;
    write_trace(&dir, &record)?;
    record.png_bytes = file_len(&png)?;
    record.linear_bytes = file_len(&linear)?;
    record.width = picture.width;
    record.height = picture.height;
    Ok(record)
}

pub fn read_reference(root: &Path, scene: &str, checkpoint: &str) -> Result<Picture, String> {
    Picture::load_linear(&checkpoint_dir(root, scene, checkpoint).join("reference.f32"))
}

#[derive(Clone, Debug, PartialEq)]
pub struct TraceNote {
    pub method: String,
    pub spp: u32,
    pub noise: f32,
    pub seconds: f32,
    pub max_bounces: u32,
}

pub fn read_trace(root: &Path, scene: &str, checkpoint: &str) -> Result<TraceNote, String> {
    let path = checkpoint_dir(root, scene, checkpoint).join("trace.txt");
    let text =
        std::fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    let mut note = TraceNote {
        method: "path".into(),
        spp: 0,
        noise: 0.0,
        seconds: 0.0,
        max_bounces: 0,
    };
    for line in text.lines() {
        let Some((key, value)) = line.split_once(' ') else {
            continue;
        };
        match key {
            "method" => note.method = value.trim().to_string(),
            "spp" => note.spp = parse_line(&path, key, value)?,
            "noise" => note.noise = parse_line(&path, key, value)?,
            "seconds" => note.seconds = parse_line(&path, key, value)?,
            "max_bounces" => note.max_bounces = parse_line(&path, key, value)?,
            _ => {}
        }
    }
    if note.spp == 0 {
        return Err(format!("{}: missing spp", path.display()));
    }
    Ok(note)
}

fn write_trace(dir: &Path, record: &ReferenceRecord) -> Result<(), String> {
    let path = dir.join("trace.txt");
    let text = format!(
        "method {}\nspp {}\nnoise {:.6}\nseconds {:.6}\nmax_bounces {}\n",
        record.method, record.spp, record.noise, record.seconds, record.max_bounces
    );
    std::fs::write(&path, text).map_err(|err| format!("{}: {err}", path.display()))
}

fn parse_line<T: std::str::FromStr>(path: &Path, key: &str, value: &str) -> Result<T, String> {
    value
        .trim()
        .parse()
        .map_err(|_| format!("{}: bad {key}", path.display()))
}

pub fn write_manifest(root: &Path, manifest: &Manifest) -> Result<PathBuf, String> {
    std::fs::create_dir_all(root).map_err(|err| format!("{}: {err}", root.display()))?;
    let path = root.join("manifest.json");
    std::fs::write(&path, manifest_json(manifest))
        .map_err(|err| format!("{}: {err}", path.display()))?;
    let sizes = root.join("sizes.txt");
    std::fs::write(&sizes, sizes_text(manifest))
        .map_err(|err| format!("{}: {err}", sizes.display()))?;
    Ok(path)
}

pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let kb = bytes as f64 / 1024.0;
    if kb < 1024.0 {
        return format!("{kb:.1} KiB");
    }
    format!("{:.2} MiB", kb / 1024.0)
}

pub fn sizes_text(manifest: &Manifest) -> String {
    let mut out = format!(
        "commit {}\nprofile {}\ntotal {}\n",
        manifest.commit,
        manifest.profile,
        format_bytes(manifest.total_bytes())
    );
    for record in &manifest.records {
        out.push_str(&format!(
            "{} {}  png {}  linear {}  {} spp  noise {:.4}  {}x{}\n",
            record.scene,
            record.checkpoint,
            format_bytes(record.png_bytes),
            format_bytes(record.linear_bytes),
            record.spp,
            record.noise,
            record.width,
            record.height
        ));
    }
    for skip in &manifest.skipped {
        out.push_str(&format!("skip {}  {}\n", skip.scene, skip.reason));
    }
    out
}

fn manifest_json(manifest: &Manifest) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!(
        "  \"commit\": \"{}\",\n",
        escape(&manifest.commit)
    ));
    out.push_str(&format!(
        "  \"profile\": \"{}\",\n",
        escape(&manifest.profile)
    ));
    out.push_str(&format!("  \"total_bytes\": {},\n", manifest.total_bytes()));
    out.push_str("  \"scenes\": [\n");
    for (i, record) in manifest.records.iter().enumerate() {
        let comma = if i + 1 == manifest.records.len() {
            ""
        } else {
            ","
        };
        out.push_str(&format!(
            "    {{\"scene\": \"{}\", \"checkpoint\": \"{}\", \"method\": \"{}\", \
             \"width\": {}, \"height\": {}, \"spp\": {}, \"noise\": {}, \
             \"seconds\": {}, \"max_bounces\": {}, \"png_bytes\": {}, \
             \"linear_bytes\": {}, \"note\": \"{}\"}}{comma}\n",
            escape(&record.scene),
            escape(&record.checkpoint),
            escape(&record.method),
            record.width,
            record.height,
            record.spp,
            fmt_f32(record.noise),
            fmt_f32(record.seconds),
            record.max_bounces,
            record.png_bytes,
            record.linear_bytes,
            escape(&record.note),
        ));
    }
    out.push_str("  ],\n  \"skipped\": [\n");
    for (i, skip) in manifest.skipped.iter().enumerate() {
        let comma = if i + 1 == manifest.skipped.len() {
            ""
        } else {
            ","
        };
        out.push_str(&format!(
            "    {{\"scene\": \"{}\", \"reason\": \"{}\"}}{comma}\n",
            escape(&skip.scene),
            escape(&skip.reason)
        ));
    }
    out.push_str("  ]\n}\n");
    out
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

fn fmt_f32(v: f32) -> String {
    if !v.is_finite() {
        return "null".to_string();
    }
    let mut s = format!("{v:.6}");
    while s.contains('.') && s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.push('0');
    }
    s
}

fn file_len(path: &Path) -> Result<u64, String> {
    std::fs::metadata(path)
        .map(|m| m.len())
        .map_err(|err| format!("{}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reference_is_versioned_and_the_sizes_match_the_files() {
        let root = std::env::temp_dir().join(format!("lref-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let picture = Picture::solid(4, 2, [0.2, 0.4, 0.6]);
        let record = write_reference(
            &root,
            ReferenceRecord {
                scene: "stress-still".into(),
                checkpoint: "hall".into(),
                commit: "abc123".into(),
                method: "path".into(),
                width: 0,
                height: 0,
                spp: 1,
                noise: 0.5,
                seconds: 0.01,
                max_bounces: 1,
                png_bytes: 0,
                linear_bytes: 0,
                note: "test".into(),
            },
            &picture,
        )
        .unwrap();
        assert_eq!(record.width, 4);
        assert!(record.png_bytes > 8);
        assert!(record.linear_bytes > 16);
        let png = std::fs::read(root.join("stress-still/hall/reference.png")).unwrap();
        assert_eq!(&png[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        assert_eq!(png.len() as u64, record.png_bytes);
        let loaded = read_reference(&root, "stress-still", "hall").unwrap();
        assert_eq!(loaded, picture);
        let trace = read_trace(&root, "stress-still", "hall").unwrap();
        assert_eq!(trace.spp, 1);
        assert_eq!(trace.method, "path");
        let manifest = Manifest {
            commit: "abc123".into(),
            profile: "test".into(),
            records: vec![record],
            skipped: vec![SkippedScene {
                scene: "sponza".into(),
                reason: "no Sponza asset in this repository".into(),
            }],
        };
        write_manifest(&root, &manifest).unwrap();
        let text = std::fs::read_to_string(root.join("manifest.json")).unwrap();
        assert!(text.contains("\"commit\": \"abc123\""));
        assert!(text.contains("\"total_bytes\":"));
        assert!(text.contains("sponza"));
        let sizes = std::fs::read_to_string(root.join("sizes.txt")).unwrap();
        assert!(sizes.contains("stress-still hall"));
        assert!(sizes.contains("skip sponza"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
