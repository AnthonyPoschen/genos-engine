//! Scores the CCD and destruction metrics from the design.
//!
//! `step` does not implement those features. A live script marks each metric
//! pending, and the line says `NOT-IMPLEMENTED`. Known-good and known-bad
//! scripts feed readings so the comparator can be shown to pass and to fail.
//!
//! ```text
//! --out DIR
//! --fixture proof|good|bad|live
//! ```
//!
//! `--out` defaults to `./genos-debug/physics`. `GENOS_PHYSICS_METRICS_OUT`
//! and `GENOS_PHYSICS_METRICS_FIXTURE` mirror those arguments and apply only
//! when the argument was omitted.

use std::path::{Path, PathBuf};
use std::process::Command;

const DEFAULT_OUT: &str = "genos-debug/physics";

#[derive(Clone, Copy)]
enum Bound {
    AtMost,
    AtLeast,
}

struct Spec {
    name: &'static str,
    bound: Bound,
    threshold: f64,
    threshold_text: &'static str,
}

const METRICS: &[Spec] = &[
    Spec {
        name: "tunnel_count",
        bound: Bound::AtMost,
        threshold: 0.0,
        threshold_text: "<=0",
    },
    Spec {
        name: "penetration_m",
        bound: Bound::AtLeast,
        threshold: -0.01,
        threshold_text: ">=-0.01",
    },
    Spec {
        name: "energy_drift",
        bound: Bound::AtMost,
        threshold: 0.01,
        threshold_text: "<=0.01",
    },
    Spec {
        name: "determinism_hash_match",
        bound: Bound::AtLeast,
        threshold: 1.0,
        threshold_text: ">=1",
    },
    Spec {
        name: "contacts_stored",
        bound: Bound::AtLeast,
        threshold: 1.0,
        threshold_text: ">=1",
    },
    Spec {
        name: "bonds_broken",
        bound: Bound::AtLeast,
        threshold: 1.0,
        threshold_text: ">=1",
    },
    Spec {
        name: "debris_alive",
        bound: Bound::AtMost,
        threshold: 8.0,
        threshold_text: "<=8",
    },
    Spec {
        name: "floor_penetration_m",
        bound: Bound::AtMost,
        threshold: 0.0,
        threshold_text: "<=0",
    },
    Spec {
        name: "toi_deferred",
        bound: Bound::AtMost,
        threshold: 0.0,
        threshold_text: "<=0",
    },
    Spec {
        name: "step_ms_potato",
        bound: Bound::AtMost,
        threshold: 0.2,
        threshold_text: "<=0.2",
    },
    Spec {
        name: "step_ms_low",
        bound: Bound::AtMost,
        threshold: 0.5,
        threshold_text: "<=0.5",
    },
    Spec {
        name: "step_ms_medium",
        bound: Bound::AtMost,
        threshold: 1.5,
        threshold_text: "<=1.5",
    },
    Spec {
        name: "step_ms_high",
        bound: Bound::AtMost,
        threshold: 3.0,
        threshold_text: "<=3",
    },
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Pass,
    Fail,
    Missing,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::Missing => "NOT-IMPLEMENTED",
        }
    }
}

struct Row {
    name: &'static str,
    value_text: String,
    threshold_text: &'static str,
    json_value: String,
    status: Status,
}

enum Fixture {
    Proof,
    Good,
    Bad,
    Live,
}

impl Fixture {
    fn name(&self) -> &'static str {
        match self {
            Self::Proof => "proof",
            Self::Good => "good",
            Self::Bad => "bad",
            Self::Live => "live",
        }
    }
}

struct Slots {
    value: Vec<Option<f64>>,
    seen: Vec<bool>,
}

impl Slots {
    fn new() -> Self {
        Self {
            value: vec![None; METRICS.len()],
            seen: vec![false; METRICS.len()],
        }
    }
}

fn main() {
    let code = match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("metrics: {err}");
            2
        }
    };
    std::process::exit(code);
}

