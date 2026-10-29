//! Headless `ReSTIR` DI reference.
//!
//! `run` writes `metrics.json` and `reference.json`. `diff` scores a candidate
//! dump against that reference. The default directory is `./genos-debug/restir`.
//! `--out` overrides it. `GENOS_RESTIR_OUT` is the mirror and applies only when
//! `--out` was omitted. The tool replaces those files. It does not write under
//! `$HOME`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use genos_restir_ref::metrics::GateLimits;
use genos_restir_ref::reservoir::Weighting;
use genos_restir_ref::restir::{Config, History, Proposal};
use genos_restir_ref::TreeFault;
use genos_restir_ref::shade::Scene;
use genos_restir_ref::{diff_dumps, evaluate, parse_dump, write_run, EvalSpec, Report};

const DEFAULT_OUT: &str = "genos-debug/restir";

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(err) => {
            eprintln!("restir-ref: {err}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<u8, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return Ok(0);
    }
    match args[0].as_str() {
        "run" => run_cmd(&args[1..]),
        "diff" => diff_cmd(&args[1..]),
        other => Err(format!("unknown command {other}")),
    }
}

fn run_cmd(args: &[String]) -> Result<u8, String> {
    let opts = parse_run(args)?;
    let spec = EvalSpec {
        scene: opts.scene,
        config: opts.config,
        seeds: (1..=opts.seeds).collect(),
        frames: opts.frames,
        limits: opts.limits,
    };
    let (report, dump) = evaluate(&spec)?;
    print_report(&report);
    write_run(&opts.out, &report, &dump)?;
    println!("wrote {}", opts.out.display());
    if report.pass {
        Ok(0)
    } else {
        Ok(1)
    }
}

fn diff_cmd(args: &[String]) -> Result<u8, String> {
    let opts = parse_diff(args)?;
    let reference = read_dump(&opts.reference)?;
    let candidate = read_dump(&opts.candidate)?;
    let diff = diff_dumps(&reference, &candidate, &opts.limits);
    println!(
        "ref_err_near {:.6} grid_worst {} frames_to_ref {} frame_ms {} mismatches {} max_w_rel {:.6}",
        diff.ref_err_near,
        fmt_opt(diff.grid_worst),
        fmt_u(diff.frames_to_ref),
        fmt_opt(diff.frame_ms),
        diff.reservoir_mismatches,
        diff.max_w_rel
    );
    for reason in &diff.reasons {
        println!("fail {reason}");
    }
    let body = diff_json(&diff);
    std::fs::create_dir_all(&opts.out)
        .map_err(|err| format!("create {}: {err}", opts.out.display()))?;
    let path = opts.out.join("diff.json");
    std::fs::write(&path, body).map_err(|err| format!("write {}: {err}", path.display()))?;
    if diff.pass() {
        Ok(0)
    } else {
        Ok(1)
    }
}

fn print_report(report: &Report) {
    println!(
        "ref_err_near {:.6} grid_worst {:.6} frames_to_ref {} frame_ms {:.3} variance_ratio {:.4} pass {}",
        report.ref_err_near,
        report.grid_worst,
        report.frames_to_ref,
        report.frame_ms,
        report.variance_ratio,
        report.pass
    );
}

fn read_dump(path: &Path) -> Result<genos_restir_ref::Dump, String> {
    let text =
        std::fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    parse_dump(&text)
}

struct RunOpts {
    scene: Scene,
    config: Config,
    seeds: u64,
    frames: u32,
    limits: GateLimits,
    out: PathBuf,
}

struct DiffOpts {
    reference: PathBuf,
    candidate: PathBuf,
    limits: GateLimits,
    out: PathBuf,
}

