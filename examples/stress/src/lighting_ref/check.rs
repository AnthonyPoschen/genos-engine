//! One check: self-test, path-traced references, live frames, a report.

use std::path::{Path, PathBuf};

use genos_lighting_ref::baseline::{self, MetricsFile};
use genos_lighting_ref::config::Config;
use genos_lighting_ref::metrics::{measure, FrameMetrics};
use genos_lighting_ref::policy::{self, ExitFacts};
use genos_lighting_ref::report::{self, CheckpointAudit, RunReport};
use genos_lighting_ref::store::{self, TraceNote};
use genos_lighting_ref::verdict::{judge, Judge, MetricId};
use genos_lighting_ref::Picture;

use super::capture::{self, CaptureOptions, Job, LiveShot};
use super::generate::{self, GenerateOptions, TraceProfile};
use super::schedule::{self, FramePlan, FrameRecord};

const STILL_REQUIRE: &[MetricId] = &[MetricId::Flicker];
const EVENT_REQUIRE: &[MetricId] = &[
    MetricId::Converge1,
    MetricId::Converge2,
    MetricId::Converge4,
    MetricId::Converge8,
];

pub struct CheckOptions {
    pub out: Option<PathBuf>,
    pub reference_dir: PathBuf,
    pub baseline_path: Option<PathBuf>,
    pub baseline_label: Option<String>,
    pub commit: String,
    pub mode: String,
    pub profile: TraceProfile,
    pub config_path: Option<PathBuf>,
    pub full_effort: bool,
    /// Renderer bounce count from `--bounces` or `GENOS_LIGHTING_REF_BOUNCES`.
    /// The path trace keeps the profile's bounce count.
    pub bounces: Option<u32>,
    pub skip_capture: bool,
    pub only_scene: Option<String>,
    pub only_checkpoint: Option<String>,
    /// Catalog order when empty. Otherwise these checkpoints, in this order.
    pub sequence: Vec<(String, String)>,
    pub plan: FramePlan,
    /// Keep lighting caches from one checkpoint to the next. The default
    /// opens a fresh renderer for each checkpoint.
    pub warm: bool,
    /// Replace reference pictures that are already in the directory.
    pub regen: bool,
}

#[derive(Clone)]
struct Wanted {
    scene: String,
    checkpoint: String,
    note: String,
    still: bool,
    event: Option<&'static str>,
}