fn run() -> Result<i32, String> {
    let args = parse_args()?;
    if args.help {
        print_help();
        return Ok(0);
    }
    match args.fixture {
        Fixture::Proof => prove(&args.out),
        Fixture::Good | Fixture::Bad | Fixture::Live => {
            let rows = score(&args.fixture)?;
            write_report(&args.out, args.fixture.name(), &rows)?;
            print_rows(&rows);
            Ok(exit_code(&rows))
        }
    }
}

struct Args {
    out: PathBuf,
    fixture: Fixture,
    help: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut out_arg = None;
    let mut fixture_arg = None;
    let mut help = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => help = true,
            "--quiet" | "--nocapture" | "--exact" => {}
            "--out" => out_arg = Some(need(&mut args, "--out")?),
            "--fixture" => fixture_arg = Some(need(&mut args, "--fixture")?),
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let out = if let Some(dir) = out_arg {
        dir
    } else if let Some(dir) = env_text("GENOS_PHYSICS_METRICS_OUT") {
        dir
    } else {
        default_out()
    };
    if out.is_empty() || out.chars().all(char::is_whitespace) {
        return Err("--out is empty".into());
    }
    let fixture = if let Some(name) = fixture_arg {
        parse_fixture(&name)?
    } else if let Some(name) = env_text("GENOS_PHYSICS_METRICS_FIXTURE") {
        parse_fixture(&name)?
    } else {
        Fixture::Proof
    };
    Ok(Args {
        out: PathBuf::from(out),
        fixture,
        help,
    })
}

fn need(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next().ok_or_else(|| format!("{flag} needs a value"))
}

