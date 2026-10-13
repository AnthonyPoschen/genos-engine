//! One folder a person can read without rerunning the check.
//!
//! Side-by-side pictures, a heatmap, `metrics.json`, and a summary that says
//! why the verdict was given. The caller chooses the directory (`--out`, or
//! `./genos-debug/refcheck/<timestamp>/`).

use std::path::{Path, PathBuf};

use crate::metrics::FrameMetrics;
use crate::picture::{encode_png, Picture};
use crate::verdict::{metric_value, Judgement, MetricId, Verdict};

/// Limits of this checker. Printed in every summary so a pass is not mistaken
/// for a proof the picture is right.
pub const BLIND_SPOTS: &[&str] = &[
    "LDR-FLIP here is not NVIDIA's flip tool. The paper is Andersson, Nilsson, and Akenine-Möller 2020; the kernel and the color difference differ, and the module lists those departures. A disagreement between FLIP and relative luminance is a human review, not a calibration against NVIDIA's numbers.",
    "The path tracer does not trace fog, particle cards, or textures. A scene that depends on those is outside this reference.",
    "Sponza is skipped until an asset is in the repository.",
    "Unfilled pixels are holes: candidate luminance near black where the reference is lit. A wrong texel that is merely dim, or a hole that matches a truly black reference, is not counted. The surface-cache set counter is not read.",
    "Flicker is temporal variance on a still camera. A moving camera is not scored for flicker. Motion aliasing is not separated from shimmer.",
    "Convergence is only the error at +1, +2, +4, and +8 frames after an event. A defect that heals at frame 3 and returns at frame 16 is missed.",
    "Mean error can hide a speck smaller than the p99. p99 and the unfilled count are the counter, and they still miss a mark on one pixel of a large picture.",
    "A one-sample reference has no noise estimate, so a small error cannot auto-pass. Two or more samples can, only when the error is above the reference noise.",
    "Hell-100 is one 25 m section with 100 lamps, the stress panel's 100-light case, not the 4×4 building. The path trace and the live capture use that same scene.",
    "Full-effort accumulation is the engine at its maximum knobs. It is not independent of a lighting change. The path trace is the ground truth this check scores against.",
    "Lavapipe is a wiring check. Quality judgement is the owner GPU run.",
    "GPU frames are not cross-device deterministic. The path trace is deterministic for a given engine commit, scene, and sample count.",
    "A held stale frame on a still scene with no event matches a converged picture and is not caught. The box-step checkpoint exists so a held frame after a change is caught.",
    "Owner mode and lavapipe-small share one frame plan: a fixed scene step (default 1/60 s), warmup frames, then a settle-frame count or a mean-relative-luminance threshold with a frame cap, and the same capture offsets (default event+1, +2, +4, +8). Wall time is a column in the frame log. The scene pose follows the frame index. picture_settled is logged and ignored on the first frame of a pose. The check does not call wait_settled. A picture that is still over the line when the settle cap is reached fails that capture and stays for review.",
];

#[derive(Clone, Debug)]
pub struct CheckpointAudit {
    pub scene: String,
    pub checkpoint: String,
    pub note: String,
    pub judgement: Judgement,
    pub metrics: FrameMetrics,
    pub reference: Option<Picture>,
    pub candidate: Option<Picture>,
    pub reference_spp: u32,
    pub reference_noise: f64,
    pub reference_method: String,
}

#[derive(Clone, Debug)]
pub struct RunReport {
    pub commit: String,
    pub baseline: Option<String>,
    pub mode: String,
    pub checkpoints: Vec<CheckpointAudit>,
    pub skipped: Vec<(String, String)>,
    pub notes: Vec<String>,
}

