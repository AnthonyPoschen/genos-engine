//! Headless gates for specular reflections, mirrors, portals, and water.
//!
//! ```text
//! reflections-check --fixture live|good|bad --out DIR
//! reflections-check --case live|good|bad --out DIR
//! ```
//!
//! `./genos-debug/reflections/` is the default directory. `--out` overrides it.
//! `GENOS_REFLECTIONS_OUT` is the mirror and applies only when `--out` was omitted.
//! The process does not write under `$HOME`.

#![expect(
    clippy::disallowed_methods,
    reason = "GENOS_REFLECTIONS_OUT is read once in main and passed down as a path"
)]

use std::process::ExitCode;

use genos_lighting_ref::reflections::{grade, resolve_out, write_report, Fixture};

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(err) => {
            eprintln!("reflections-check: {err}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<u8, String> {
    let env_out = env_mirror();
    let opts = parse(std::env::args().skip(1), env_out.as_deref())?;
    if opts.help {
        print!("{HELP}");
        return Ok(0);
    }
    let report = grade(opts.fixture);
    print!("{}", report.summary());
    write_report(&opts.out, &report)?;
    Ok(report.exit_code())
}

struct Opts {
    help: bool,
    fixture: Fixture,
    out: std::path::PathBuf,
}

fn parse(args: impl Iterator<Item = String>, env_out: Option<&str>) -> Result<Opts, String> {
    let mut help = false;
    let mut fixture = Fixture::Live;
    let mut cli_out: Option<String> = None;
    let mut args = args;
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            help = true;
            continue;
        }
        if arg == "--fixture" || arg == "--case" {
            let value = args
                .next()
                .ok_or_else(|| "--fixture/--case needs live, good, or bad".to_string())?;
            fixture = Fixture::parse(&value)?;
            continue;
        }
        if let Some(value) = arg
            .strip_prefix("--fixture=")
            .or_else(|| arg.strip_prefix("--case="))
        {
            fixture = Fixture::parse(value)?;
            continue;
        }
        if arg == "--out" {
            let value = args
                .next()
                .ok_or_else(|| "--out needs a directory".to_string())?;
            cli_out = Some(nonempty("--out", &value)?);
            continue;
        }
        if let Some(value) = arg.strip_prefix("--out=") {
            cli_out = Some(nonempty("--out", value)?);
            continue;
        }
        return Err(format!("unknown argument {arg}"));
    }
    Ok(Opts {
        help,
        fixture,
        out: resolve_out(cli_out.as_deref(), env_out),
    })
}

fn nonempty(flag: &str, value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(format!("{flag} needs a directory"))
    } else {
        Ok(trimmed.to_string())
    }
}

fn env_mirror() -> Option<String> {
    let raw = std::env::var("GENOS_REFLECTIONS_OUT").ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

const HELP: &str = "\
reflections-check — analytic gates for specular reflections, mirrors, portals, and water.

The default fixture is live. Implemented rows read the presented frame, the
ray dispatch counter, the wave oracle, or the Rust optics parity. ssr_depth_rel, denoise_lag_frames,
caustic_rel, ms_frac, and vram_frac are NOT-IMPLEMENTED. good grades the
engine frame. bad grades a deliberately broken candidate and exits 1.

  --fixture live|good|bad   candidate to grade (default live)
  --case live|good|bad      alias of --fixture
  --out DIR                 report directory (default ./genos-debug/reflections)
  --help                    this text

Writes DIR/metrics.json. One stdout line per metric: name, value, threshold, status.
GENOS_REFLECTIONS_OUT mirrors --out and applies only when --out was omitted.
";
