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
const FLICK_SCRIPT: &str = include_str!("../../scripts/gi_check/flick.rhai");
const REPRO_SCRIPT: &str = include_str!("../../scripts/gi_check/repro.rhai");
const TURN_SCRIPT: &str = include_str!("../../scripts/gi_check/turn.rhai");
const MIX_SCRIPT: &str = include_str!("../../scripts/gi_check/mix.rhai");
const ROOM_SCRIPT: &str = include_str!("../../scripts/gi_check/room.rhai");
const ROOM_SCENE: &str = include_str!("../../scenes/room.rhai");
const RUNS: [&str; 12] = [
    "repro", "settled", "moving", "walk", "flicker", "camera", "trail", "flick", "turn", "room",
    "mix", "spin",
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
/// Flick run: light going back and forth per frame (flick_rows), linear
/// luminance. Set from the box-crossing fix (debugging.md).
const FLICK_LIMIT: f64 = 0.0003;
/// Turn run (turn.rhai, turn_rows): blob of the frame that arrives at the pose
/// (turning at 3 degrees a frame, or walking) against the pose settled from a
/// fresh start, and of the last of 30 frames held there. Set from 10ce430 (turns
/// fail, walking passes) and the leading-edge fill (debugging.md, Validation).
const TURN_ARRIVE_LIMIT: f64 = 0.008;
const TURN_HELD_LIMIT: f64 = 0.002;
/// Turn run, soak / move / return: the first 5 moving frames against each pose
/// settled (onset pop, worst frame) and the frame held after turning out and back
/// against the soaked baseline (return).
const TURN_ONSET_LIMIT: f64 = 0.009;
const TURN_RETURN_LIMIT: f64 = 0.002;
/// Room run (room.rhai, room_rows): Anthony's make-run spot, turning on the spot
/// in the default room, scored relative to the light where it is (a dim wall lit
/// only by bounce counts as much as a lit one), on numbers alone:
/// - turn swim: the world swim of the moving frames (metrics::world_swim: each
///   frame turned onto the next, low-frequency change), mean over the turn;
/// - grid redraw: the same between the pose settled and settled again half a
///   probe further round (only the probe grid slid);
/// - blotch: the settled picture against the path-traced reference at two poses
///   (metrics::blotch: band of the error where blotches live).
/// edffa81 0.0045 / 0.0037 / 0.0250-0.0270 (FAIL), the a-trous filter and 64-ray
/// cache 0.0033 / 0.0019 / 0.0176-0.0206 (the blotch moves from process to process:
/// which pixel claims a cache patch first pins where it is lit); lines between.
const ROOM_SWIM_LIMIT: f64 = 0.0039;
const ROOM_REDRAW_LIMIT: f64 = 0.0027;
const ROOM_BLOTCH_LIMIT: f64 = 0.023;
/// Mix run (mix.rhai): lamps and boxes moving, camera still and turning 3 deg a
/// frame; relative error against each moving frame's scene state settled.
/// edffa81: still 0.0043, turning 0.0070 (FAIL); fixed 0.0021 / 0.0040.
const MIX_LIMIT: f64 = 0.0055;
/// Spin run (mix.rhai, spin): error and flicker on the moving green box and the
/// light around it (where its settled picture differs from the baseline).
/// edffa81 0.0338 / 0.0302 (FAIL); fixed 0.0196 / 0.0119.
const SPIN_ERR_LIMIT: f64 = 0.027;
const SPIN_FLICKER_LIMIT: f64 = 0.02;

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
            // No moving views: each needs a fresh reference of its last frame
            // (minutes, and too noisy when capped). The turn run (soak, move,
            // return; turning and walking), the trail and the flick run are quick's
            // moving checks; the 90-pose camera clip (45 s) was cut 2026-10-10: the
            // turn run covers camera motion in a third of the time and caught what
            // the clip missed (the turn swim).
            &["repro", "settled", "trail", "flick", "turn", "room", "mix", "spin"]
        }
        "mid" => {
            o.stride = 2;
            &["repro", "settled", "moving", "camera", "trail", "flick", "turn", "room", "mix", "spin"]
        }
        "full" => &[
            "repro", "settled", "moving", "camera", "trail", "flick", "turn", "room", "mix", "spin", "flicker",
        ],
        other => return Err(format!("gi-check: --tier quick, mid or full, not {other}")),
    };
    o.runs = runs.iter().map(|r| r.to_string()).collect();
    o.fail_fast = true;
    Ok(())
}

