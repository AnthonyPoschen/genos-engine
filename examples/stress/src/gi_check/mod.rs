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
const CAMERA_SCRIPT: &str = include_str!("../../scripts/gi_check/camera.rhai");
const TRAIL_SCRIPT: &str = include_str!("../../scripts/gi_check/trail.rhai");
const REPRO_SCRIPT: &str = include_str!("../../scripts/gi_check/repro.rhai");
const RUNS: [&str; 7] = [
    "repro", "settled", "moving", "walk", "flicker", "camera", "trail",
];
const FLICKER_SCRIPT: &str = include_str!("../../scripts/temporal_flicker_aligned.rhai");
const VIEWS: [&str; 5] = ["room-a", "corner", "contact", "lamp-wall", "hall"];
/// The close-up view (views.rhai, settled only), scored on its own rows.
const CLOSE_VIEW: &str = "close";
/// Close-up limits, the five views' contact limit: GI v2 at 1129d5d (before the
/// contact term) scored mean 0.030, contact 0.026, open 0.028.
const CLOSE_MEAN_LIMIT: f64 = 0.045;
const CLOSE_CONTACT_LIMIT: f64 = 0.045;
const CLOSE_OPEN_LIMIT: f64 = 0.045;

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
/// Camera run: each moving frame against the settled picture at its pose (mean
/// relative error; mean over frames of the worst 10 x 10 px tile) and frame-to-
/// frame change beyond the settled pictures' own. Tile and flicker limits come
/// from GI v1 in motion (0.29, 0.034) with a margin (debugging.md, Validation).
const CAMERA_ERR_LIMIT: f64 = 0.03;
const CAMERA_TILE_LIMIT: f64 = 0.35;
const CAMERA_FLICKER_LIMIT: f64 = 0.05;
/// After the camera stops: error of the first 10 frames against the settled
/// picture, and frames until it stays under STOP_SETTLED.
const STOP_ERR_LIMIT: f64 = 0.02;
const STOP_SETTLED: f64 = 0.01;
const STOP_FRAMES_LIMIT: f64 = 10.0;
/// Trail run: error in the space a moving box left in the last 10 frames, mean
/// over the frames and the worst frame.
const TRAIL_LIMIT: f64 = 0.05;
const TRAIL_WORST_LIMIT: f64 = 0.10;
/// Repro run: the last frame of each clip that ends on the settled pose, against
/// the settled picture: |mean ratio - 1|.
const REPRO_LIMIT: f64 = 0.03;

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
    /// Camera run: settle at every stride-th pose.
    stride: u32,
    /// Camera run: only the first this many poses of the path (0: all).
    clip: u32,
    /// Stop at the first run with a FAIL (every --tier does).
    fail_fast: bool,
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
        stride: 1,
        clip: 0,
        fail_fast: false,
    };
    let mut keep_going = false;
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
            "--stride" => o.stride = value()?.parse().map_err(|e| format!("{arg}: {e}"))?,
            "--clip" => o.clip = value()?.parse().map_err(|e| format!("{arg}: {e}"))?,
            "--fail-fast" => o.fail_fast = true,
            "--tier" => tier(&mut o, &value()?)?,
            "--keep-going" => keep_going = true,
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
    if keep_going {
        o.fail_fast = false;
    }
    for r in &o.runs {
        if !RUNS.contains(&r.as_str()) {
            return Err(format!("gi-check: unknown run {r}"));
        }
    }
    std::fs::create_dir_all(&o.out).map_err(|e| format!("{}: {e}", o.out.display()))?;
    let out = std::fs::canonicalize(&o.out).map_err(|e| e.to_string())?;
    let started = std::time::Instant::now();
    let mut rows = Vec::new();
    for r in &o.runs {
        if !o.score_only {
            render(&o, &out, r)?;
        }
        score_run(&o, &out, r, &mut rows)?;
        if o.fail_fast && rows.iter().any(|row| !row.pass()) {
            println!("gi-check: stopped after the {r} run (first FAIL)");
            break;
        }
    }
    println!("gi-check: {:.0} s", started.elapsed().as_secs_f64());
    let failed = print_table(&rows);
    let table = rows.iter().map(Row::line).collect::<Vec<_>>().join("\n");
    let _ = std::fs::write(out.join("gi-check.txt"), format!("{table}\n"));
    if failed > 0 {
        return Err(format!("gi-check: {failed} check(s) failed"));
    }
    Ok(())
}

