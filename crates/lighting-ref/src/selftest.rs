//! Planted defects. The checker has to catch each one without a GPU.
//!
//! A picture verdict of fail here is the intended result: the defect was
//! caught. The self-test passes only when every planted case has the verdict
//! it should, and a clean picture passes. Borderline stays needs-human.

use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::metrics::{measure, MetricParams};
use crate::picture::Picture;
use crate::report::{write_report, CheckpointAudit, RunReport};
use crate::verdict::{judge, Judge, Judgement, MetricId, Verdict};

const SPP: u32 = 8;

struct Plant {
    name: &'static str,
    note: &'static str,
    expect: Verdict,
    /// Substrings that must show up in the reasons. A miss means the defect
    /// was not caught by the metric that should see it.
    caught_by: &'static [&'static str],
    require: &'static [MetricId],
    reference: Picture,
    candidate: Picture,
    still: Option<Vec<Picture>>,
    after: Vec<(u32, Picture)>,
}

pub struct SelfTest {
    pub ok: bool,
    pub misses: Vec<String>,
    pub report: RunReport,
}

impl SelfTest {
    pub fn summary_line(&self) -> String {
        if self.ok {
            format!(
                "self-test passed: {} planted pictures, clean passed, borderline stayed needs human",
                self.report.checkpoints.len()
            )
        } else {
            format!("self-test missed: {}", self.misses.join("; "))
        }
    }
}

/// Judge the planted pictures. Does not write a folder.
pub fn run() -> SelfTest {
    let config = Config::builtin();
    let plants = plants();
    let mut checkpoints = Vec::new();
    let mut misses = Vec::new();
    for plant in &plants {
        let (metrics, judgement) = score(plant, &config);
        misses.extend(grade(plant.name, plant.expect, plant.caught_by, &judgement));
        checkpoints.push(CheckpointAudit {
            scene: "self-test".into(),
            checkpoint: plant.name.into(),
            note: plant.note.into(),
            judgement,
            metrics,
            reference: Some(plant.reference.clone()),
            candidate: Some(plant.candidate.clone()),
            reference_spp: SPP,
            reference_noise: 0.0,
            reference_method: "planted".into(),
        });
    }
    let ok = misses.is_empty();
    let mut notes = vec![
        "These pictures are planted defects, not frames from the runtime.".into(),
        "A fail on a planted picture means the checker caught it.".into(),
        "The near-bias picture must stay needs human. It must not auto-pass.".into(),
    ];
    if ok {
        notes.push("Self-test passed.".into());
    } else {
        notes.push(format!("Self-test missed: {}", misses.join("; ")));
    }
    SelfTest {
        ok,
        misses,
        report: RunReport {
            commit: "self-test".into(),
            baseline: None,
            mode: "self-test".into(),
            checkpoints,
            skipped: Vec::new(),
            notes,
            unscored_fails: false,
        },
    }
}

pub fn write(dir: &Path) -> Result<(SelfTest, PathBuf), String> {
    let result = run();
    let path = write_report(dir, &result.report)?;
    Ok((result, path))
}

fn score(plant: &Plant, config: &Config) -> (crate::metrics::FrameMetrics, Judgement) {
    let still = plant.still.as_deref();
    let after_refs: Vec<(u32, &Picture)> = plant.after.iter().map(|(k, p)| (*k, p)).collect();
    let metrics = measure(
        &plant.reference,
        &plant.candidate,
        still,
        &after_refs,
        &MetricParams {
            rel_floor: config.params.rel_floor,
            dark_max: config.params.dark_max,
            unfilled_candidate_max: config.params.unfilled_candidate_max,
            unfilled_reference_min: config.params.unfilled_reference_min,
        },
    )
    .expect("planted pictures share a size");
    let judgement = judge(Judge {
        metrics: &metrics,
        config,
        baseline: None,
        reference_spp: SPP,
        reference_noise: 0.0,
        require: plant.require,
    });
    (metrics, judgement)
}

