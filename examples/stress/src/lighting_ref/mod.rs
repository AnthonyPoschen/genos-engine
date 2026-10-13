//! Driver for the lighting reference-image checker.
//!
//! Scenes live here. The metric core is `genos-lighting-ref`, which does not
//! open a window. Reference pictures go under `./genos-debug/reference/<commit>/`
//! unless `--reference-dir` is set. That path is relative to the current
//! directory.

mod generate;
mod scenes;

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
        Command::Generate => {
            let records = generate(&opts.generate)?;
            let total: u64 = records.iter().map(|r| r.png_bytes + r.linear_bytes).sum();
            println!(
                "wrote {} references to {} ({})",
                records.len(),
                opts.generate.reference_dir.display(),
                genos_lighting_ref::store::format_bytes(total)
            );
            Ok(0)
        }
    }
}

enum Command {
    Generate,
}

struct Options {
    command: Command,
    generate: generate::GenerateOptions,
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut args = args.peekable();
        let command = match args.peek().map(String::as_str) {
            Some("generate") => {
                args.next();
                Command::Generate
            }
            Some(other) if other.starts_with('-') => Command::Generate,
            Some(other) => return Err(format!("unknown lighting-ref command {other}")),
            None => Command::Generate,
        };
        let mut mode = "owner".to_string();
        let mut reference_dir: Option<PathBuf> = None;
        let mut commit: Option<String> = None;
        let mut only_scene: Option<String> = None;
        let mut only_checkpoint: Option<String> = None;
        let mut width = None;
        let mut height = None;
        let mut spp = None;
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
                "--out" => {
                    return Err(
                        "--out selects the audit report; generate writes references \
                         (--reference-dir overrides those)"
                            .to_string(),
                    );
                }
                other => return Err(format!("unknown argument {other}")),
            }
        }
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
        Ok(Self {
            command,
            generate: generate::GenerateOptions {
                reference_dir,
                commit,
                profile,
                only_scene,
                only_checkpoint,
            },
        })
    }
}

fn parse_u32(text: &str) -> Result<u32, String> {
    text.parse::<u32>()
        .map_err(|_| format!("expected a number, got {text}"))
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
