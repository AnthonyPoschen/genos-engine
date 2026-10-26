//! Grade lamp-layer metrics. No window and no Vulkan.
//!
//! `--case good` and `--case broken` are planted pictures. `--case instant`
//! is a step request whose picture fades for 50 ms. `--case noisy` shimmers
//! on the hold. `--capture DIR` scores engine frames against the requested
//! lamp weight. `--case baseline` with no capture prints `NOT-IMPLEMENTED`.

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use genos_lighting_ref::layering::{
    evaluate, evaluate_capture, summary_line, write_report, Case, DEFAULT_OUT, UNSETTLED_CAPTURE,
};

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(err) if err == UNSETTLED_CAPTURE => {
            println!("{UNSETTLED_CAPTURE}");
            ExitCode::from(2)
        }
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
    let report = match opts.capture {
        Some(dir) => evaluate_capture(&dir)?,
        None => evaluate(opts.case.ok_or("missing --case or --capture")?)?,
    };
    for metric in &report.metrics {
        println!("{}", summary_line(metric));
    }
    write_report(&opts.out, &report)?;
    u8::try_from(report.exit_code()).map_err(|_| "exit code out of range".to_string())
}

struct Opts {
    case: Option<Case>,
    capture: Option<PathBuf>,
    out: PathBuf,
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut case = None;
    let mut capture = None;
    let mut out = PathBuf::from(DEFAULT_OUT);
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "--case" => {
                let name = next_value(args, &mut index, "--case")?;
                case = Some(Case::parse(name).ok_or_else(|| format!("unknown case {name}"))?);
            }
            "--capture" => {
                capture = Some(PathBuf::from(next_value(args, &mut index, "--capture")?));
            }
            "--out" => {
                out = PathBuf::from(next_value(args, &mut index, "--out")?);
            }
            other => return Err(format!("unknown argument {other}")),
        }
        index += 1;
    }
    if capture.is_some() {
        if let Some(case) = case {
            if case != Case::Baseline {
                return Err("--capture grades the baseline; drop --case".to_string());
            }
        }
    } else if case.is_none() {
        return Err("missing --case or --capture".to_string());
    }
    Ok(Opts { case, capture, out })
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

  --case good|broken|instant|noisy|baseline
                                planted pass, planted fail, 50 ms fade after
                                a step request, noisy hold, or no frames
  --capture DIR                 grade frame-NNNN.png against weights.txt
                                and expected-on / expected-off
                                (DIR or DIR/frames; reference PNGs may sit
                                in the parent as expected-*-reference.png)
  --out DIR                     directory for {{case}}.json
                                default: {DEFAULT_OUT}

Prints one line per metric: name, value, threshold, PASS/FAIL/NOT-IMPLEMENTED.
Exit 0 when no row is FAIL. Exit 1 when a row is FAIL. NOT-IMPLEMENTED does
not fail the process. Exit 2 is a bad argument, a write error, or
UNSETTLED-CAPTURE (the pre-toggle frames never sat within 0.05 of the on
image for four frames). UNSETTLED-CAPTURE is printed alone and is not a FAIL.
An endpoint trace whose logged noise is above 0.025 is rejected the same way.
--capture writes baseline.json. Layer rows stay NOT-IMPLEMENTED.
Settle, fade, and dim stay NOT-IMPLEMENTED unless weights.txt and both
expected images are present. The expected image is the path-traced or
analytic picture at that frame's requested weight."
    );
}