fn grade(name: &str, expect: Verdict, caught_by: &[&str], judgement: &Judgement) -> Vec<String> {
    let mut misses = Vec::new();
    if judgement.verdict != expect {
        misses.push(format!(
            "{name}: verdict {}, expected {}",
            judgement.verdict.label(),
            expect.label()
        ));
    }
    for needle in caught_by {
        if !judgement
            .reasons
            .iter()
            .any(|reason| reason.contains(needle))
        {
            misses.push(format!(
                "{name}: reasons do not mention \"{needle}\" ({})",
                judgement.reasons.join(" | ")
            ));
        }
    }
    misses
}

fn plants() -> Vec<Plant> {
    vec![
        clean(),
        brightness_bias(),
        near_bias(),
        held_stale(),
        black_block(),
        flicker(),
        missing_bounce(),
    ]
}

fn gray(w: u32, h: u32, v: f32) -> Picture {
    Picture::solid(w, h, [v, v, v])
}

fn clean() -> Plant {
    let picture = gray(16, 8, 0.4);
    Plant {
        name: "clean",
        note: "Planted: a copy of the reference. Expected verdict: pass.",
        expect: Verdict::Pass,
        caught_by: &["every gated metric is under its band"],
        require: &[MetricId::Flicker],
        candidate: picture.clone(),
        still: Some(vec![picture.clone(), picture.clone()]),
        after: Vec::new(),
        reference: picture,
    }
}

fn brightness_bias() -> Plant {
    let reference = gray(16, 8, 0.4);
    let candidate = reference.scaled(1.4);
    Plant {
        name: "brightness-bias",
        note: "Planted: every channel ×1.4. Expected verdict: fail on mean relative luminance.",
        expect: Verdict::Fail,
        caught_by: &["mean relative luminance is", "past"],
        require: &[MetricId::MeanRel],
        reference,
        candidate,
        still: None,
        after: Vec::new(),
    }
}

fn near_bias() -> Plant {
    let reference = gray(16, 8, 0.4);
    let candidate = reference.scaled(1.075);
    Plant {
        name: "near-bias",
        note: "Planted: every channel ×1.075, inside the band around the fail line. Expected verdict: needs human, not a pass.",
        expect: Verdict::NeedsHuman,
        caught_by: &["mean relative luminance is", "needs a person"],
        require: &[MetricId::MeanRel],
        reference,
        candidate,
        still: None,
        after: Vec::new(),
    }
}

fn held_stale() -> Plant {
    let reference = gray(16, 8, 0.55);
    let stale = gray(16, 8, 0.20);
    Plant {
        name: "held-stale",
        note: "Planted: the picture stays at the pre-step value after the boxes jump. Expected verdict: fail on the error 1, 2, 4, and 8 frames later.",
        expect: Verdict::Fail,
        caught_by: &[
            "error 1 frame after the event is",
            "error 8 frames after the event is",
            "past",
        ],
        require: &[
            MetricId::Converge1,
            MetricId::Converge2,
            MetricId::Converge4,
            MetricId::Converge8,
        ],
        candidate: stale.clone(),
        still: Some(vec![stale.clone(), stale.clone()]),
        after: vec![
            (1, stale.clone()),
            (2, stale.clone()),
            (4, stale.clone()),
            (8, stale.clone()),
        ],
        reference,
    }
}

fn black_block() -> Plant {
    let reference = gray(16, 8, 0.45);
    let mut candidate = reference.clone();
    candidate.fill_rect(1, 1, 4, 4, [0.0, 0.0, 0.0]);
    Plant {
        name: "black-block",
        note: "Planted: a 4×4 black rectangle on a lit field. Expected verdict: fail on the unfilled-pixel share.",
        expect: Verdict::Fail,
        caught_by: &["unfilled-pixel share is", "past"],
        require: &[MetricId::UnfilledShare],
        reference,
        candidate,
        still: None,
        after: Vec::new(),
    }
}

