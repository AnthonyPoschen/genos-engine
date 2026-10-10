//! `genos-stress gi-check`: renders the GI checks with this binary, scores them
//! against CPU path-traced references and prints PASS or FAIL against limits
//! (docs/systems/debugging.md, "gi-check").
//!
//! Each run starts this executable again as a proof-mode script run (one window at a
//! time), into `<out>/<run>`. `--score-only` rescores an earlier `<out>`.

mod metrics;

use std::path::{Path, PathBuf};
use std::process::Command;

use metrics::ViewScore;

const VIEWS_SCRIPT: &str = include_str!("../../scripts/gi_check/views.rhai");
const WALK_SCRIPT: &str = include_str!("../../scripts/gi_check/walk.rhai");
const FLICKER_SCRIPT: &str = include_str!("../../scripts/temporal_flicker_aligned.rhai");
const VIEWS: [&str; 5] = ["room-a", "corner", "contact", "lamp-wall", "hall"];

/// Limits, set 2026-10-10 from the GI v2 baseline measured on an RTX 4070 and
/// the de-splotch targets (moving blob at most 0.02, mean at most 0.04). Errors are
/// shares of the light; each is the mean over the five views.
const MEAN_LIMIT: f64 = 0.04;
const BLOB_LIMIT: f64 = 0.02;
/// Blob of the worst single view: v2 settled's worst (lamp-wall) scored 0.027.
const BLOB_VIEW_LIMIT: f64 = 0.03;
/// Within 2 px of a contact line: v2 settled scored 0.038.
const CONTACT_LIMIT: f64 = 0.045;
/// More than 20 px from contact lines: v2 settled scored 0.019.
const OPEN_LIMIT: f64 = 0.03;
/// Worst 10 x 10 px tile, cold against warm walk (v1 0.002, v2 0.17 at noon).
const POP_LIMIT: f64 = 0.05;
/// temporal_flicker_aligned: frozen pictures must not change (one display code).
const FROZEN_LIMIT: f64 = 0.004;
/// Moving, worst pose mean_delta in the still region (v1 baseline at most 0.0004).
const MOVING_FLICKER_LIMIT: f64 = 0.0005;

struct Options {
    out: PathBuf,
    runs: Vec<String>,
    score_only: bool,
    noise: f64,
    seconds: f64,
    max_spp: u32,
    size: String,
    walk_size: String,
    settled_refs: Option<PathBuf>,
    /// GI v2 (default) or v1, for checking the tool on the old path.
    v2: bool,
    /// Views to render (all five when empty).
    views: String,
}

pub fn run(args: impl Iterator<Item = String>) -> Result<(), String> {
    let mut o = Options {
        out: PathBuf::from("target/gi-check"),
        runs: ["settled", "moving", "walk", "flicker"]
            .map(String::from)
            .to_vec(),
        score_only: false,
        noise: 0.05,
        seconds: 900.0,
        max_spp: 2048,
        size: "640x360".into(),
        walk_size: "1280x720".into(),
        settled_refs: None,
        v2: true,
        views: String::new(),
    };
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--out" => o.out = value()?.into(),
            "--only" => o.runs = value()?.split(',').map(String::from).collect(),
            "--score-only" => o.score_only = true,
            "--ref-noise" => o.noise = value()?.parse().map_err(|e| format!("{arg}: {e}"))?,
            "--ref-seconds" => o.seconds = value()?.parse().map_err(|e| format!("{arg}: {e}"))?,
            "--ref-spp" => o.max_spp = value()?.parse().map_err(|e| format!("{arg}: {e}"))?,
            "--size" => o.size = value()?,
            "--walk-size" => o.walk_size = value()?,
            "--settled-refs" => o.settled_refs = Some(value()?.into()),
            "--views" => o.views = value()?,
            "--gi" => {
                o.v2 = match value()?.as_str() {
                    "v1" => false,
                    "v2" => true,
                    other => return Err(format!("gi-check: --gi v1 or v2, not {other}")),
                }
            }
            "-h" | "--help" => {
                println!("{}", HELP);
                return Ok(());
            }
            other => return Err(format!("gi-check: unknown option {other}\n{HELP}")),
        }
    }
    for r in &o.runs {
        if !["settled", "moving", "walk", "flicker"].contains(&r.as_str()) {
            return Err(format!("gi-check: unknown run {r}"));
        }
    }
    std::fs::create_dir_all(&o.out).map_err(|e| format!("{}: {e}", o.out.display()))?;
    let out = std::fs::canonicalize(&o.out).map_err(|e| e.to_string())?;
    if !o.score_only {
        for r in &o.runs {
            render(&o, &out, r)?;
        }
    }
    let rows = score(&o, &out)?;
    let failed = print_table(&rows);
    let table = rows.iter().map(Row::line).collect::<Vec<_>>().join("\n");
    let _ = std::fs::write(out.join("gi-check.txt"), format!("{table}\n"));
    if failed > 0 {
        return Err(format!("gi-check: {failed} check(s) failed"));
    }
    Ok(())
}

