//! Driver for the lighting reference-image checker.
//!
//! Scenes live here. The metric core is `genos-lighting-ref`, which does not
//! open a window. Reference pictures go under `./genos-debug/reference/<commit>/`
//! unless `--reference-dir` is set. Reports go under
//! `./genos-debug/refcheck/<timestamp>/` unless `--out` is set. Both paths are
//! relative to the current directory.

mod capture;
mod check;
mod generate;
mod scenes;
mod schedule;

use std::path::PathBuf;

pub use generate::{generate, GenerateOptions, TraceProfile};

/// Every available catalog checkpoint has a camera in this driver.
pub fn scenes_cover_the_catalog() -> bool {
    scenes::every_available_shot_has_a_camera()
}

/// `genos-stress lighting-ref …`
pub fn run(args: impl Iterator<Item = String>) -> Result<i32, String> {
    let opts = Options::parse(args)?;
    match opts.command {
        Command::Generate(generate) => {
            let records = generate::generate(&generate)?;
            let total: u64 = records.iter().map(|r| r.png_bytes + r.linear_bytes).sum();
            println!(
                "wrote {} references to {} ({})",
                records.len(),
                generate.reference_dir.display(),
                genos_lighting_ref::store::format_bytes(total)
            );
            Ok(0)
        }
        Command::Check(check) => check::run(&check),
    }
}

enum Command {
    Generate(generate::GenerateOptions),
    Check(check::CheckOptions),
}

struct Options {
    command: Command,
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut args = args.peekable();
        let check = match args.peek().map(String::as_str) {
            Some("generate") => {
                args.next();
                false
            }
            Some("check") => {
                args.next();
                true
            }
            Some(other) if other.starts_with('-') => false,
            Some(other) => return Err(format!("unknown lighting-ref command {other}")),
            None => false,
        };
        let mut mode = "owner".to_string();
        let mut reference_dir: Option<PathBuf> = None;
        let mut commit: Option<String> = None;
        let mut only_scene: Option<String> = None;
        let mut only_checkpoint: Option<String> = None;
        let mut width = None;
        let mut height = None;
        let mut spp = None;
        let mut out: Option<PathBuf> = None;
        let mut base: Option<String> = None;
        let mut baseline: Option<PathBuf> = None;
        let mut config: Option<PathBuf> = None;
        let mut candidate = "default".to_string();
        let mut reference_method = "path".to_string();
        let mut skip_capture = false;
        let mut fixed_dt = None;
        let mut warmup_frames = None;
        let mut settle_frames = None;
        let mut settle = None;
        let mut settle_cap = None;
        let mut capture_frames = None;
        while let Some(arg) = args.next() {
            let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
            match arg.as_str() {
                "--mode" => mode = value()?,
                "--reference-dir" => reference_dir = Some(PathBuf::from(value()?)),
                "--commit" => commit = Some(value()?),
                "--scene" => only_scene = Some(value()?),
                "--checkpoint" => only_checkpoint = Some(value()?),
                "--width" => width = Some(parse_u32(&value()?)?),
                "--height" => height = Some(parse_u32(&value()?)?),
                "--spp" => spp = Some(parse_u32(&value()?)?),
                "--out" => out = Some(PathBuf::from(value()?)),
                "--base" => base = Some(value()?),
                "--baseline" => baseline = Some(PathBuf::from(value()?)),
                "--config" => config = Some(PathBuf::from(value()?)),
                "--candidate" => candidate = value()?,
                "--reference" => reference_method = value()?,
                "--skip-capture" => skip_capture = true,
                "--fixed-dt" => fixed_dt = Some(value()?),
                "--warmup-frames" => warmup_frames = Some(value()?),
                "--settle-frames" => settle_frames = Some(value()?),
                "--settle" => settle = Some(value()?),
                "--settle-cap" => settle_cap = Some(value()?),
                "--capture-frames" => capture_frames = Some(value()?),
                other => return Err(format!("unknown argument {other}")),
            }
        }
        if !check {
            if out.is_some() {
                return Err(
                    "--out selects the audit report; generate writes references \
                     (--reference-dir overrides those)"
                        .into(),
                );
            }
            if base.is_some()
                || baseline.is_some()
                || config.is_some()
                || skip_capture
                || fixed_dt.is_some()
                || warmup_frames.is_some()
                || settle_frames.is_some()
                || settle.is_some()
                || settle_cap.is_some()
                || capture_frames.is_some()
            {
                return Err("that flag is for lighting-ref check".into());
            }
            if candidate != "default" || reference_method != "path" {
                return Err("that flag is for lighting-ref check".into());
            }
        }
        if reference_method != "path" {
            return Err(
                "the scored reference is the path trace; full effort is --candidate full-effort"
                    .into(),
            );
        }
        let full_effort = match candidate.as_str() {
            "default" => false,
            "full-effort" => true,
            other => {
                return Err(format!(
                    "unknown candidate {other} (default or full-effort)"
                ))
            }
        };
        let mut profile = match mode.as_str() {
            "owner" => TraceProfile::owner(),
            "lavapipe-small" => TraceProfile::lavapipe_small(),
            other => return Err(format!("unknown mode {other}")),
        };
        if let Some(width) = width {
            profile.width = width.max(1);
        }
        if let Some(height) = height {
            profile.height = height.max(1);
        }
        if let Some(spp) = spp {
            profile.spp = spp.max(1);
            profile.max_spp = profile.max_spp.max(profile.spp);
        }
        let commit = commit.unwrap_or_else(git_commit);
        let reference_dir =
            reference_dir.unwrap_or_else(|| genos_lighting_ref::reference_dir(&commit));
        let shared = generate::GenerateOptions {
            reference_dir: reference_dir.clone(),
            commit: commit.clone(),
            profile,
            only_scene: only_scene.clone(),
            only_checkpoint: only_checkpoint.clone(),
        };
        if !check {
            return Ok(Self {
                command: Command::Generate(shared),
            });
        }
        let baseline_label = base
            .clone()
            .or_else(|| baseline.as_ref().map(|p| p.display().to_string()));
        let baseline_path = baseline.or_else(|| {
            base.map(|sha| genos_lighting_ref::reference_dir(&sha).join("metrics.json"))
        });
        let env_fixed = env_text("GENOS_LIGHTING_REF_FIXED_DT");
        let env_warmup = env_text("GENOS_LIGHTING_REF_WARMUP_FRAMES");
        let env_settle_frames = env_text("GENOS_LIGHTING_REF_SETTLE_FRAMES");
        let env_settle = env_text("GENOS_LIGHTING_REF_SETTLE");
        let env_cap = env_text("GENOS_LIGHTING_REF_SETTLE_CAP");
        let env_capture = env_text("GENOS_LIGHTING_REF_CAPTURE_FRAMES");
        let plan = schedule::resolve_plan(
            &schedule::PlanText {
                fixed_dt: fixed_dt.as_deref(),
                warmup_frames: warmup_frames.as_deref(),
                settle_frames: settle_frames.as_deref(),
                settle: settle.as_deref(),
                settle_cap: settle_cap.as_deref(),
                capture_frames: capture_frames.as_deref(),
            },
            &schedule::PlanText {
                fixed_dt: env_fixed.as_deref(),
                warmup_frames: env_warmup.as_deref(),
                settle_frames: env_settle_frames.as_deref(),
                settle: env_settle.as_deref(),
                settle_cap: env_cap.as_deref(),
                capture_frames: env_capture.as_deref(),
            },
        )?;
        Ok(Self {
            command: Command::Check(check::CheckOptions {
                out,
                reference_dir,
                baseline_path,
                baseline_label,
                commit,
                mode,
                profile,
                config_path: config,
                full_effort,
                skip_capture,
                only_scene,
                only_checkpoint,
                plan,
            }),
        })
    }
}