fn parse_run(args: &[String]) -> Result<RunOpts, String> {
    let mut scene_name = "gate".to_string();
    let mut config = Config::gate();
    let mut seeds = 64u64;
    let mut frames = 8u32;
    let mut limits = GateLimits::reference();
    let mut out = None;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "--scene" => scene_name = next_value(args, &mut index, "--scene")?,
            "--frames" => {
                frames = parse_u32(&next_value(args, &mut index, "--frames")?, "--frames")?;
            }
            "--seeds" => seeds = parse_u64(&next_value(args, &mut index, "--seeds")?, "--seeds")?,
            "--candidates" => {
                config.candidates = parse_u32(
                    &next_value(args, &mut index, "--candidates")?,
                    "--candidates",
                )?;
            }
            "--spatial" => {
                config.spatial_neighbors =
                    parse_u32(&next_value(args, &mut index, "--spatial")?, "--spatial")?;
            }
            "--radius" => {
                config.spatial_radius =
                    parse_u32(&next_value(args, &mut index, "--radius")?, "--radius")?;
            }
            "--proposal" => {
                config.proposal = proposal(&next_value(args, &mut index, "--proposal")?)?;
            }
            "--tree-fault" => {
                config.tree_fault =
                    TreeFault::parse(&next_value(args, &mut index, "--tree-fault")?)?;
            }
            "--weighting" => {
                config.weighting = weighting(&next_value(args, &mut index, "--weighting")?)?;
            }
            "--history" => config.history = history(&next_value(args, &mut index, "--history")?)?,
            "--temporal" => config.temporal = true,
            "--no-temporal" => config.temporal = false,
            "--m-clamp" => {
                config.m_clamp =
                    parse_u32(&next_value(args, &mut index, "--m-clamp")?, "--m-clamp")?;
            }
            "--frame-ms" => {
                limits.frame_ms =
                    parse_f32(&next_value(args, &mut index, "--frame-ms")?, "--frame-ms")?;
            }
            "--ref-err" => {
                limits.ref_err_near =
                    parse_f32(&next_value(args, &mut index, "--ref-err")?, "--ref-err")?;
            }
            "--grid-worst" => {
                limits.grid_worst = parse_f32(
                    &next_value(args, &mut index, "--grid-worst")?,
                    "--grid-worst",
                )?;
            }
            "--frames-to-ref" => {
                limits.frames_to_ref = parse_u32(
                    &next_value(args, &mut index, "--frames-to-ref")?,
                    "--frames-to-ref",
                )?;
            }
            "--out" => out = Some(PathBuf::from(next_value(args, &mut index, "--out")?)),
            other => return Err(format!("unknown argument {other}")),
        }
        index += 1;
    }
    if seeds == 0 {
        return Err("--seeds must be positive".to_string());
    }
    if config.tree_fault != TreeFault::None && config.proposal != Proposal::Tree {
        return Err("--tree-fault applies to --proposal tree".to_string());
    }
    Ok(RunOpts {
        scene: scene_by_name(&scene_name)?,
        config,
        seeds,
        frames,
        limits,
        out: out.unwrap_or_else(default_out),
    })
}

fn parse_diff(args: &[String]) -> Result<DiffOpts, String> {
    let mut reference = None;
    let mut candidate = None;
    let mut limits = GateLimits::reference();
    let mut out = None;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "--reference" => {
                reference = Some(PathBuf::from(next_value(args, &mut index, "--reference")?));
            }
            "--candidate" => {
                candidate = Some(PathBuf::from(next_value(args, &mut index, "--candidate")?));
            }
            "--frame-ms" => {
                limits.frame_ms =
                    parse_f32(&next_value(args, &mut index, "--frame-ms")?, "--frame-ms")?;
            }
            "--ref-err" => {
                limits.ref_err_near =
                    parse_f32(&next_value(args, &mut index, "--ref-err")?, "--ref-err")?;
            }
            "--grid-worst" => {
                limits.grid_worst = parse_f32(
                    &next_value(args, &mut index, "--grid-worst")?,
                    "--grid-worst",
                )?;
            }
            "--frames-to-ref" => {
                limits.frames_to_ref = parse_u32(
                    &next_value(args, &mut index, "--frames-to-ref")?,
                    "--frames-to-ref",
                )?;
            }
            "--out" => out = Some(PathBuf::from(next_value(args, &mut index, "--out")?)),
            other => return Err(format!("unknown argument {other}")),
        }
        index += 1;
    }
    Ok(DiffOpts {
        reference: reference.ok_or("--reference is required")?,
        candidate: candidate.ok_or("--candidate is required")?,
        limits,
        out: out.unwrap_or_else(default_out),
    })
}

fn scene_by_name(name: &str) -> Result<Scene, String> {
    match name {
        "gate" => Ok(Scene::gate()),
        "shadow" => Ok(Scene::shadow()),
        "open" => Ok(Scene::open_grid()),
        "row" => Ok(Scene::open_row()),
        "domain" => Ok(Scene::domain_pair()),
        "variance" => Ok(Scene::variance_point()),
        "many" => Ok(Scene::many_lights()),
        other => Err(format!("unknown scene {other}")),
    }
}

fn proposal(name: &str) -> Result<Proposal, String> {
    match name {
        "uniform" => Ok(Proposal::Uniform),
        "target" => Ok(Proposal::Target),
        "tree" => Ok(Proposal::Tree),
        other => Err(format!("unknown proposal {other}")),
    }
}

fn weighting(name: &str) -> Result<Weighting, String> {
    match name {
        "balance" => Ok(Weighting::Balance),
        "section3" => Ok(Weighting::Section3),
        other => Err(format!("unknown weighting {other}")),
    }
}

fn history(name: &str) -> Result<History, String> {
    match name {
        "temporal" => Ok(History::Temporal),
        "final" => Ok(History::Final),
        other => Err(format!("unknown history {other}")),
    }
}

fn next_value(args: &[String], index: &mut usize, flag: &str) -> Result<String, String> {
    let next = index.saturating_add(1);
    let value = args
        .get(next)
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value"))?;
    *index = next;
    Ok(value)
}