const HELP: &str = "genos-stress gi-check [--out DIR] [--only settled,moving,walk,flicker] \
[--score-only] [--ref-noise 0.05] [--ref-spp 2048] [--ref-seconds 900] [--size 640x360] [--walk-size 1280x720] \
[--settled-refs DIR] [--gi v2|v1] [--views corner,contact]
Renders with GENOS_GI=v2 (or v1); GENOS_GI2_TILE defaults to the 1440p probe spacing (height / 90), references are \
cached in $GENOS_REFERENCE_CACHE (default target/reference-cache). Exit status 1 on a FAIL.";

fn render(o: &Options, out: &Path, run: &str) -> Result<(), String> {
    let dir = out.join(run);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let fill = |s: &str| {
        s.replace("__MOVING__", if run == "moving" { "true" } else { "false" })
            .replace("__NOISE__", &format!("{:?}", o.noise))
            .replace("__MAXSPP__", &o.max_spp.to_string())
            .replace("__SECS__", &format!("{:?}", o.seconds))
            .replace("__ONLY__", &o.views)
            .replace("__GI__", if o.v2 { "1.0" } else { "0.0" })
            .replace("__DIR__", &dir.display().to_string())
    };
    let (script, size) = match run {
        "settled" | "moving" => (fill(VIEWS_SCRIPT), &o.size),
        "walk" => (fill(WALK_SCRIPT), &o.walk_size),
        // The repo's flicker gate, with a heatmap per pose and window.
        _ => (
            fill(
                &FLICKER_SCRIPT
                    .replace(
                        "threshold: 0.004 })",
                        "threshold: 0.004, heatmap: `__DIR__/${name}-frozen-heatmap.png` })",
                    )
                    .replace(
                        "region: [0.0, 0.0, 1.0, 0.33] })",
                        "region: [0.0, 0.0, 1.0, 0.33], heatmap: `__DIR__/${name}-moving-heatmap.png` })",
                    ),
            ),
            &o.walk_size,
        ),
    };
    let path = dir.join("script.rhai");
    std::fs::write(&path, script).map_err(|e| e.to_string())?;
    let log = std::fs::File::create(dir.join("run.log")).map_err(|e| e.to_string())?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(exe);
    cmd.args(["--proof", "--no-panel", "--size", size, "--script"])
        .arg(&path)
        .arg("--report")
        .arg(&dir)
        .env("GENOS_GI", if o.v2 { "v2" } else { "v1" })
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log);
    // Probes as far apart on screen as the 16 px tile at 1440p, at every size.
    if std::env::var_os("GENOS_GI2_TILE").is_none() {
        let rows: u32 = size
            .split('x')
            .nth(1)
            .and_then(|h| h.parse().ok())
            .unwrap_or(720);
        cmd.env("GENOS_GI2_TILE", (rows / 90).clamp(4, 64).to_string());
    }
    if run == "moving" || run == "walk" {
        cmd.env("GENOS_GI_DEBUG", "1");
    }
    eprintln!("gi-check: {run} -> {}", dir.display());
    let status = cmd.status().map_err(|e| e.to_string())?;
    // The flicker script ends on its own gate; a failed gate is scored, not an error.
    if !status.success() && run != "flicker" {
        return Err(format!(
            "gi-check: the {run} run failed ({status}); see {}/run.log",
            dir.display()
        ));
    }
    Ok(())
}