impl RunReport {
    pub fn verdict(&self) -> Verdict {
        self.checkpoints
            .iter()
            .fold(Verdict::Pass, |acc, cp| match (acc, cp.judgement.verdict) {
                (Verdict::Fail, _) | (_, Verdict::Fail) => Verdict::Fail,
                (Verdict::NeedsHuman, _) | (_, Verdict::NeedsHuman) => Verdict::NeedsHuman,
                _ => Verdict::Pass,
            })
    }
}

pub fn write_report(dir: &Path, report: &RunReport) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    for cp in &report.checkpoints {
        let folder = dir.join(&cp.scene).join(&cp.checkpoint);
        std::fs::create_dir_all(&folder).map_err(|err| format!("{}: {err}", folder.display()))?;
        if let (Some(reference), Some(candidate)) = (&cp.reference, &cp.candidate) {
            reference.save_png(&folder.join("reference.png"))?;
            candidate.save_png(&folder.join("candidate.png"))?;
            let side = side_by_side(reference, candidate);
            std::fs::write(folder.join("side-by-side.png"), side)
                .map_err(|err| format!("side-by-side: {err}"))?;
            let heat = heatmap_png(reference, candidate);
            std::fs::write(folder.join("heatmap.png"), heat)
                .map_err(|err| format!("heatmap: {err}"))?;
        } else if let Some(reference) = &cp.reference {
            reference.save_png(&folder.join("reference.png"))?;
        }
        std::fs::write(folder.join("metrics.json"), checkpoint_json(cp))
            .map_err(|err| format!("metrics.json: {err}"))?;
    }
    let summary = summary_md(report);
    std::fs::write(dir.join("summary.md"), &summary).map_err(|err| format!("summary.md: {err}"))?;
    std::fs::write(dir.join("summary.html"), summary_html(report))
        .map_err(|err| format!("summary.html: {err}"))?;
    std::fs::write(dir.join("metrics.json"), run_json(report))
        .map_err(|err| format!("metrics.json: {err}"))?;
    Ok(dir.to_path_buf())
}

fn side_by_side(reference: &Picture, candidate: &Picture) -> Vec<u8> {
    let heat = heatmap_rgb(reference, candidate);
    let left = reference.toned_rgb8();
    let mid = candidate.toned_rgb8();
    let w = reference.width;
    let h = reference.height;
    let gap = 4u32;
    let out_w = w * 3 + gap * 2;
    let mut rgb = vec![32u8; (out_w * h * 3) as usize];
    blit(&mut rgb, out_w, &left, w, h, 0);
    blit(&mut rgb, out_w, &mid, w, h, w + gap);
    blit(&mut rgb, out_w, &heat, w, h, w * 2 + gap * 2);
    encode_png(out_w, h, &rgb)
}

fn blit(dst: &mut [u8], dst_w: u32, src: &[u8], w: u32, h: u32, x0: u32) {
    for y in 0..h {
        for x in 0..w {
            let s = ((y * w + x) * 3) as usize;
            let d = ((y * dst_w + x0 + x) * 3) as usize;
            if s + 2 < src.len() && d + 2 < dst.len() {
                dst[d..d + 3].copy_from_slice(&src[s..s + 3]);
            }
        }
    }
}

fn heatmap_png(reference: &Picture, candidate: &Picture) -> Vec<u8> {
    let rgb = heatmap_rgb(reference, candidate);
    encode_png(reference.width, reference.height, &rgb)
}

/// Relative luminance, black through blue and red to white. `scale` is 0.5:
/// an error of half the light is white.
fn heatmap_rgb(reference: &Picture, candidate: &Picture) -> Vec<u8> {
    let scale = 0.5f32;
    let mut rgb = Vec::with_capacity(reference.pixels() * 3);
    let n = reference.pixels().min(candidate.pixels());
    for i in 0..n {
        let yr = reference.luma_at(i);
        let yc = candidate.luma_at(i);
        let rel = (yc - yr).abs() / yr.max(0.02);
        let t = (rel / scale).clamp(0.0, 1.0);
        let (r, g, b) = ramp(t);
        rgb.extend_from_slice(&[r, g, b]);
    }
    rgb
}