/// Tiers (debugging.md, gi-check): quick to iterate on, mid when quick passes,
/// full before pushing. Every tier stops at the first run with a FAIL.
fn tier(o: &mut Options, name: &str) -> Result<(), String> {
    let runs: &[&str] = match name {
        "quick" => {
            o.size = "640x360".into();
            o.views = "contact,lamp-wall,close".into();
            o.stride = 4;
            o.clip = 90;
            // No moving views: each needs a fresh reference of its last frame
            // (minutes, and too noisy when capped). The camera clip and the trail
            // are quick's moving checks.
            &["repro", "settled", "camera", "trail"]
        }
        "mid" => {
            o.stride = 2;
            &["repro", "settled", "moving", "camera", "trail"]
        }
        "full" => &[
            "repro", "settled", "moving", "camera", "trail", "walk", "flicker",
        ],
        other => return Err(format!("gi-check: --tier quick, mid or full, not {other}")),
    };
    o.runs = runs.iter().map(|r| r.to_string()).collect();
    o.fail_fast = true;
    Ok(())
}

const HELP: &str = "genos-stress gi-check [--tier quick|mid|full] [--out DIR] [--only repro,settled,moving,walk,flicker,camera,trail] \
[--score-only] [--ref-noise 0.05] [--ref-spp 2048] [--ref-seconds 900] [--size 640x360] [--walk-size 1280x720] \
[--settled-refs DIR] [--gi v2|v1] [--views corner,contact] [--stride 1] [--clip N] [--fail-fast] [--keep-going]
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
            .replace("__STRIDE__", &o.stride.max(1).to_string())
            .replace("__CLIP__", &o.clip.to_string())
            .replace("__DIR__", &dir.display().to_string())
    };
    let (script, size) = match run {
        "settled" | "moving" => (fill(VIEWS_SCRIPT), &o.size),
        "walk" => (fill(WALK_SCRIPT), &o.walk_size),
        "camera" => (fill(CAMERA_SCRIPT), &o.size),
        "trail" => (fill(TRAIL_SCRIPT), &o.size),
        "repro" => (fill(REPRO_SCRIPT), &o.size),
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
    if matches!(run, "moving" | "walk" | "camera" | "trail") {
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
    // The close-up: its own rows, so the five-view means stay comparable.
    if dir.join(format!("{CLOSE_VIEW}-live.png")).exists() {
        let noise = ref_noise(&refs.join("report.md"), CLOSE_VIEW);
        let s = metrics::score_view(dir, refs, CLOSE_VIEW, noise)?;
        println!(
            "{run:<8} {CLOSE_VIEW:<10} mean {:.4} bias {:+.4} blob {:.4} (floor {:.4}, excess {:.4}) contact {:.4} open {:.4} edge {:.3}",
            s.mean_rel, s.bias, s.blob, s.blob_floor, s.blob_excess, s.contact, s.open, s.edge_err
        );
        let image = dir.join(format!("{CLOSE_VIEW}-heatmap.png"));
        for (name, value, limit) in [
            ("mean error", s.mean_rel, CLOSE_MEAN_LIMIT),
            ("contact error", s.contact, CLOSE_CONTACT_LIMIT),
            ("open-wall error", s.open, CLOSE_OPEN_LIMIT),
        ] {
            rows.push(Row {
                check: format!("{run} close-up {name}"),
                value,
                limit,
                note: format!("bias {:+.4}", s.bias),
                image: image.clone(),
            });
        }
    }
    Ok(())
}

fn score_run(o: &Options, out: &Path, run: &str, rows: &mut Vec<Row>) -> Result<(), String> {
    {
        let dir = out.join(run);
        match run {
            "repro" => repro_rows(rows, &dir)?,
            "settled" => {
                let refs = o.settled_refs.clone().unwrap_or_else(|| dir.clone());
                view_rows(rows, run, &dir, &refs)?;
            }
            "moving" => {
                view_rows(rows, run, &dir, &dir)?;
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
            "camera" => camera_rows(rows, &dir)?,
            "trail" => trail_rows(rows, &dir)?,
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
    Ok(())
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

fn frame(dir: &Path, sub: &str, i: usize) -> PathBuf {
    dir.join(format!("{sub}/frame-{i:04}.png"))
}

/// `per_probe` (GENOS_GI_DEBUG) of the first `n` probe frames after the scene
/// first stood frozen: the camera run's moving pass.
fn rays_after_frozen(log: &Path, n: usize) -> Option<(f64, f64)> {
    let text = std::fs::read_to_string(log).ok()?;
    let v: Vec<f64> = text
        .lines()
        .skip_while(|l| !l.starts_with("gi2 frozen"))
        .filter_map(|l| l.split("per_probe ").nth(1)?.trim().parse().ok())
        .take(n)
        .collect();
    (!v.is_empty()).then(|| {
        (
            v.iter().sum::<f64>() / v.len() as f64,
            v.iter().copied().fold(f64::MAX, f64::min),
        )
    })
}

fn camera_rows(rows: &mut Vec<Row>, dir: &Path) -> Result<(), String> {
    let mut moving = Vec::new();
    while frame(dir, "move", moving.len()).exists() {
        moving.push(metrics::lum320(&frame(dir, "move", moving.len()))?);
    }
    let still: Vec<(usize, metrics::Plane)> = (0..moving.len())
        .filter(|&i| frame(dir, "still", i).exists())
        .map(|i| metrics::lum320(&frame(dir, "still", i)).map(|p| (i, p)))
        .collect::<Result<_, _>>()?;
    if moving.is_empty() || still.is_empty() {
        return Err(format!("gi-check: no camera frames in {}", dir.display()));
    }
    // The moving frames may start a frame early or late against the path: take the
    // offset that fits best.
    let at = |i: usize, o: isize| -> Option<&metrics::Plane> {
        moving.get((i as isize + o).max(0) as usize)
    };
    let offset = (-2isize..=2)
        .min_by(|&a, &b| {
            let e = |o| {
                still
                    .iter()
                    .filter_map(|(i, s)| at(*i, o).map(|m| metrics::rel_err(m, s).0))
                    .sum::<f64>()
            };
            e(a).total_cmp(&e(b))
        })
        .unwrap_or(0);
    let (mut err, mut n, mut tiles, mut worst, mut worst_at) = (0.0, 0, 0.0, 0.0f64, 0);
    let (mut worst_err, mut worst_err_at) = (0.0f64, 0);
    for (i, s) in &still {
        if let Some(m) = at(*i, offset) {
            let (e, t) = metrics::rel_err(m, s);
            err += e;
            tiles += t;
            n += 1;
            if t > worst {
                (worst, worst_at) = (t, *i);
            }
            if e > worst_err {
                (worst_err, worst_err_at) = (e, *i);
            }
        }
    }
    let err = err / n.max(1) as f64;
    let tiles = tiles / n.max(1) as f64;
    let mut flick = (0.0, 0);
    for w in still.windows(2) {
        let ((i0, s0), (i1, s1)) = (&w[0], &w[1]);
        if i1 - i0 == 1 {
            if let (Some(m0), Some(m1)) = (at(*i0, offset), at(*i1, offset)) {
                flick.0 += metrics::excess_change(m0, m1, s0, s1);
                flick.1 += 1;
            }
        }
    }
    if let Some((mean, least)) = rays_after_frozen(&dir.join("run.log"), moving.len()) {
        println!("camera   rays held per probe while moving: mean {mean:.1}, least {least:.1}");
    }
    println!(
        "camera   {} moving frames, {} settled poses, offset {offset}; worst frame {worst_err_at} error {worst_err:.4}",
        moving.len(),
        still.len()
    );
    let image = |i: usize| frame(dir, "move", (i as isize + offset).max(0) as usize);
    rows.push(Row {
        check: "camera mid-move error".into(),
        value: err,
        limit: CAMERA_ERR_LIMIT,
        note: format!("worst frame {worst_err_at} {worst_err:.4}"),
        image: image(worst_err_at),
    });
    rows.push(Row {
        check: "camera mid-move worst tile".into(),
        value: tiles,
        limit: CAMERA_TILE_LIMIT,
        note: format!("worst frame {worst_at} {worst:.4}"),
        image: image(worst_at),
    });
    if flick.1 > 0 {
        rows.push(Row {
            check: "camera mid-move flicker".into(),
            value: flick.0 / flick.1 as f64,
            limit: CAMERA_FLICKER_LIMIT,
            note: format!("{} frame pairs", flick.1),
            image: image(worst_err_at),
        });
    }
    // After stopping: the frames held on the last pose against its settled picture.
    let last = moving.len() - 1;
    if let Some((_, settled)) = still.iter().find(|(i, _)| *i == last) {
        let mut errs = Vec::new();
        while frame(dir, "stop", errs.len()).exists() {
            let f = metrics::lum320(&frame(dir, "stop", errs.len()))?;
            errs.push(metrics::rel_err(&f, settled).0);
        }
        if !errs.is_empty() {
            let first = &errs[..10.min(errs.len())];
            let mean = first.iter().sum::<f64>() / first.len() as f64;
            let settle = errs
                .iter()
                .rposition(|&e| e > STOP_SETTLED)
                .map_or(0, |i| i + 1);
            println!(
                "camera   after stopping, error by frame: {}",
                errs.iter()
                    .take(20)
                    .map(|e| format!("{e:.3}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            rows.push(Row {
                check: "camera post-stop error (frames 0-9)".into(),
                value: mean,
                limit: STOP_ERR_LIMIT,
                note: format!("frame 0 {:.4}", errs[0]),
                image: frame(dir, "stop", 0),
            });
            rows.push(Row {
                check: "camera post-stop frames to settle".into(),
                value: settle as f64,
                limit: STOP_FRAMES_LIMIT,
                note: format!("until under {STOP_SETTLED} for good, of {}", errs.len()),
                image: frame(dir, "stop", settle.saturating_sub(1)),
            });
        }
    }
    Ok(())
}

/// Repro run: each clip's last frame (on the settled pose) against the settled
/// picture, as |mean ratio - 1| of the luminance.
fn repro_rows(rows: &mut Vec<Row>, dir: &Path) -> Result<(), String> {
    let still = metrics::lum320(&dir.join("still.png"))?;
    let mean = |p: &metrics::Plane| p.v.iter().sum::<f64>() / p.v.len().max(1) as f64;
    let base = mean(&still).max(1.0e-6);
    for clip in ["jitter", "slide", "turn"] {
        let mut last = None;
        for i in 0.. {
            let f = frame(dir, clip, i);
            if !f.exists() {
                break;
            }
            last = Some((i, f));
        }
        let (i, f) = last.ok_or(format!("gi-check: no {clip} frames in {}", dir.display()))?;
        let ratio = mean(&metrics::lum320(&f)?) / base;
        let err = metrics::rel_err(&metrics::lum320(&f)?, &still).0;
        rows.push(Row {
            check: format!("repro {clip} brightness"),
            value: (ratio - 1.0).abs(),
            limit: REPRO_LIMIT,
            note: format!("ratio {ratio:.4}, error {err:.4}, frame {i}"),
            image: f,
        });
    }
    Ok(())
}

fn trail_rows(rows: &mut Vec<Row>, dir: &Path) -> Result<(), String> {
    let mut moving = Vec::new();
    while frame(dir, "move", moving.len()).exists() {
        moving.push(metrics::lum320(&frame(dir, "move", moving.len()))?);
    }
    let refs: Vec<(usize, metrics::Plane, metrics::Plane)> = (0..moving.len())
        .filter(|&i| frame(dir, "still", i).exists() && frame(dir, "depth", i).exists())
        .map(|i| {
            Ok((
                i,
                metrics::lum320(&frame(dir, "still", i))?,
                metrics::depth320(&frame(dir, "depth", i))?,
            ))
        })
        .collect::<Result<_, String>>()?;
    if moving.is_empty() || refs.len() < 2 {
        return Err(format!("gi-check: no trail frames in {}", dir.display()));
    }
    // Per settled pose: the space the box left over the last 10 frames.
    let masks: Vec<(usize, Vec<bool>)> = refs
        .iter()
        .map(|(i, _, d)| {
            let before: Vec<&metrics::Plane> = refs
                .iter()
                .filter(|(j, _, _)| *j < *i && *j + 10 >= *i)
                .map(|(_, _, d)| d)
                .collect();
            (*i, metrics::vacated(d, &before, 6))
        })
        .collect();
    let at = |i: usize, o: isize| moving.get((i as isize + o).max(0) as usize);
    let err_at = |o: isize| -> Vec<(usize, f64, usize)> {
        refs.iter()
            .zip(&masks)
            .filter_map(|((i, s, _), (_, m))| {
                let px = m.iter().filter(|&&b| b).count();
                (px >= 30).then(|| at(*i, o).map(|f| (*i, metrics::masked_err(f, s, m), px)))?
            })
            .collect()
    };
    // The moving frames may start a frame early or late against the path.
    let whole = |o: isize| {
        refs.iter()
            .filter_map(|(i, s, _)| at(*i, o).map(|f| metrics::rel_err(f, s).0))
            .sum::<f64>()
    };
    let offset = (-2isize..=2)
        .min_by(|&a, &b| whole(a).total_cmp(&whole(b)))
        .unwrap_or(0);
    let errs = err_at(offset);
    if errs.is_empty() {
        return Err(format!(
            "gi-check: the trail box left too little space in {}",
            dir.display()
        ));
    }
    let mean = errs.iter().map(|e| e.1).sum::<f64>() / errs.len() as f64;
    let worst = errs
        .iter()
        .copied()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap_or((0, 0.0, 0));
    let image = |i: usize| frame(dir, "move", (i as isize + offset).max(0) as usize);
    println!(
        "trail    offset {offset}; error in the space left, by frame: {}",
        errs.iter()
            .map(|(i, e, _)| format!("{i}:{e:.3}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    if let Some((mean, least)) = rays_after_frozen(&dir.join("run.log"), moving.len()) {
        println!("trail    rays held per probe while it moves (whole picture): mean {mean:.1}, least {least:.1}");
    }
    rows.push(Row {
        check: "trail error (space left, moving)".into(),
        value: mean,
        limit: TRAIL_LIMIT,
        note: format!("worst frame {} {:.4} ({} px)", worst.0, worst.1, worst.2),
        image: image(worst.0),
    });
    rows.push(Row {
        check: "trail worst frame".into(),
        value: worst.1,
        limit: TRAIL_WORST_LIMIT,
        note: format!("frame {}", worst.0),
        image: image(worst.0),
    });
    Ok(())
}