struct Row {
    check: String,
    value: f64,
    limit: f64,
    note: String,
    /// The picture to look at first when the check fails.
    image: PathBuf,
}

impl Row {
    fn pass(&self) -> bool {
        self.value <= self.limit
    }
    fn line(&self) -> String {
        let mut line = format!(
            "{:<34} {:>9.4} {:>9.4}  {:<4}  {}",
            self.check,
            self.value,
            self.limit,
            if self.pass() { "PASS" } else { "FAIL" },
            self.note
        );
        if !self.pass() {
            line += &format!("\n{:<34} see {}", "", self.image.display());
        }
        line
    }
}

/// `RESULT <view> ... refnoise X` lines of a report.
fn ref_noise(report: &Path, view: &str) -> f64 {
    let text = std::fs::read_to_string(report).unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let rest = l.split("RESULT ").nth(1)?;
            let mut words = rest.split_whitespace();
            (words.next()? == view).then_some(())?;
            let words: Vec<&str> = words.collect();
            let i = words.iter().position(|w| *w == "refnoise")?;
            words.get(i + 1)?.parse().ok()
        })
        .last()
        .unwrap_or(0.03)
}

/// Mean of `per_probe` over the GENOS_GI_DEBUG lines of a run log.
fn rays_per_probe(log: &Path) -> Option<(f64, f64)> {
    let text = std::fs::read_to_string(log).ok()?;
    let v: Vec<f64> = text
        .lines()
        .filter_map(|l| l.split("per_probe ").nth(1)?.trim().parse().ok())
        .collect();
    (!v.is_empty()).then(|| {
        (
            v.iter().sum::<f64>() / v.len() as f64,
            v.iter().copied().fold(f64::MAX, f64::min),
        )
    })
}

fn view_rows(rows: &mut Vec<Row>, run: &str, dir: &Path, refs: &Path) -> Result<(), String> {
    let mut scores: Vec<ViewScore> = Vec::new();
    for v in VIEWS {
        if !dir.join(format!("{v}-live.png")).exists() {
            continue;
        }
        let noise = ref_noise(&refs.join("report.md"), v);
        let s = metrics::score_view(dir, refs, v, noise)?;
        println!(
            "{run:<8} {v:<10} mean {:.4} bias {:+.4} blob {:.4} (floor {:.4}, excess {:.4}) contact {:.4} open {:.4} edge {:.3}",
            s.mean_rel, s.bias, s.blob, s.blob_floor, s.blob_excess, s.contact, s.open, s.edge_err
        );
        scores.push(s);
    }
    if scores.is_empty() {
        return Err(format!("gi-check: no views in {}", dir.display()));
    }
    let avg = |f: &dyn Fn(&ViewScore) -> f64| {
        let v: Vec<f64> = scores.iter().map(f).filter(|x| x.is_finite()).collect();
        v.iter().sum::<f64>() / v.len().max(1) as f64
    };
    let worst = |f: &dyn Fn(&ViewScore) -> f64| {
        scores
            .iter()
            .filter(|s| f(s).is_finite())
            .max_by(|a, b| f(a).total_cmp(&f(b)))
            .map(|s| format!("worst {} {:.4}", s.view, f(s)))
            .unwrap_or_default()
    };
    let worst_image = |f: &dyn Fn(&ViewScore) -> f64| {
        let view = scores
            .iter()
            .filter(|s| f(s).is_finite())
            .max_by(|a, b| f(a).total_cmp(&f(b)))
            .map_or("", |s| s.view.as_str());
        dir.join(format!("{view}-heatmap.png"))
    };
    let n = scores.len();
    let mut push = |name: &str, f: &dyn Fn(&ViewScore) -> f64, limit: f64| {
        rows.push(Row {
            check: format!("{run} {name} ({n} views)"),
            value: avg(f),
            limit,
            note: worst(f),
            image: worst_image(f),
        })
    };
    push("mean error", &|s| s.mean_rel, MEAN_LIMIT);
    push("blob", &|s| s.blob_excess, BLOB_LIMIT);
    push("contact error", &|s| s.contact, CONTACT_LIMIT);
    push("open-wall error", &|s| s.open, OPEN_LIMIT);
    let blob = |s: &ViewScore| s.blob_excess;
    rows.push(Row {
        check: format!("{run} blob, worst view"),
        value: scores.iter().map(blob).fold(0.0, f64::max),
        limit: BLOB_VIEW_LIMIT,
        note: worst(&blob),
        image: worst_image(&blob),
    });
    Ok(())
}