pub fn run(opts: &CheckOptions) -> Result<i32, String> {
    let config = match &opts.config_path {
        Some(path) => Config::load(path)?,
        None => Config::builtin(),
    };
    let out = opts
        .out
        .clone()
        .unwrap_or_else(|| genos_lighting_ref::refcheck_dir(&genos_lighting_ref::utc_stamp()));
    let (self_test, _) = genos_lighting_ref::selftest::write(&out.join("self-test"))?;
    println!("{}", self_test.summary_line());

    let wanted = wanted(opts);
    if wanted.is_empty() {
        println!("lighting-ref: no checkpoints matched");
    }
    let trace_these = if opts.regen {
        wanted.clone()
    } else {
        let (missing, errors) = reference_gaps(&opts.reference_dir, opts, &wanted)?;
        if !errors.is_empty() {
            return Err(errors.join("\n"));
        }
        missing
    };
    let reused = trace_these.is_empty();
    if reused {
        println!("reusing references in {}", opts.reference_dir.display());
    } else {
        let sequence = trace_these
            .iter()
            .map(|item| (item.scene.clone(), item.checkpoint.clone()))
            .collect();
        generate::generate(&GenerateOptions {
            reference_dir: opts.reference_dir.clone(),
            commit: opts.commit.clone(),
            profile: opts.profile.clone(),
            only_scene: None,
            only_checkpoint: None,
            sequence,
            regen: opts.regen,
        })?;
    }

    let mut notes = vec![self_test.summary_line()];
    if reused {
        notes.push(format!(
            "Reused reference pictures in {}.",
            opts.reference_dir.display()
        ));
    } else if opts.regen {
        notes.push(format!(
            "Replaced references in {} (--regen).",
            opts.reference_dir.display()
        ));
    } else if trace_these.len() == wanted.len() {
        notes.push(format!(
            "Traced references into {}.",
            opts.reference_dir.display()
        ));
    } else {
        notes.push(format!(
            "Traced missing references into {}. Existing pictures were kept. Pass --regen to replace them.",
            opts.reference_dir.display()
        ));
    }
    notes.push(candidate_note(opts.full_effort));
    if let Some(count) = opts.bounces {
        notes.push(format!(
            "Renderer bounce count {}. The path trace keeps its own bounce count. direct-only stays bounce 0 with ambient off.",
            bounce_label(count)
        ));
    }
    notes.push(plan_note(&opts.plan));
    notes.push(if opts.warm {
        "Warm chain: one renderer keeps its lighting caches across checkpoints."
            .to_string()
    } else {
        "Each checkpoint opens a fresh renderer, so the verdict does not depend on the checkpoint before it."
            .to_string()
    });

    ensure_reference_metrics(opts, &wanted)?;
    let (baselines, baseline_label) = load_baseline(opts, &mut notes)?;
    let live = capture_live(opts, &wanted, &config.params, &mut notes)?;
    if live.iter().any(|shot| !shot.log.is_empty()) {
        let rows: Vec<FrameRecord> = live
            .iter()
            .flat_map(|shot| shot.log.iter().cloned())
            .collect();
        let path = out.join("frames.tsv");
        schedule::write_frames_tsv(&path, &rows)?;
        notes.push(format!("Frame log {}.", path.display()));
    }
    let mut checkpoints = Vec::new();
    let mut saved = Vec::new();
    let mut scored = 0usize;
    for item in &wanted {
        let Some(shot) = live
            .iter()
            .find(|shot| shot.scene == item.scene && shot.checkpoint == item.checkpoint)
        else {
            continue;
        };
        if shot.frames.is_empty() && shot.settled.is_none() {
            notes.push(format!(
                "{} {}: capture produced no picture",
                item.scene, item.checkpoint
            ));
            continue;
        }
        let reference = store::read_reference(&opts.reference_dir, &item.scene, &item.checkpoint)?;
        let trace = store::read_trace(&opts.reference_dir, &item.scene, &item.checkpoint)
            .unwrap_or(TraceNote {
                method: "path".into(),
                spp: opts.profile.spp,
                noise: 0.0,
                seconds: 0.0,
                max_bounces: opts.profile.max_bounces,
                sample_cap: opts.profile.max_spp,
                build: opts.commit.clone(),
            });
        let candidate = candidate_picture(item, shot);
        let still = (item.still && shot.frames.len() >= 2).then_some(shot.frames.as_slice());
        let after = after_frames(item, shot);
        let after_refs: Vec<(u32, &Picture)> = after.iter().map(|(k, p)| (*k, *p)).collect();
        let mut metrics = match measure(&reference, &candidate, still, &after_refs, &config.params)
        {
            Ok(metrics) => metrics,
            Err(err) => {
                notes.push(format!("{} {}: {err}", item.scene, item.checkpoint));
                continue;
            }
        };
        metrics.cold_frames = shot.cold_frames;
        metrics.cold_settled = shot.cold_settled;
        let require = require_of(item);
        let baseline = baselines.as_ref().and_then(|file| {
            file.rows
                .iter()
                .find(|row| row.scene == item.scene && row.checkpoint == item.checkpoint)
        });
        let judgement = judge(Judge {
            metrics: &metrics,
            config: &config,
            baseline: baseline.map(|row| &row.baseline),
            reference_spp: trace.spp,
            reference_noise: f64::from(trace.noise),
            require,
            flip_gates: item.scene == "direct-only",
        });
        println!(
            "{} {}  {}",
            item.scene,
            item.checkpoint,
            judgement.verdict.label()
        );
        saved.push((item.scene.clone(), item.checkpoint.clone(), metrics.clone()));
        checkpoints.push(CheckpointAudit {
            scene: item.scene.clone(),
            checkpoint: item.checkpoint.clone(),
            note: format!("{} {}", item.note, shot.note),
            judgement,
            metrics,
            reference: Some(reference),
            candidate: Some(candidate),
            reference_spp: trace.spp,
            reference_noise: f64::from(trace.noise),
            reference_method: trace.method,
        });
        scored += 1;
    }
    if !saved.is_empty() {
        let refs: Vec<(&str, &str, &FrameMetrics)> = saved
            .iter()
            .map(|(scene, checkpoint, metrics)| (scene.as_str(), checkpoint.as_str(), metrics))
            .collect();
        let path = opts.reference_dir.join("metrics.json");
        baseline::upsert_metrics(&path, &opts.commit, &refs)?;
        notes.push(format!("Wrote candidate metrics to {}.", path.display()));
    }

    let skipped = skipped_scenes(opts);
    let report = RunReport {
        commit: opts.commit.clone(),
        baseline: baseline_label,
        mode: opts.mode.clone(),
        checkpoints,
        skipped,
        notes,
        unscored_fails: true,
    };
    let verdict = report.verdict();
    report::write_report(&out, &report)?;
    let code = policy::exit_code(ExitFacts {
        self_test_ok: self_test.ok,
        scene_verdict: verdict,
        scored: scored > 0,
        complete: scored == wanted.len(),
    });
    println!("lighting-ref: {} (exit {code})", verdict.label());
    println!("report {}", out.join("summary.md").display());
    Ok(code)
}