fn ramp(t: f32) -> (u8, u8, u8) {
    let (r, g, b) = if t < 0.33 {
        let u = t / 0.33;
        (0.0, 0.15 * u, 0.85 * u)
    } else if t < 0.66 {
        let u = (t - 0.33) / 0.33;
        (u, 0.15 * (1.0 - u), 0.85 * (1.0 - u) + 0.05 * u)
    } else {
        let u = (t - 0.66) / 0.34;
        (1.0, u, 0.05 * (1.0 - u) + u)
    };
    ((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

fn summary_md(report: &RunReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# Lighting reference check\n\nVerdict: **{}**\n\n",
        report.verdict().label()
    ));
    out.push_str(&format!("- Commit: `{}`\n", report.commit));
    out.push_str(&format!("- Mode: {}\n", report.mode));
    match &report.baseline {
        Some(commit) => out.push_str(&format!("- Baseline: `{commit}`\n")),
        None => out.push_str("- Baseline: none stored, so regressions were not judged\n"),
    }
    out.push_str("\n");
    for note in &report.notes {
        out.push_str(&format!("- {note}\n"));
    }
    if !report.notes.is_empty() {
        out.push('\n');
    }
    for cp in &report.checkpoints {
        out.push_str(&format!(
            "## {} / {} — {}\n\n{}\n\n",
            cp.scene,
            cp.checkpoint,
            cp.judgement.verdict.label(),
            cp.note
        ));
        out.push_str(&format!(
            "Reference: {} , {} spp, noise {:.4}.\n\n",
            cp.reference_method, cp.reference_spp, cp.reference_noise
        ));
        out.push_str("Why:\n\n");
        for reason in &cp.judgement.reasons {
            out.push_str(&format!("- {reason}\n"));
        }
        out.push_str("\n| Metric | Value | Baseline | Verdict | Why |\n|---|---:|---:|---|---|\n");
        for row in &cp.judgement.rows {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                row.id.label(),
                fmt_opt(row.value),
                fmt_opt(row.baseline),
                row.verdict.label(),
                row.reason.replace('|', "/")
            ));
        }
        if cp.candidate.is_some() {
            out.push_str(&format!(
                "\n![reference, candidate, heatmap]({}/{}/side-by-side.png)\n\n",
                cp.scene, cp.checkpoint
            ));
        } else {
            out.push_str("\nNo candidate picture was captured, so there is no side-by-side.\n\n");
        }
    }
    if !report.skipped.is_empty() {
        out.push_str("## Skipped\n\n");
        for (scene, reason) in &report.skipped {
            out.push_str(&format!("- {scene}: {reason}\n"));
        }
        out.push('\n');
    }
    out.push_str("## Blind spots\n\n");
    for spot in BLIND_SPOTS {
        out.push_str(&format!("- {spot}\n"));
    }
    out.push('\n');
    out
}