fn flicker() -> Plant {
    let reference = gray(16, 8, 0.4);
    let hi = gray(16, 8, 0.5);
    let lo = gray(16, 8, 0.3);
    let candidate = Picture::average(&[hi.clone(), lo.clone()]).expect("same size");
    Plant {
        name: "flicker",
        note: "Planted: a still camera alternates 0.5 and 0.3. The average matches the reference, so mean error stays quiet. Expected verdict: fail on still-camera flicker.",
        expect: Verdict::Fail,
        caught_by: &["still-camera flicker is", "past"],
        require: &[MetricId::Flicker],
        reference,
        candidate,
        still: Some(vec![hi, lo]),
        after: Vec::new(),
    }
}

fn missing_bounce() -> Plant {
    let mut reference = Picture::solid(32, 8, [0.50, 0.48, 0.45]);
    reference.fill_rect(16, 0, 16, 8, [0.09, 0.015, 0.012]);
    let mut candidate = Picture::solid(32, 8, [0.50, 0.48, 0.45]);
    candidate.fill_rect(16, 0, 16, 8, [0.003, 0.003, 0.003]);
    Plant {
        name: "missing-bounce",
        note: "Planted: the lit half matches; the shadow keeps only a dark direct remainder and loses the red bounce. Expected verdict: fail on dark-region relative luminance.",
        expect: Verdict::Fail,
        caught_by: &["dark-region relative luminance is", "past"],
        require: &[MetricId::MeanRelDark],
        reference,
        candidate,
        still: None,
        after: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verdict::MetricId;

    #[test]
    fn each_planted_defect_is_caught_and_the_clean_picture_passes() {
        let result = run();
        assert!(result.ok, "{}", result.summary_line());
        let verdict = |name: &str| {
            result
                .report
                .checkpoints
                .iter()
                .find(|cp| cp.checkpoint == name)
                .unwrap_or_else(|| panic!("missing {name}"))
        };
        assert_eq!(verdict("clean").judgement.verdict, Verdict::Pass);
        assert_eq!(verdict("brightness-bias").judgement.verdict, Verdict::Fail);
        assert_eq!(verdict("near-bias").judgement.verdict, Verdict::NeedsHuman);
        assert_ne!(verdict("near-bias").judgement.verdict, Verdict::Pass);
        assert_eq!(verdict("held-stale").judgement.verdict, Verdict::Fail);
        assert_eq!(verdict("black-block").judgement.verdict, Verdict::Fail);
        assert_eq!(verdict("flicker").judgement.verdict, Verdict::Fail);
        assert_eq!(verdict("missing-bounce").judgement.verdict, Verdict::Fail);

        let flicker = verdict("flicker");
        let row = |id: MetricId| {
            flicker
                .judgement
                .rows
                .iter()
                .find(|row| row.id == id)
                .unwrap()
        };
        assert_eq!(row(MetricId::Flicker).verdict, Verdict::Fail);
        assert_eq!(row(MetricId::MeanRel).verdict, Verdict::Pass);

        let bounce = verdict("missing-bounce");
        assert!(bounce.metrics.mean_rel_dark.unwrap() > 0.5);
        assert!(bounce.metrics.mean_rel_lit.unwrap() < 0.02);
        let dark = bounce
            .judgement
            .rows
            .iter()
            .find(|row| row.id == MetricId::MeanRelDark)
            .unwrap();
        assert_eq!(dark.verdict, Verdict::Fail);

        let block = verdict("black-block");
        assert!(block.metrics.unfilled.unwrap() >= 16);

        let dir = std::env::temp_dir().join(format!("lref-selftest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (written, _) = write(&dir).unwrap();
        assert!(written.ok);
        let md = std::fs::read_to_string(dir.join("summary.md")).unwrap();
        assert!(md.contains("Self-test passed"));
        assert!(md.contains("brightness-bias"));
        assert!(md.contains("held-stale"));
        assert!(md.contains("black-block"));
        assert!(md.contains("missing-bounce"));
        assert!(md.contains("needs human"));
        assert!(md.contains("**fail**"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_wrong_verdict_is_a_miss() {
        let config = Config::builtin();
        let plant = clean();
        let (_, judgement) = score(&plant, &config);
        let misses = grade("clean", Verdict::Fail, &["no such phrase"], &judgement);
        assert!(misses.iter().any(|m| m.contains("expected fail")));
        assert!(misses.iter().any(|m| m.contains("no such phrase")));
    }
}