fn wanted(opts: &CheckOptions) -> Vec<Wanted> {
    if !opts.sequence.is_empty() {
        return opts
            .sequence
            .iter()
            .filter_map(|(scene, name)| {
                genos_lighting_ref::checkpoints()
                    .iter()
                    .find(|cp| cp.scene == scene && cp.name == name)
                    .map(|cp| Wanted {
                        scene: cp.scene.to_string(),
                        checkpoint: cp.name.to_string(),
                        note: cp.note.to_string(),
                        still: cp.still,
                        event: cp.event,
                    })
            })
            .collect();
    }
    genos_lighting_ref::available_checkpoints()
        .filter(|cp| {
            opts.only_scene.as_ref().is_none_or(|id| id == cp.scene)
                && opts
                    .only_checkpoint
                    .as_ref()
                    .is_none_or(|name| name == cp.name)
        })
        .map(|cp| Wanted {
            scene: cp.scene.to_string(),
            checkpoint: cp.name.to_string(),
            note: cp.note.to_string(),
            still: cp.still,
            event: cp.event,
        })
        .collect()
}

/// A reused reference directory from before base metrics were stored still
/// grows a `metrics.json`. A file already there is left alone: it may be a
/// previous candidate baseline.
fn ensure_reference_metrics(opts: &CheckOptions, wanted: &[Wanted]) -> Result<(), String> {
    let path = opts.reference_dir.join("metrics.json");
    if path.exists() {
        return Ok(());
    }
    let mut owned = Vec::new();
    for item in wanted {
        let Ok(picture) = store::read_reference(&opts.reference_dir, &item.scene, &item.checkpoint)
        else {
            continue;
        };
        let noise = store::read_trace(&opts.reference_dir, &item.scene, &item.checkpoint)
            .map(|trace| f64::from(trace.noise))
            .unwrap_or(0.0);
        let metrics = baseline::reference_metrics(&picture, noise)?;
        owned.push((item.scene.clone(), item.checkpoint.clone(), metrics));
    }
    let refs: Vec<(&str, &str, &FrameMetrics)> = owned
        .iter()
        .map(|(scene, checkpoint, metrics)| (scene.as_str(), checkpoint.as_str(), metrics))
        .collect();
    baseline::write_metrics(&path, &opts.commit, &refs)?;
    println!("base metrics {}", path.display());
    Ok(())
}

/// Pictures that are absent, and pictures that are present but do not match
/// this run. A mismatch is an error. It is not traced over.
fn reference_gaps(
    dir: &Path,
    opts: &CheckOptions,
    wanted: &[Wanted],
) -> Result<(Vec<Wanted>, Vec<String>), String> {
    let mut missing = Vec::new();
    let mut errors = Vec::new();
    for item in wanted {
        let picture = match store::read_reference(dir, &item.scene, &item.checkpoint) {
            Ok(picture) => picture,
            Err(_) => {
                missing.push(item.clone());
                continue;
            }
        };
        let Some(shot) = super::scenes::shot(&item.scene, &item.checkpoint) else {
            errors.push(format!(
                "{} {}: no camera, so the stored reference cannot be checked",
                item.scene, item.checkpoint
            ));
            continue;
        };
        let setup = super::scenes::reference_setup(&shot, &opts.profile);
        let trace = match store::read_trace(dir, &item.scene, &item.checkpoint) {
            Ok(trace) => trace,
            Err(err) => {
                errors.push(format!(
                    "{}/{} in {} has a picture and no usable trace ({err}). Pass --regen to replace it.",
                    item.scene,
                    item.checkpoint,
                    dir.display()
                ));
                continue;
            }
        };
        let mut why = Vec::new();
        if picture.width != opts.profile.width || picture.height != opts.profile.height {
            why.push(format!(
                "size {}x{} vs {}x{}",
                picture.width, picture.height, opts.profile.width, opts.profile.height
            ));
        }
        if trace.sample_cap != setup.max_spp {
            why.push(format!(
                "sample cap {} vs {}",
                trace.sample_cap, setup.max_spp
            ));
        }
        if trace.build != opts.commit {
            why.push(format!("build {} vs {}", trace.build, opts.commit));
        }
        if trace.max_bounces != setup.max_bounces {
            why.push(format!(
                "bounces {} vs {}",
                trace.max_bounces, setup.max_bounces
            ));
        }
        if !why.is_empty() {
            errors.push(format!(
                "{}/{} in {} does not match this run ({}). Pass --regen to replace it.",
                item.scene,
                item.checkpoint,
                dir.display(),
                why.join(", ")
            ));
        }
    }
    Ok((missing, errors))
}