fn parse_u32(text: &str, flag: &str) -> Result<u32, String> {
    text.parse::<u32>()
        .map_err(|_| format!("{flag} wants an integer"))
}

fn parse_u64(text: &str, flag: &str) -> Result<u64, String> {
    text.parse::<u64>()
        .map_err(|_| format!("{flag} wants an integer"))
}

fn parse_f32(text: &str, flag: &str) -> Result<f32, String> {
    let value = text
        .parse::<f32>()
        .map_err(|_| format!("{flag} wants a number"))?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(format!("{flag} is not finite"))
    }
}

fn default_out() -> PathBuf {
    out_from_env().unwrap_or_else(|| PathBuf::from(DEFAULT_OUT))
}

#[expect(
    clippy::disallowed_methods,
    reason = "GENOS_RESTIR_OUT mirrors --out and is read once when the argument was omitted"
)]
fn out_from_env() -> Option<PathBuf> {
    std::env::var_os("GENOS_RESTIR_OUT").map(PathBuf::from)
}

fn fmt_opt(value: Option<f32>) -> String {
    match value {
        Some(number) => format!("{number:.6}"),
        None => "none".to_string(),
    }
}

fn fmt_u(value: Option<u32>) -> String {
    match value {
        Some(number) => number.to_string(),
        None => "none".to_string(),
    }
}

fn diff_json(diff: &genos_restir_ref::Diff) -> String {
    let reasons = genos_json::array(
        diff.reasons
            .iter()
            .map(|reason| genos_json::string(reason.as_str()))
            .collect(),
    );
    let value = genos_json::object([
        ("kind", genos_json::string("genos-restir-diff")),
        ("version", genos_json::int(1)),
        (
            "ref_err_near",
            genos_json::float(f64::from(diff.ref_err_near)),
        ),
        ("grid_worst", opt_f32(diff.grid_worst)),
        ("frames_to_ref", opt_u32(diff.frames_to_ref)),
        ("frame_ms", opt_f32(diff.frame_ms)),
        (
            "reservoir_mismatches",
            genos_json::int(i64::from(diff.reservoir_mismatches)),
        ),
        ("max_w_rel", genos_json::float(f64::from(diff.max_w_rel))),
        ("pass", genos_json::bool(diff.pass())),
        ("reasons", reasons),
    ]);
    genos_json::encode(&value)
}

fn opt_f32(value: Option<f32>) -> genos_json::Value {
    match value {
        Some(number) => genos_json::float(f64::from(number)),
        None => genos_json::Value::Null,
    }
}

fn opt_u32(value: Option<u32>) -> genos_json::Value {
    match value {
        Some(number) => genos_json::int(i64::from(number)),
        None => genos_json::Value::Null,
    }
}

fn print_help() {
    println!(
        "\
restir-ref — CPU reference for ReSTIR direct lighting

Usage:
  restir-ref run [options]
  restir-ref diff --reference FILE --candidate FILE [options]
  restir-ref --help

run writes metrics.json and reference.json. diff writes diff.json.
The default directory is ./genos-debug/restir. The two run files are replaced.
Nothing is written under $HOME.

Options for run:
  --scene NAME          gate (default), shadow, open, row, domain, variance, many
  --frames N            frames per seed (default 8)
  --seeds N             seeds 1..=N (default 64)
  --candidates N        initial RIS candidates (default 8)
  --spatial N           spatial neighbours (default 0; the open-grid test uses 3)
  --radius N            Chebyshev radius (default 2)
  --proposal NAME       target (default), uniform, or tree
  --tree-fault NAME     none (default) or ignore-distance (tree proposal only)
  --weighting NAME      balance (default) or section3
  --history NAME        temporal (default) or final
  --temporal            enable temporal reuse (default)
  --no-temporal         fresh candidates each frame
  --m-clamp N           clamp history M to N times this frame's candidate count
                        (default 20; 0 leaves M unclamped)
  --ref-err X           ref_err_near ceiling (default 0.05)
  --grid-worst X        grid_worst ceiling (default 0.15)
  --frames-to-ref N     frames_to_ref ceiling (default 8)
  --frame-ms X          CPU step ceiling in milliseconds (default 50)
  --out DIR             output directory

Options for diff:
  --reference FILE      reference.json from run
  --candidate FILE      dump in the same schema
  --ref-err X           ref_err_near ceiling (default 0.05)
  --grid-worst X        grid_worst ceiling (default 0.15)
  --frames-to-ref N     frames_to_ref ceiling (default 8)
  --frame-ms X          frame_ms ceiling when the candidate records one
  --out DIR             directory for diff.json

GENOS_RESTIR_OUT mirrors --out and applies only when --out was omitted.
Exit 0 passes the gates, 1 fails them, 2 is a usage or IO error.
"
    );
}
