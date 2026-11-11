//! Headless screen-tile fill reference.
//!
//! `run` writes `metrics.json`. The default directory is `./genos-debug/fill-ref`.
//! `--out` overrides it. `GENOS_FILL_REF_OUT` applies only when `--out` was omitted.
//! `--tile-fill` defaults off. `GENOS_FILL_TILE` applies only when `--tile-fill`
//! was omitted. The tool replaces `metrics.json`. It does not write under `$HOME`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use genos_fill_ref::{grade, CommitRule, Config, Grade, MEASURED_COMMIT};

const DEFAULT_OUT: &str = "genos-debug/fill-ref";

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(err) => {
            eprintln!("fill-ref: {err}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<u8, String> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return Ok(0);
    }
    if args[0] != "run" {
        return Err(format!("unknown command {}", args[0]));
    }
    let opts = parse(&args[1..])?;
    let report = grade(&opts.config);
    print_grade(&report);
    write_metrics(&opts.out, &report)?;
    println!("wrote {}", opts.out.display());
    if report.pass { Ok(0) } else { Ok(1) }
}

struct Opts {
    out: PathBuf,
    config: Config,
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut out: Option<PathBuf> = None;
    let mut tile_fill: Option<bool> = None;
    let mut commit: Option<CommitRule> = None;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "--out" => {
                out = Some(PathBuf::from(need_value(args, &mut index, "--out")?));
            }
            "--tile-fill" => {
                tile_fill = Some(parse_on(need_value(args, &mut index, "--tile-fill")?)?);
            }
            "--commit" => {
                commit = Some(parse_commit(need_value(args, &mut index, "--commit")?)?);
            }
            other => return Err(format!("unknown argument {other}")),
        }
        index += 1;
    }
    let out = out.unwrap_or_else(default_out);
    let tile_fill = tile_fill.unwrap_or_else(tile_fill_from_env);
    Ok(Opts {
        out,
        config: Config {
            tile_fill,
            commit: commit.unwrap_or(MEASURED_COMMIT),
        },
    })
}

fn need_value<'a>(args: &'a [String], index: &mut usize, flag: &str) -> Result<&'a str, String> {
    *index += 1;
    args.get(*index)
        .map(String::as_str)
        .ok_or_else(|| format!("{flag} needs a value"))
}

fn parse_on(value: &str) -> Result<bool, String> {
    match value {
        "on" | "1" | "true" => Ok(true),
        "off" | "0" | "false" => Ok(false),
        other => Err(format!("--tile-fill expects on or off, got {other}")),
    }
}

fn parse_commit(value: &str) -> Result<CommitRule, String> {
    match value {
        "immediate" => Ok(CommitRule::Immediate),
        "quarters" => Ok(CommitRule::HoldQuarters { quarters: 4 }),
        other => Err(format!("--commit expects immediate or quarters, got {other}")),
    }
}

fn print_grade(grade: &Grade) {
    println!(
        "tile_fill {} pass {} plane_rel {:.6} leak_other {:.6} ceiling_picks {} far_picks {} faults_caught {}",
        grade.ran,
        grade.pass,
        grade.plane_rel,
        grade.leak_other,
        grade.ceiling_picks,
        grade.far_picks,
        grade.faults_caught
    );
}

fn write_metrics(dir: &Path, grade: &Grade) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|err| format!("create {}: {err}", dir.display()))?;
    let body = format!(
        "{{\n  \"tile_fill\": {},\n  \"pass\": {},\n  \"plane_rel\": {:.6},\n  \"leak_other\": {:.6},\n  \"ceiling_picks\": {},\n  \"far_picks\": {},\n  \"faults_caught\": {}\n}}\n",
        grade.ran, grade.pass, grade.plane_rel, grade.leak_other, grade.ceiling_picks, grade.far_picks, grade.faults_caught
    );
    let path = dir.join("metrics.json");
    fs::write(&path, body).map_err(|err| format!("write {}: {err}", path.display()))
}

fn default_out() -> PathBuf {
    env_path("GENOS_FILL_REF_OUT").unwrap_or_else(|| PathBuf::from(DEFAULT_OUT))
}

fn tile_fill_from_env() -> bool {
    matches!(env_text("GENOS_FILL_TILE").as_deref(), Some("1" | "on" | "true"))
}

#[expect(
    clippy::disallowed_methods,
    reason = "the env mirror is read once, and only when the matching argument was omitted"
)]
fn env_text(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

#[expect(
    clippy::disallowed_methods,
    reason = "GENOS_FILL_REF_OUT mirrors --out and is read once when the argument was omitted"
)]
fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

fn print_help() {
    println!(
        "\
fill-ref — CPU reference for screen-tile fill

Usage:
  fill-ref run [--tile-fill on|off] [--commit immediate|quarters] [--out DIR]
  fill-ref --help

--tile-fill   Grade the estimators. Default off. The live cache does not read this.
              GENOS_FILL_TILE=on is the mirror and applies only when the argument is omitted.
--commit      Resolve rule. Default is the measured rule (immediate until the age
              runs say otherwise). quarters holds the shown mean for 4 deposits.
--out         Directory for metrics.json. Default ./genos-debug/fill-ref.
              GENOS_FILL_REF_OUT is the mirror and applies only when --out is omitted.
"
    );
}