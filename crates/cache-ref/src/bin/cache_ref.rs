//! `cache-ref`: run the ADR 0039 world-cache comparisons and write a markdown
//! report under `./genos-debug/cache-ref/`.
//!
//! ```text
//! cargo run --release -p genos-cache-ref -- compare [--cell 0.25] [--rays 16]
//!     [--history 32] [--pixel 0.05] [--spp 2048] [--seeds 11,12,13,14]
//!     [--warm 160] [--samples 8] [--threads N] [--skews 8,64] [--out DIR]
//! cargo run --release -p genos-cache-ref -- variants [--configs "plain;side+pos+compact"] [same flags]
//! ```

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use genos_cache_ref::harness::Bench;
use genos_cache_ref::report::{self, Knobs};

struct Args {
    cmd: String,
    bench: Bench,
    knobs: Knobs,
    out: PathBuf,
    specs: String,
    skews: Vec<f64>,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut b = Bench::default();
    let mut k = Knobs::default();
    let mut out = PathBuf::from("genos-debug/cache-ref");
    let mut specs = report::DEFAULT_SPECS.to_owned();
    let mut skews = vec![8.0, 64.0];
    let cmd = args.first().cloned().ok_or("missing command: compare")?;
    let mut it = args.iter().skip(1);
    while let Some(flag) = it.next() {
        let val = it.next().ok_or(format!("{flag} needs a value"))?;
        let f = || val.parse::<f64>().map_err(|e| format!("{flag}: {e}"));
        let u = || val.parse::<usize>().map_err(|e| format!("{flag}: {e}"));
        match flag.as_str() {
            "--cell" => k.cell_size = f()?,
            "--rays" => k.rays_per_cell = u()?,
            "--history" => k.history_frames = f()?,
            "--linger-after" => k.linger_after = u()?,
            "--pixel" => b.pixel_size = f()?,
            "--spp" => b.ref_spp = u()?,
            "--warm" => b.warm_frames = u()?,
            "--samples" => b.samples = u()?,
            "--threads" => b.threads = u()?,
            "--seeds" => {
                b.seeds = val
                    .split(',')
                    .map(|s| s.trim().parse::<u64>().map_err(|e| format!("--seeds: {e}")))
                    .collect::<Result<_, _>>()?;
            }
            "--out" => out = PathBuf::from(val),
            "--configs" => specs.clone_from(val),
            "--skews" => {
                skews = val
                    .split(',')
                    .map(|s| s.trim().parse::<f64>().map_err(|e| format!("--skews: {e}")))
                    .collect::<Result<_, _>>()?;
            }
            _ => return Err(format!("unknown flag {flag}")),
        }
    }
    b.ref_dir = Some(out.join("reference"));
    Ok(Args {
        cmd,
        bench: b,
        knobs: k,
        out,
        specs,
        skews,
    })
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Args {
        cmd,
        bench: b,
        knobs: k,
        out,
        specs,
        skews,
    } = match parse_args(&args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("cache-ref: {e}");
            return ExitCode::from(2);
        }
    };
    let text = match cmd.as_str() {
        "compare" => report::compare(&b, k, &skews).0,
        "variants" => report::variants(&b, k, &specs),
        _ => {
            eprintln!("cache-ref: unknown command {cmd}");
            return ExitCode::from(2);
        }
    };
    let path = out.join(format!("{cmd}.md"));
    if let Err(e) = fs::create_dir_all(&out).and_then(|()| fs::write(&path, &text)) {
        eprintln!("cache-ref: write {}: {e}", path.display());
        return ExitCode::from(1);
    }
    println!("{text}");
    println!("wrote {}", path.display());
    ExitCode::SUCCESS
}