fn parse_u32(text: &str) -> Result<u32, String> {
    text.parse::<u32>()
        .map_err(|_| format!("expected a number, got {text}"))
}

fn env_text(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn git_commit() -> String {
    if let Ok(value) = std::env::var("GENOS_ENGINE_COMMIT") {
        let value = value.trim();
        if !value.is_empty() {
            return value.to_string();
        }
    }
    std::process::Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn check_keeps_reports_in_the_given_directory() {
        let opts = Options::parse(
            words("check --mode lavapipe-small --base abc --out reports --candidate default")
                .into_iter(),
        )
        .unwrap();
        let Command::Check(check) = opts.command else {
            panic!("expected check");
        };
        assert_eq!(check.mode, "lavapipe-small");
        assert_eq!(check.out.unwrap(), PathBuf::from("reports"));
        let baseline = check.baseline_path.unwrap();
        assert!(baseline.ends_with("metrics.json"));
        assert!(baseline.starts_with("genos-debug/reference"));
        assert!(!check.full_effort);
        assert!(!check.skip_capture);
    }

    #[test]
    fn check_defaults_to_a_fixed_sixtieth() {
        let opts = Options::parse(words("check").into_iter()).unwrap();
        let Command::Check(check) = opts.command else {
            panic!("expected check");
        };
        assert!((check.plan.fixed_dt - schedule::DEFAULT_DT).abs() < 1.0e-6);
        assert_eq!(check.plan.warmup_frames, 0);
        assert_eq!(check.plan.capture_offsets, vec![1, 2, 4, 8]);
        assert_eq!(
            check.plan.settle,
            schedule::SettleMode::Metric {
                threshold: schedule::DEFAULT_SETTLE_THRESHOLD,
                cap: schedule::DEFAULT_SETTLE_CAP,
            }
        );
    }

    #[test]
    fn capture_frames_accept_the_event_form() {
        let opts = Options::parse(
            words("check --capture-frames event+1,+2,+4,+8 --settle-frames 6").into_iter(),
        )
        .unwrap();
        let Command::Check(check) = opts.command else {
            panic!("expected check");
        };
        assert_eq!(check.plan.capture_offsets, vec![1, 2, 4, 8]);
        assert_eq!(check.plan.settle, schedule::SettleMode::Frames(6));
    }

    #[test]
    fn settle_switches_cannot_both_be_set() {
        let err = match Options::parse(words("check --settle-frames 4 --settle 0.05").into_iter()) {
            Err(err) => err,
            Ok(_) => panic!("both settle switches were accepted"),
        };
        assert!(err.contains("settle"), "{err}");
    }

    #[test]
    fn generate_rejects_an_audit_directory() {
        let err = match Options::parse(words("--out reports").into_iter()) {
            Err(err) => err,
            Ok(_) => panic!("generate accepted --out"),
        };
        assert!(err.contains("--out"));
    }

    #[test]
    fn generate_rejects_a_frame_flag() {
        let err = match Options::parse(words("--fixed-dt 0.1").into_iter()) {
            Err(err) => err,
            Ok(_) => panic!("generate accepted --fixed-dt"),
        };
        assert!(err.contains("check"), "{err}");
    }
}