fn summary_html(report: &RunReport) -> String {
    let mut body = String::new();
    body.push_str(&format!(
        "<h1>Lighting reference check</h1>\n<p class=\"verdict {}\">{}</p>\n",
        verdict_class(report.verdict()),
        escape(report.verdict().label())
    ));
    body.push_str(&format!(
        "<p>Commit <code>{}</code>. Mode {}. Baseline: {}.</p>\n",
        escape(&report.commit),
        escape(&report.mode),
        escape(
            &report
                .baseline
                .clone()
                .unwrap_or_else(|| "none stored, so regressions were not judged".into())
        )
    ));
    if !report.notes.is_empty() {
        body.push_str("<ul>\n");
        for note in &report.notes {
            body.push_str(&format!("<li>{}</li>\n", escape(note)));
        }
        body.push_str("</ul>\n");
    }
    for cp in &report.checkpoints {
        body.push_str(&format!(
            "<h2>{} / {} <span class=\"{}\">{}</span></h2>\n<p>{}</p>\n",
            escape(&cp.scene),
            escape(&cp.checkpoint),
            verdict_class(cp.judgement.verdict),
            escape(cp.judgement.verdict.label()),
            escape(&cp.note)
        ));
        body.push_str("<p>Why:</p><ul>\n");
        for reason in &cp.judgement.reasons {
            body.push_str(&format!("<li>{}</li>\n", escape(reason)));
        }
        body.push_str("</ul>\n<table><tr><th>Metric</th><th>Value</th><th>Baseline</th><th>Verdict</th><th>Why</th></tr>\n");
        for row in &cp.judgement.rows {
            body.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td class=\"{}\">{}</td><td>{}</td></tr>\n",
                escape(row.id.label()),
                fmt_opt(row.value),
                fmt_opt(row.baseline),
                verdict_class(row.verdict),
                escape(row.verdict.label()),
                escape(&row.reason)
            ));
        }
        body.push_str("</table>\n");
        if cp.candidate.is_some() {
            let src = format!("{}/{}/side-by-side.png", cp.scene, cp.checkpoint);
            body.push_str(&format!(
                "<figure><img src=\"{}\" alt=\"reference, candidate, and heatmap for {} {}\"><figcaption>Reference, candidate, heatmap.</figcaption></figure>\n",
                escape(&src),
                escape(&cp.scene),
                escape(&cp.checkpoint)
            ));
        } else {
            body.push_str(
                "<p>No candidate picture was captured, so there is no side-by-side.</p>\n",
            );
        }
    }
    body.push_str("<h2>Blind spots</h2><ul>\n");
    for spot in BLIND_SPOTS {
        body.push_str(&format!("<li>{}</li>\n", escape(spot)));
    }
    body.push_str("</ul>\n");
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><title>Lighting reference check</title>\n\
         <style>body{{font:16px/1.45 sans-serif;margin:2rem;max-width:64rem}} table{{border-collapse:collapse;width:100%}} \
         td,th{{border-bottom:1px solid #ccc;text-align:left;padding:0.3rem;vertical-align:top}} img{{max-width:100%}} \
         .fail{{color:#8b1e1e;font-weight:700}} .human{{color:#8a5a00;font-weight:700}} .pass{{color:#1d5c32;font-weight:700}} \
         .verdict{{font-size:1.4rem}}</style></head><body>\n{body}</body></html>\n"
    )
}

fn run_json(report: &RunReport) -> String {
    let mut out = String::from("{\n");
    out.push_str(&format!(
        "  \"verdict\": \"{}\",\n",
        report.verdict().label()
    ));
    out.push_str(&format!(
        "  \"commit\": \"{}\",\n",
        escape_json(&report.commit)
    ));
    out.push_str(&format!("  \"mode\": \"{}\",\n", escape_json(&report.mode)));
    out.push_str(&format!(
        "  \"baseline\": {},\n",
        report
            .baseline
            .as_ref()
            .map(|c| format!("\"{}\"", escape_json(c)))
            .unwrap_or_else(|| "null".into())
    ));
    out.push_str("  \"checkpoints\": [\n");
    for (i, cp) in report.checkpoints.iter().enumerate() {
        let comma = if i + 1 == report.checkpoints.len() {
            ""
        } else {
            ","
        };
        out.push_str(&format!(
            "    {}{comma}\n",
            checkpoint_json(cp).replace('\n', "\n    ")
        ));
    }
    out.push_str("  ]\n}\n");
    out
}

fn checkpoint_json(cp: &CheckpointAudit) -> String {
    let mut metrics = String::from("{");
    for (i, id) in MetricId::all().iter().enumerate() {
        if i > 0 {
            metrics.push_str(", ");
        }
        metrics.push_str(&format!(
            "\"{}\": {}",
            id.key(),
            json_opt(metric_value(&cp.metrics, *id))
        ));
    }
    metrics.push('}');
    let mut reasons = String::from("[");
    for (i, reason) in cp.judgement.reasons.iter().enumerate() {
        if i > 0 {
            reasons.push_str(", ");
        }
        reasons.push_str(&format!("\"{}\"", escape_json(reason)));
    }
    reasons.push(']');
    format!(
        "{{\n  \"scene\": \"{}\",\n  \"checkpoint\": \"{}\",\n  \"verdict\": \"{}\",\n  \
         \"reference_method\": \"{}\",\n  \"reference_spp\": {},\n  \"reference_noise\": {},\n  \
         \"reasons\": {reasons},\n  \"metrics\": {metrics}\n}}",
        escape_json(&cp.scene),
        escape_json(&cp.checkpoint),
        cp.judgement.verdict.label(),
        escape_json(&cp.reference_method),
        cp.reference_spp,
        json_opt(Some(cp.reference_noise)),
    )
}

fn verdict_class(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Pass => "pass",
        Verdict::Fail => "fail",
        Verdict::NeedsHuman => "human",
    }
}

fn fmt_opt(value: Option<f64>) -> String {
    match value {
        Some(v) => format!("{v:.4}"),
        None => "—".into(),
    }
}

fn json_opt(value: Option<f64>) -> String {
    match value {
        Some(v) if v.is_finite() => {
            let mut s = format!("{v:.6}");
            while s.contains('.') && s.ends_with('0') {
                s.pop();
            }
            if s.ends_with('.') {
                s.push('0');
            }
            s
        }
        _ => "null".into(),
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn escape_json(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::metrics::{measure, MetricParams};
    use crate::picture::Picture;
    use crate::verdict::{judge, Judge};

    #[test]
    fn a_failure_is_explained_next_to_the_pictures() {
        let reference = Picture::solid(8, 4, [0.4, 0.4, 0.4]);
        let candidate = reference.scaled(1.5);
        let metrics = measure(&reference, &candidate, None, &[], &MetricParams::default()).unwrap();
        let judgement = judge(Judge {
            metrics: &metrics,
            config: &Config::builtin(),
            baseline: None,
            reference_spp: 8,
            reference_noise: 0.0,
            require: &[],
        });
        assert_eq!(judgement.verdict, Verdict::Fail);
        let dir = std::env::temp_dir().join(format!("lref-report-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let report = RunReport {
            commit: "abc".into(),
            baseline: None,
            mode: "test".into(),
            checkpoints: vec![CheckpointAudit {
                scene: "stress-still".into(),
                checkpoint: "hall".into(),
                note: "hall bench".into(),
                judgement,
                metrics,
                reference: Some(reference),
                candidate: Some(candidate),
                reference_spp: 8,
                reference_noise: 0.0,
                reference_method: "path".into(),
            }],
            skipped: vec![("sponza".into(), "no asset".into())],
            notes: vec!["live capture skipped".into()],
        };
        write_report(&dir, &report).unwrap();
        let md = std::fs::read_to_string(dir.join("summary.md")).unwrap();
        assert!(md.contains("**fail**"));
        assert!(md.contains("mean relative luminance"));
        assert!(md.contains("Blind spots"));
        assert!(md.contains("Sponza"));
        assert!(!md.contains("no side-by-side"));
        let html = std::fs::read_to_string(dir.join("summary.html")).unwrap();
        assert!(html.contains("mean relative luminance"));
        assert!(html.contains("side-by-side.png"));
        let png = std::fs::read(dir.join("stress-still/hall/heatmap.png")).unwrap();
        assert_eq!(&png[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        let side = std::fs::read(dir.join("stress-still/hall/side-by-side.png")).unwrap();
        assert_eq!(&side[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        let json = std::fs::read_to_string(dir.join("stress-still/hall/metrics.json")).unwrap();
        assert!(json.contains("\"verdict\": \"fail\""));
        assert!(json.contains("mean_rel"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