fn plan_note(plan: &FramePlan) -> String {
    let settle = match plan.settle {
        schedule::SettleMode::Frames(n) => format!("{n} settle frames"),
        schedule::SettleMode::Metric { threshold, cap } => {
            format!("score frame {cap} of {cap}; blurred relative luminance under {threshold}, raised by that checkpoint's reference noise, is the cold-start line")
        }
    };
    let offsets = plan
        .capture_offsets
        .iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(",+");
    format!(
        "Frame clock {} s, warmup {} frames, {settle}, captures at event+{offsets}. A metric event holds the pose before the jump for the settle cap, then jumps. Lavapipe-small and owner mode use this same list. Wall time is logged.",
        schedule::format_dt(plan.fixed_dt),
        plan.warmup_frames,
    )
}

fn bounce_label(count: u32) -> String {
    if count == genos_scene::LIGHTING_BOUNCES_UNLIMITED {
        "unlimited".into()
    } else {
        count.to_string()
    }
}

fn candidate_note(full_effort: bool) -> String {
    if full_effort {
        "Candidate lighting is full effort: maximum knobs, not a second ground truth.".into()
    } else {
        "Candidate lighting is the build default. GENOS_LIGHTING_* still overlays it.".into()
    }
}

fn load_baseline(
    opts: &CheckOptions,
    notes: &mut Vec<String>,
) -> Result<(Option<MetricsFile>, Option<String>), String> {
    let Some(path) = &opts.baseline_path else {
        notes.push("No baseline given, so regressions were not judged.".into());
        return Ok((None, None));
    };
    if !path.exists() {
        notes.push(format!(
            "No metrics at {}, so regressions were not judged.",
            path.display()
        ));
        return Ok((None, None));
    }
    let file = baseline::read_metrics(path)?;
    let label = opts
        .baseline_label
        .clone()
        .unwrap_or_else(|| file.commit.clone());
    notes.push(format!("Regressions use {} ({}).", path.display(), label));
    Ok((Some(file), Some(label)))
}

fn capture_live(
    opts: &CheckOptions,
    wanted: &[Wanted],
    params: &genos_lighting_ref::MetricParams,
    notes: &mut Vec<String>,
) -> Result<Vec<LiveShot>, String> {
    if opts.skip_capture {
        notes.push("Live capture skipped (--skip-capture).".into());
        return Ok(Vec::new());
    }
    let jobs = wanted
        .iter()
        .map(|item| Job {
            scene: item.scene.clone(),
            checkpoint: item.checkpoint.clone(),
            still: item.still,
            event: item.event,
            reference: store::read_reference(&opts.reference_dir, &item.scene, &item.checkpoint)
                .ok(),
            reference_noise: store::read_trace(&opts.reference_dir, &item.scene, &item.checkpoint)
                .map(|trace| f64::from(trace.noise))
                .unwrap_or(0.0),
        })
        .collect();
    match capture::capture(&CaptureOptions {
        width: opts.profile.width,
        height: opts.profile.height,
        full_effort: opts.full_effort,
        bounces: opts.bounces,
        lavapipe_small: opts.mode == "lavapipe-small",
        plan: opts.plan.clone(),
        params: *params,
        jobs,
        warm: opts.warm,
        lighting_seed: None,
    }) {
        Ok(outcome) => {
            for (scene, checkpoint, reason) in outcome.skipped {
                notes.push(format!("{scene} {checkpoint}: {reason}"));
            }
            if outcome.shots.is_empty() {
                notes.push("Live capture produced no pictures.".into());
            }
            Ok(outcome.shots)
        }
        Err(err) => {
            notes.push(format!("Live capture skipped: {err}"));
            Ok(Vec::new())
        }
    }
}

fn candidate_picture(item: &Wanted, shot: &LiveShot) -> Picture {
    if let Some(settled) = &shot.settled {
        return settled.clone();
    }
    if item.still && shot.frames.len() >= 2 {
        if let Ok(average) = Picture::average(&shot.frames) {
            return average;
        }
    }
    shot.frames
        .last()
        .cloned()
        .or_else(|| shot.settled.clone())
        .expect("caller checks the shot has a picture")
}

fn after_frames<'a>(item: &Wanted, shot: &'a LiveShot) -> Vec<(u32, &'a Picture)> {
    if item.event.is_none() {
        return Vec::new();
    }
    shot.event_frames
        .iter()
        .map(|(k, picture)| (*k, picture))
        .collect()
}

fn require_of(item: &Wanted) -> &'static [MetricId] {
    if item.event.is_some() {
        EVENT_REQUIRE
    } else if item.still {
        STILL_REQUIRE
    } else {
        &[]
    }
}

fn skipped_scenes(opts: &CheckOptions) -> Vec<(String, String)> {
    genos_lighting_ref::scenes()
        .iter()
        .filter(|scene| !scene.available)
        .filter(|scene| opts.only_scene.as_ref().is_none_or(|id| id == scene.id))
        .map(|scene| (scene.id.to_string(), scene.skip_reason.to_string()))
        .collect()
}