#[expect(
    clippy::disallowed_methods,
    reason = "these variables only mirror --out and --fixture, and only when that argument was omitted"
)]
fn env_text(key: &str) -> Option<String> {
    let Ok(value) = std::env::var(key) else {
        return None;
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// `cargo test` starts in the package directory. The default still writes to
/// the repository `genos-debug/physics`. An explicit `--out` or env path stays
/// relative to the process directory.
fn default_out() -> String {
    let Ok(here) = std::env::current_dir() else {
        return DEFAULT_OUT.to_string();
    };
    let Ok(text) = std::fs::read_to_string(here.join("Cargo.toml")) else {
        return DEFAULT_OUT.to_string();
    };
    if text
        .lines()
        .any(|line| line.trim() == "name = \"genos-physics\"")
    {
        here.join("../../genos-debug/physics")
            .to_string_lossy()
            .into_owned()
    } else {
        DEFAULT_OUT.to_string()
    }
}

fn parse_fixture(text: &str) -> Result<Fixture, String> {
    match text {
        "proof" => Ok(Fixture::Proof),
        "good" => Ok(Fixture::Good),
        "bad" => Ok(Fixture::Bad),
        "live" => Ok(Fixture::Live),
        other => Err(format!("unknown fixture {other}")),
    }
}

fn print_help() {
    println!(
        "\
--out DIR
    JSON directory. Default ./genos-debug/physics.
    GENOS_PHYSICS_METRICS_OUT applies only when --out is omitted.
--fixture proof|good|bad|live
    proof runs good, bad, and live. Default proof.
    GENOS_PHYSICS_METRICS_FIXTURE applies only when --fixture is omitted.
Exit 0 when no metric is FAIL. Exit 1 when a metric is FAIL.
Exit 2 on a bad argument or a bad script.
NOT-IMPLEMENTED is not a failure."
    );
}

fn score(fixture: &Fixture) -> Result<Vec<Row>, String> {
    if matches!(fixture, Fixture::Live) && genos_physics::GRAVITY.y >= 0.0 {
        return Err("live fixture would have to call step, and GRAVITY points up".into());
    }
    let script = scene_path(fixture.name());
    let text =
        std::fs::read_to_string(&script).map_err(|err| format!("{}: {err}", script.display()))?;
    let slots = load_script(&text).map_err(|err| format!("{}: {err}", script.display()))?;
    for (index, spec) in METRICS.iter().enumerate() {
        if !slots.seen[index] {
            return Err(format!("{} does not set {}", script.display(), spec.name));
        }
    }
    let mut rows = Vec::with_capacity(METRICS.len());
    for (index, spec) in METRICS.iter().enumerate() {
        rows.push(judge(spec, slots.value[index]));
    }
    Ok(rows)
}

/// `reading("name", number);`, `pending("name");`, and full-line `//` comments.
/// The files stay valid Rhai. This host only accepts that subset.
fn load_script(text: &str) -> Result<Slots, String> {
    let mut slots = Slots::new();
    for (index, raw) in text.lines().enumerate() {
        let line_no = index + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let result = if let Some(name) = pending_call(line) {
            note(&mut slots, &name, None)
        } else if let Some((name, value)) = reading_call(line) {
            note(&mut slots, &name, Some(value))
        } else {
            Err("expected reading or pending".to_string())
        };
        if let Err(err) = result {
            return Err(format!("line {line_no}: {err}"));
        }
    }
    Ok(slots)
}

fn pending_call(line: &str) -> Option<String> {
    let inner = line.strip_prefix("pending(")?.strip_suffix(");")?;
    quoted(inner.trim())
}

fn reading_call(line: &str) -> Option<(String, f64)> {
    let inner = line.strip_prefix("reading(")?.strip_suffix(");")?;
    let (name, value) = inner.split_once(',')?;
    let name = quoted(name.trim())?;
    let value = value.trim().parse::<f64>().ok()?;
    Some((name, value))
}

fn quoted(text: &str) -> Option<String> {
    let inner = text.strip_prefix('"')?.strip_suffix('"')?;
    if inner.contains('"') || inner.contains('\\') {
        None
    } else {
        Some(inner.to_string())
    }
}

fn scene_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scenes")
        .join(format!("{name}.rhai"))
}

fn note(slots: &mut Slots, name: &str, value: Option<f64>) -> Result<(), String> {
    let index = metric_index(name).ok_or_else(|| format!("unknown metric {name}"))?;
    if slots.seen[index] {
        return Err(format!("{name} was set twice"));
    }
    slots.seen[index] = true;
    slots.value[index] = value;
    Ok(())
}

fn metric_index(name: &str) -> Option<usize> {
    METRICS.iter().position(|spec| spec.name == name)
}

fn judge(spec: &Spec, value: Option<f64>) -> Row {
    let Some(value) = value else {
        return Row {
            name: spec.name,
            value_text: "na".to_string(),
            threshold_text: spec.threshold_text,
            json_value: "null".to_string(),
            status: Status::Missing,
        };
    };
    let finite = value.is_finite();
    let inside = finite
        && match spec.bound {
            Bound::AtMost => value <= spec.threshold,
            Bound::AtLeast => value >= spec.threshold,
        };
    Row {
        name: spec.name,
        value_text: if finite {
            fmt_num(value)
        } else {
            "nan".to_string()
        },
        threshold_text: spec.threshold_text,
        json_value: if finite {
            fmt_num(value)
        } else {
            "null".to_string()
        },
        status: if inside { Status::Pass } else { Status::Fail },
    }
}

fn fmt_num(value: f64) -> String {
    let mut text = format!("{value:.4}");
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    if text == "-0" {
        "0".to_string()
    } else {
        text
    }
}

fn line(row: &Row) -> String {
    format!(
        "{} {} {} {}",
        row.name,
        row.value_text,
        row.threshold_text,
        row.status.as_str()
    )
}

fn print_rows(rows: &[Row]) {
    for row in rows {
        println!("{}", line(row));
    }
}

fn exit_code(rows: &[Row]) -> i32 {
    i32::from(rows.iter().any(|row| row.status == Status::Fail))
}

fn json(fixture: &str, rows: &[Row]) -> String {
    let failed = rows.iter().filter(|row| row.status == Status::Fail).count();
    let mut out =
        format!("{{\n  \"fixture\": \"{fixture}\",\n  \"failed\": {failed},\n  \"metrics\": [\n");
    for (index, row) in rows.iter().enumerate() {
        let comma = if index + 1 == rows.len() { "" } else { "," };
        out.push_str(&format!(
            "    {{\"name\": \"{}\", \"value\": {}, \"threshold\": \"{}\", \"status\": \"{}\"}}{comma}\n",
            row.name, row.json_value, row.threshold_text, row.status.as_str()
        ));
    }
    out.push_str("  ]\n}\n");
    out
}

fn write_report(dir: &Path, fixture: &str, rows: &[Row]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    let path = dir.join(format!("{fixture}.json"));
    std::fs::write(&path, json(fixture, rows)).map_err(|err| format!("{}: {err}", path.display()))
}

struct Child {
    code: i32,
    stdout: String,
}

fn prove(out: &Path) -> Result<i32, String> {
    std::fs::create_dir_all(out).map_err(|err| format!("{}: {err}", out.display()))?;
    let exe = std::env::current_exe().map_err(|err| format!("current exe: {err}"))?;
    let good = run_child(&exe, out, "good")?;
    let bad = run_child(&exe, out, "bad")?;
    let live = run_child(&exe, out, "live")?;
    println!("# good");
    print!("{}", good.stdout);
    println!("# bad");
    print!("{}", bad.stdout);
    println!("# live");
    print!("{}", live.stdout);
    if good.code != 0 || bad.code != 1 || live.code != 0 {
        return Err(format!(
            "proof exits good {} bad {} live {}",
            good.code, bad.code, live.code
        ));
    }
    expect_lines(&good.stdout, "PASS")?;
    expect_lines(&bad.stdout, "FAIL")?;
    expect_lines(&live.stdout, "NOT-IMPLEMENTED")?;
    expect_file(out, "good", "PASS", 0)?;
    expect_file(out, "bad", "FAIL", METRICS.len())?;
    expect_file(out, "live", "NOT-IMPLEMENTED", 0)?;
    Ok(0)
}

fn run_child(exe: &Path, out: &Path, fixture: &str) -> Result<Child, String> {
    let output = Command::new(exe)
        .arg("--fixture")
        .arg(fixture)
        .arg("--out")
        .arg(out)
        .output()
        .map_err(|err| format!("spawn {}: {err}", exe.display()))?;
    let stdout =
        String::from_utf8(output.stdout).map_err(|_| "child stdout is not utf-8".to_string())?;
    let code = output
        .status
        .code()
        .ok_or_else(|| format!("{fixture} did not exit"))?;
    if code == 2 {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("{fixture}: {stderr}{stdout}"));
    }
    Ok(Child { code, stdout })
}

fn expect_lines(text: &str, status: &str) -> Result<(), String> {
    let mut count = 0usize;
    for row in text.lines() {
        if row.is_empty() {
            continue;
        }
        let last = row
            .split(' ')
            .next_back()
            .ok_or_else(|| format!("blank line in {status}"))?;
        if last != status {
            return Err(format!("{row} wanted {status}"));
        }
        count += 1;
    }
    if count != METRICS.len() {
        return Err(format!("{count} {status} lines, wanted {}", METRICS.len()));
    }
    Ok(())
}

fn expect_file(dir: &Path, fixture: &str, status: &str, failed: usize) -> Result<(), String> {
    let path = dir.join(format!("{fixture}.json"));
    let text =
        std::fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    let needle = format!("\"status\": \"{status}\"");
    let count = text.matches(&needle).count();
    if count != METRICS.len() {
        return Err(format!("{fixture} json has {count} {status}"));
    }
    let failed_needle = format!("\"failed\": {failed}");
    if !text.contains(&failed_needle) {
        return Err(format!("{fixture} json missing {failed_needle}"));
    }
    if !text.contains(&format!("\"fixture\": \"{fixture}\"")) {
        return Err(format!("{fixture} json has the wrong fixture"));
    }
    Ok(())
}