const HELP: &str = "genos-stress gi-check [--tier quick|mid|full] [--out DIR] [--only repro,settled,moving,walk,flicker,camera,trail,flick,turn,room,mix,spin] \
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
        "flick" => (fill(FLICK_SCRIPT), &o.size),
        "repro" => (fill(REPRO_SCRIPT), &o.size),
        "turn" => (fill(TURN_SCRIPT), &o.size),
        "room" => (fill(ROOM_SCRIPT), &o.size),
        "mix" => (fill(&MIX_SCRIPT.replace("__SEGS__", "mix")), &o.size),
        "spin" => (fill(&MIX_SCRIPT.replace("__SEGS__", "spin")), &o.size),
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
    if run == "room" {
        let scene = dir.join("room-scene.rhai");
        std::fs::write(&scene, ROOM_SCENE).map_err(|e| e.to_string())?;
        cmd.arg("--scene").arg(scene);
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
            "flick" => flick_rows(rows, &dir)?,
            "turn" => turn_rows(rows, &dir)?,
            "room" => room_rows(rows, &dir)?,
            "mix" => mix_rows(rows, &dir)?,
            "spin" => spin_rows(rows, &dir)?,
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

/// Flick run (flick.rhai): the light that goes back and forth in the upper third
/// of the doorway pose while lamps and boxes move. Per 4 x 4 block at 320 x 180:
/// (sum of |frame-to-frame change| - |last - first|) / frames, linear luminance,
/// averaged over the blocks. A lamp's light drifts one way and scores near zero.
fn flick_rows(rows: &mut Vec<Row>, dir: &Path) -> Result<(), String> {
    let mut frames = Vec::new();
    while frame(dir, "move", frames.len()).exists() {
        frames.push(metrics::lum320(&frame(dir, "move", frames.len()))?);
    }
    if frames.len() < 3 {
        return Err(format!("gi-check: no flick frames in {}", dir.display()));
    }
    let (w, h) = (frames[0].w, frames[0].h);
    let (bw, bh) = (w / 4, h / 3 / 4);
    let block = |p: &metrics::Plane, bx: usize, by: usize| -> f64 {
        let mut s = 0.0;
        for y in by * 4..by * 4 + 4 {
            for x in bx * 4..bx * 4 + 4 {
                s += p.v[y * w + x];
            }
        }
        s / 16.0
    };
    let (mut back, mut total) = (0.0, 0.0);
    for by in 0..bh {
        for bx in 0..bw {
            let v: Vec<f64> = frames.iter().map(|p| block(p, bx, by)).collect();
            let steps: f64 = v.windows(2).map(|p| (p[1] - p[0]).abs()).sum();
            let net = (v[v.len() - 1] - v[0]).abs();
            back += steps - net;
            total += steps;
        }
    }
    let n = (bw * bh * (frames.len() - 1)) as f64;
    rows.push(Row {
        check: "flick back-and-forth".into(),
        value: back / n,
        limit: FLICK_LIMIT,
        note: format!("all change {:.5} per frame", total / n),
        image: frame(dir, "move", frames.len() - 1),
    });
    Ok(())
}

/// Turn run (turn.rhai): per segment, the arrival frame (the last moving frame,
/// standing exactly at the pose) and the last held frame against ref.png, the
/// pose settled from a fresh start (metrics::blob). Rows: the worst turn arrival,
/// walking's arrival (the control, same limit) and the worst held frame.
fn turn_rows(rows: &mut Vec<Row>, dir: &Path) -> Result<(), String> {
    let reference = metrics::lum640(&dir.join("ref.png"))?;
    let mut turn = (0.0f64, PathBuf::new(), String::new());
    let mut held = (0.0f64, PathBuf::new(), String::new());
    let mut walk = None;
    let mut notes = Vec::new();
    for seg in ["turn", "walk"] {
        let last = |sub: &str| {
            let mut n = 0;
            while frame(dir, &format!("{seg}/{sub}"), n).exists() {
                n += 1;
            }
            (n > 0).then(|| frame(dir, &format!("{seg}/{sub}"), n - 1))
        };
        let (Some(arrive), Some(hold)) = (last("move"), last("hold")) else {
            return Err(format!("gi-check: no {seg} frames in {}", dir.display()));
        };
        let a = metrics::blob(&metrics::lum640(&arrive)?, &reference);
        let h = metrics::blob(&metrics::lum640(&hold)?, &reference);
        notes.push(format!("{seg} {a:.4}/{h:.4}"));
        if seg == "walk" {
            walk = Some((a, arrive));
        } else if a >= turn.0 {
            turn = (a, arrive, seg.to_string());
        }
        if h >= held.0 {
            held = (h, hold, seg.to_string());
        }
    }
    println!("turn     arrival/held blob by segment: {}", notes.join(", "));
    // Onset: the first moving frame saved already stands one step in (back/move
    // frame i - 1 is at back pose i; checked against the settled poses), so the
    // soaked picture before it is ref.png.
    let mut onset = (0.0f64, 0usize, 1.0f64);
    let mut onset_notes = Vec::new();
    for i in 1..6 {
        let settled = frame(dir, "onset", i);
        let moving = frame(dir, "back/move", i - 1);
        if !settled.exists() || !moving.exists() {
            return Err(format!("gi-check: no onset frame {i} in {}", dir.display()));
        }
        let (m, s) = (metrics::lum640(&moving)?, metrics::lum640(&settled)?);
        let b = metrics::blob(&m, &s);
        let ratio = m.v.iter().sum::<f64>() / s.v.iter().sum::<f64>().max(1.0e-6);
        onset_notes.push(format!("{i}:{b:.4}/x{ratio:.3}"));
        if b >= onset.0 {
            onset = (b, i, ratio);
        }
    }
    println!("turn     onset blob/ratio by frame: {}", onset_notes.join(" "));
    let mut n = 0;
    while frame(dir, "back/hold", n).exists() {
        n += 1;
    }
    if n == 0 {
        return Err(format!("gi-check: no back/hold frames in {}", dir.display()));
    }
    let back_hold = frame(dir, "back/hold", n - 1);
    let ret = metrics::blob(&metrics::lum640(&back_hold)?, &reference);
    // Every segment ends at the pose, held until settled: against ref.png, the pose
    // settled from a fresh start in the same scene state (stuck or leftover light).
    let mut end = (0.0f64, PathBuf::new(), String::new());
    for seg in ["turn", "walk", "back"] {
        let p = dir.join(format!("{seg}/end.png"));
        let b = metrics::blob(&metrics::lum640(&p)?, &reference);
        if b >= end.0 {
            end = (b, p, seg.to_string());
        }
    }
    rows.push(Row {
        check: "turn end, soaked, against fresh".into(),
        value: end.0,
        limit: TURN_RETURN_LIMIT,
        note: end.2,
        image: end.1,
    });
    rows.push(Row {
        check: "turn onset pop (frames 1-5, worst)".into(),
        value: onset.0,
        limit: TURN_ONSET_LIMIT,
        note: format!("frame {} ratio {:.3}", onset.1, onset.2),
        image: frame(dir, "back/move", onset.1 - 1),
    });
    rows.push(Row {
        check: "turn return to soaked baseline".into(),
        value: ret,
        limit: TURN_RETURN_LIMIT,
        note: "out 45 deg and back, held 30".into(),
        image: back_hold,
    });
    let (walk, walk_image) = walk.unwrap_or_default();
    rows.push(Row {
        check: "turn arrival blob (3 deg/frame)".into(),
        value: turn.0,
        limit: TURN_ARRIVE_LIMIT,
        note: turn.2,
        image: turn.1,
    });
    rows.push(Row {
        check: "turn arrival blob (walking)".into(),
        value: walk,
        limit: TURN_ARRIVE_LIMIT,
        note: "control".into(),
        image: walk_image,
    });
    rows.push(Row {
        check: "turn held 30 frames blob".into(),
        value: held.0,
        limit: TURN_HELD_LIMIT,
        note: held.2,
        image: held.1,
    });
    Ok(())
}

/// One soak / move / return segment (mix.rhai, room.rhai): each moving frame with
/// a settled picture of its scene state and pose. Returns the relative error
/// (mean, worst frame), the swim between consecutive frames (mean) and the soaked
/// end frame against the baseline (metrics::blob).
fn segment_scores(dir: &Path) -> Result<(f64, (f64, usize), f64, f64), String> {
    let mut errs = Vec::new();
    let mut swims = Vec::new();
    let mut prev: Option<(usize, metrics::Plane)> = None;
    let mut k = 0;
    let mut missing = 0;
    while missing < 8 {
        let (still, moving) = (frame(dir, "still", k), frame(dir, "move", k));
        if !still.exists() || !moving.exists() {
            missing += 1;
            k += 1;
            continue;
        }
        missing = 0;
        let s = metrics::lum640(&still)?;
        let e = metrics::rel_error(&metrics::lum640(&moving)?, &s);
        let base = metrics::rel_base(&s);
        errs.push((metrics::rel_mean(&e, &base), k));
        if let Some((pk, pe)) = &prev {
            if *pk + 1 == k {
                swims.push(metrics::rel_swim(pe, &e, &base));
            }
        }
        prev = Some((k, e));
        k += 1;
    }
    if errs.is_empty() {
        return Err(format!("gi-check: no still/move frames in {}", dir.display()));
    }
    let mean = errs.iter().map(|e| e.0).sum::<f64>() / errs.len() as f64;
    let worst = errs.iter().copied().fold((0.0, 0), |a, e| if e.0 > a.0 { e } else { a });
    let swim = if swims.is_empty() {
        0.0
    } else {
        swims.iter().sum::<f64>() / swims.len() as f64
    };
    let end = metrics::blob(
        &metrics::lum640(&dir.join("end.png"))?,
        &metrics::lum640(&dir.join("baseline.png"))?,
    );
    Ok((mean, worst, swim, end))
}

/// The yaw of each frame of room.rhai's turn (path[0] the start pose; moving and
/// settled frame k are at path[k + 1]).
fn room_yaws() -> Vec<f64> {
    let d2r = 0.0174533f64;
    let y0 = -0.679f64;
    let marks = [y0 / d2r, -16.0, -57.0, y0 / d2r];
    let mut path = vec![y0];
    for m in 1..marks.len() {
        let (from, to) = (marks[m - 1], marks[m]);
        let n = ((to - from).abs() / 3.0).ceil() as usize;
        for i in 1..=n {
            path.push((from + (to - from) * i as f64 / n as f64) * d2r);
        }
    }
    path
}

const ROOM_PITCH: f64 = -0.12;

/// World swim over a run's turn frames in `sub` (move or still): mean over
/// consecutive pairs.
fn room_swim(dir: &Path, sub: &str) -> Result<f64, String> {
    let yaws = room_yaws();
    let mut v = Vec::new();
    for k in 0..yaws.len().saturating_sub(2) {
        let (a, b) = (frame(dir, sub, k), frame(dir, sub, k + 1));
        if !a.exists() || !b.exists() {
            continue;
        }
        let (a, b) = (metrics::lum640(&a)?, metrics::lum640(&b)?);
        v.push(metrics::world_swim(&a, &b, yaws[k + 1], yaws[k + 2], ROOM_PITCH, 8.0));
    }
    if v.is_empty() {
        return Err(format!("gi-check: no {sub} frames in {}", dir.display()));
    }
    Ok(v.iter().sum::<f64>() / v.len() as f64)
}

/// Room run (room.rhai): Anthony's make-run spot, turning on the spot in the
/// default room. Graded on numbers alone: the world swim of what is seen while
/// turning (dim bounce-lit walls count as much as lit ones), the blotch of the
/// settled picture against a path-traced reference, and the soaked end.
fn room_rows(rows: &mut Vec<Row>, dir: &Path) -> Result<(), String> {
    let swim = room_swim(dir, "move")?;
    let settled_swim = room_swim(dir, "still").unwrap_or(f64::NAN);
    let lum = |p: &str| metrics::lum320(&dir.join(p));
    let floor = metrics::world_swim(
        &lum("room-b-reference.png")?,
        &lum("room-c-reference.png")?,
        -36f64.to_radians(),
        -39f64.to_radians(),
        ROOM_PITCH,
        4.0,
    );
    let blotch_a = metrics::blotch(&lum("room-a-live.png")?, &lum("room-a-reference.png")?, 4.0);
    let blotch_b = metrics::blotch(&lum("room-b-live.png")?, &lum("room-b-reference.png")?, 4.0);
    let blotch = 0.5 * (blotch_a + blotch_b);
    let end = metrics::blob(
        &metrics::lum640(&dir.join("end.png"))?,
        &metrics::lum640(&dir.join("baseline.png"))?,
    );
    let redraw = metrics::world_swim(
        &metrics::lum640(&dir.join("grid-0.png"))?,
        &metrics::lum640(&dir.join("grid-1.png"))?,
        -36f64.to_radians(),
        -36.23f64.to_radians(),
        ROOM_PITCH,
        8.0,
    );
    println!(
        "room     world swim moving {swim:.4} settled-per-pose {settled_swim:.4} reference {floor:.4} | grid redraw {redraw:.4} | blotch start {blotch_a:.4} mid {blotch_b:.4} | end {end:.4}"
    );
    rows.push(Row {
        check: "room grid redraw (half a probe)".into(),
        value: redraw,
        limit: ROOM_REDRAW_LIMIT,
        note: "settled, 0.23 deg apart".into(),
        image: dir.join("grid-1.png"),
    });
    rows.push(Row {
        check: "room turn swim (world, relative)".into(),
        value: swim,
        limit: ROOM_SWIM_LIMIT,
        note: format!("settled per pose {settled_swim:.4}, reference {floor:.4}"),
        image: frame(dir, "move", room_yaws().len() / 2),
    });
    rows.push(Row {
        check: "room blotch against reference".into(),
        value: blotch,
        limit: ROOM_BLOTCH_LIMIT,
        note: format!("start {blotch_a:.4} mid {blotch_b:.4}"),
        image: dir.join("room-b-heatmap.png"),
    });
    rows.push(Row {
        check: "room end, soaked, against baseline".into(),
        value: end,
        limit: TURN_RETURN_LIMIT,
        note: String::new(),
        image: dir.join("end.png"),
    });
    Ok(())
}

/// Mix run (mix.rhai): lamps and boxes moving, camera still, then turning.
fn mix_rows(rows: &mut Vec<Row>, dir: &Path) -> Result<(), String> {
    let mut end = (0.0f64, PathBuf::new(), String::new());
    for (seg, check) in [
        ("still", "mix lamps+boxes, camera still (relative)"),
        ("turn", "mix lamps+boxes while turning (relative)"),
    ] {
        let d = dir.join(seg);
        let (mean, worst, _swim, e) = segment_scores(&d)?;
        println!("mix      {seg} relative error mean {mean:.4} worst {:.4}@{} end {e:.4}", worst.0, worst.1);
        rows.push(Row {
            check: check.into(),
            value: mean,
            limit: MIX_LIMIT,
            note: format!("worst frame {} {:.4}", worst.1, worst.0),
            image: frame(&d, "move", worst.1),
        });
        if e >= end.0 {
            end = (e, d.join("end.png"), seg.to_string());
        }
    }
    rows.push(Row {
        check: "mix end, soaked, against baseline".into(),
        value: end.0,
        limit: TURN_RETURN_LIMIT,
        note: end.2,
        image: end.1,
    });
    Ok(())
}

/// Spin run (mix.rhai, spin): the green box moving and spinning before a still
/// camera. Where its settled picture differs from the baseline (the box, its
/// shadow and bounce): |moving - settled| over the settled light (error), and how
/// much that error changes from frame to frame (flicker).
fn spin_rows(rows: &mut Vec<Row>, dir: &Path) -> Result<(), String> {
    let d = dir.join("green");
    let base = metrics::lum640(&d.join("baseline.png"))?;
    let mean_b = base.v.iter().sum::<f64>() / base.v.len().max(1) as f64;
    let mut errs = Vec::new();
    let mut flicks = Vec::new();
    let mut prev: Option<(Vec<f64>, Vec<bool>)> = None;
    let mut k = 0;
    while frame(&d, "still", k).exists() && frame(&d, "move", k).exists() {
        let s = metrics::lum640(&frame(&d, "still", k))?;
        let m = metrics::lum640(&frame(&d, "move", k))?;
        let mask: Vec<bool> = s.v.iter().zip(&base.v).map(|(a, b)| (a - b).abs() > 0.1 * mean_b).collect();
        let e: Vec<f64> = m.v.iter().zip(&s.v).map(|(a, b)| a - b).collect();
        let (mut num, mut den) = (0.0, 0.0);
        for i in 0..e.len() {
            if mask[i] {
                num += e[i].abs();
                den += s.v[i];
            }
        }
        if den > 0.0 {
            errs.push(num / den);
        }
        if let Some((pe, pm)) = &prev {
            let (mut num, mut den) = (0.0, 0.0);
            for i in 0..e.len() {
                if mask[i] || pm[i] {
                    num += (e[i] - pe[i]).abs();
                    den += s.v[i];
                }
            }
            if den > 0.0 {
                flicks.push(num / den);
            }
        }
        prev = Some((e, mask));
        k += 1;
    }
    if errs.is_empty() {
        return Err(format!("gi-check: no spin frames in {}", d.display()));
    }
    let err = errs.iter().sum::<f64>() / errs.len() as f64;
    let flick = flicks.iter().sum::<f64>() / flicks.len().max(1) as f64;
    let end = metrics::blob(&metrics::lum640(&d.join("end.png"))?, &base);
    println!("spin     green box error {err:.4} flicker {flick:.4} end {end:.4}");
    rows.push(Row {
        check: "spin green box error".into(),
        value: err,
        limit: SPIN_ERR_LIMIT,
        note: format!("{} frames", errs.len()),
        image: frame(&d, "move", errs.len() / 2),
    });
    rows.push(Row {
        check: "spin green box flicker".into(),
        value: flick,
        limit: SPIN_FLICKER_LIMIT,
        note: "frame to frame".into(),
        image: frame(&d, "move", errs.len() / 2),
    });
    rows.push(Row {
        check: "spin end, soaked, against baseline".into(),
        value: end,
        limit: TURN_RETURN_LIMIT,
        note: String::new(),
        image: d.join("end.png"),
    });
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
