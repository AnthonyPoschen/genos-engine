//! Grade lamp-layer metrics. No window and no Vulkan.
//!
//! `--case good` and `--case broken` are planted pictures. `--case baseline`
//! reports `NOT-IMPLEMENTED` for a live lamp toggle, which this binary does
//! not capture.

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use genos_lighting_ref::layering::{evaluate, summary_line, write_report, Case, DEFAULT_OUT};

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(err) => {
            eprintln!("layering-gate: {err}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<u8, String> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return Ok(0);
    }
    let opts = parse(&args)?;
    let report = evaluate(opts.case)?;
    for metric in &report.metrics {
        println!("{}", summary_line(metric));
    }
    write_report(&opts.out, &report)?;
    u8::try_from(report.exit_code()).map_err(|_| "exit code out of range".to_string())
}

struct Opts {
    case: Case,
    out: PathBuf,
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut case = None;
    let mut out = PathBuf::from(DEFAULT_OUT);
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "--case" => {
                let name = next_value(args, &mut index, "--case")?;
                case = Some(Case::parse(name).ok_or_else(|| format!("unknown case {name}"))?);
            }
            "--out" => {
                out = PathBuf::from(next_value(args, &mut index, "--out")?);
            }
            other => return Err(format!("unknown argument {other}")),
        }
        index += 1;
    }
    let case = case.ok_or("missing --case (good, broken, or baseline)")?;
    Ok(Opts { case, out })
}

fn next_value<'a>(args: &'a [String], index: &mut usize, flag: &str) -> Result<&'a str, String> {
    *index += 1;
    args.get(*index)
        .map(String::as_str)
        .ok_or_else(|| format!("missing value for {flag}"))
}

fn print_help() {
    println!(
        "\
layering-gate — score lamp-layer settle metrics

  --case good|broken|baseline   planted pass, planted fail, or today's engine
  --out DIR                     directory for {{case}}.json
                                default: {DEFAULT_OUT}

Prints one line per metric: name, value, threshold, PASS/FAIL/NOT-IMPLEMENTED.
Exit 0 when no row is FAIL. Exit 1 when a row is FAIL. NOT-IMPLEMENTED does
not fail the process. Exit 2 is a bad argument or a write error.
A live lamp toggle is not captured here. Baseline rows stay NOT-IMPLEMENTED."
    );
}