fn score(o: &Options, out: &Path) -> Result<Vec<Row>, String> {
    let mut rows = Vec::new();
    for run in &o.runs {
        let dir = out.join(run);
        match run.as_str() {
            "settled" => {
                let refs = o.settled_refs.clone().unwrap_or_else(|| dir.clone());
                view_rows(&mut rows, run, &dir, &refs)?;
            }
            "moving" => {
                view_rows(&mut rows, run, &dir, &dir)?;
                if let Some((mean, min)) = rays_per_probe(&dir.join("run.log")) {
                    println!(
                        "moving   rays held per probe while moving: mean {mean:.1}, least {min:.1}"
                    );
                }
            }
            "walk" => {
                let (mut worst, mut worst_at, mut over, mut n, mut gap) = (0.0f64, 0, 0, 0, 0.0f64);
                for i in 0.. {
                    let cold = dir.join(format!("cold/frame-{i:04}.png"));
                    let warm = dir.join(format!("warm/frame-{i:04}.png"));
                    if !cold.exists() || !warm.exists() {
                        break;
                    }
                    let (g, t) = metrics::walk_gap(&cold, &warm)?;
                    gap = gap.max(g);
                    if t > worst {
                        (worst, worst_at) = (t, i);
                    }
                    over += usize::from(t > POP_LIMIT);
                    n += 1;
                }
                if n == 0 {
                    return Err(format!("gi-check: no walk frames in {}", dir.display()));
                }
                if let Some((mean, min)) = rays_per_probe(&dir.join("run.log")) {
                    println!("walk     rays held per probe: mean {mean:.1}, least {min:.1}");
                }
                rows.push(Row {
                    check: "corner-walk pop (worst tile)".into(),
                    value: worst,
                    limit: POP_LIMIT,
                    note: format!(
                        "frame {worst_at}; {over} of {n} frames over; worst frame gap {gap:.4}"
                    ),
                    image: dir.join(format!("cold/frame-{worst_at:04}.png")),
                });
            }
            _ => {
                let text = std::fs::read_to_string(dir.join("report.md"))
                    .map_err(|e| format!("{}: {e}", dir.display()))?;
                let num = |l: &str, key: &str| -> Option<f64> {
                    l.split(&format!("{key} "))
                        .nth(1)?
                        .split_whitespace()
                        .next()?
                        .trim_end_matches(',')
                        .parse()
                        .ok()
                };
                let (mut frozen, mut moving) = ((0.0f64, String::new()), (0.0f64, String::new()));
                for l in text.lines() {
                    let pose = l
                        .trim_start_matches(['-', ' '])
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .to_string();
                    if l.contains(" frozen: ") {
                        if let Some(v) = num(l, "max_delta").filter(|v| *v >= frozen.0) {
                            frozen = (v, pose);
                        }
                    } else if l.contains(" moving: ") {
                        if let Some(v) = num(l, "mean_delta").filter(|v| *v >= moving.0) {
                            moving = (v, pose);
                        }
                    }
                }
                rows.push(Row {
                    check: "flicker frozen max_delta".into(),
                    value: frozen.0,
                    limit: FROZEN_LIMIT,
                    note: format!("worst {}", frozen.1),
                    image: dir.join(format!("{}-frozen-heatmap.png", frozen.1)),
                });
                rows.push(Row {
                    check: "flicker moving mean_delta".into(),
                    value: moving.0,
                    limit: MOVING_FLICKER_LIMIT,
                    note: format!("worst {}", moving.1),
                    image: dir.join(format!("{}-moving-heatmap.png", moving.1)),
                });
            }
        }
    }
    Ok(rows)
}

fn print_table(rows: &[Row]) -> usize {
    println!();
    println!(
        "{:<34} {:>9} {:>9}  {}",
        "check", "value", "limit", "result"
    );
    for r in rows {
        println!("{}", r.line());
    }
    rows.iter().filter(|r| !r.pass()).count()
}
