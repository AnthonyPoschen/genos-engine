//! Deterministic rhai scripts over the debug commands, for regression tests.
//!
//! The script runs on its own thread. Every command goes through the same queue as
//! the MCP port, so the frame runs it at the same point a live client's would. A
//! script that sets `time(#{ pause: true })` and steps frames sees the same frames
//! on every run, up to the probe budget (see `docs/systems/debugging.md`).
//!
//! Each command is a function of the same name taking a map (`camera(#{ yaw: 1.0 })`)
//! or nothing. Shorthands: `wait(n)`, `step(n)`, `view("direct")`, `knob("day", 0.5)`,
//! `knob("day")`, `shot("name")`, `shot("name", "mode")`. Checks: `check(cond, msg)`
//! records and goes on, `require(cond, msg)` stops the script, `check_lt(a, b, msg)`,
//! `check_gt(a, b, msg)`, `check_near(a, b, tol, msg)`. `log(text)` adds a line.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use genos_mcp::json::{self, Number, Value};
use rhai::{Array, Dynamic, Engine, EvalAltResult, Map, NativeCallContext, Position, INT};

/// One script run.
#[derive(Clone, Debug)]
pub struct ScriptRun {
    pub script: PathBuf,
    /// The report, the captures and the trace go here.
    pub report_dir: PathBuf,
    /// Capture the picture after every command that changes something.
    pub trace: bool,
}

#[derive(Default)]
struct Report {
    steps: Vec<String>,
    checks: Vec<(bool, String)>,
    images: Vec<String>,
    logs: Vec<String>,
    trace_index: u32,
}

/// Commands that change what the picture shows; `trace` captures after each.
const CHANGES: [&str; 10] = [
    "light",
    "object",
    "camera",
    "time",
    "step",
    "wait",
    "wait_settled",
    "view",
    "knobs",
    "lighting",
];

/// Run the script on a new thread. When it ends it writes the report and asks the
/// frame to quit with 0 (every check passed) or 1.
pub fn spawn(run: ScriptRun) -> std::thread::JoinHandle<bool> {
    std::thread::spawn(move || {
        let ok = run_script(&run);
        let code = if ok { 0 } else { 1 };
        let _ = genos_mcp::command::call(
            "quit",
            json::object([("code", json::int(code))]),
            Duration::from_secs(30),
        );
        ok
    })
}

/// Run the script here (blocking) against a frame that serves commands.
pub fn run_script(run: &ScriptRun) -> bool {
    let started = Instant::now();
    let report = Rc::new(RefCell::new(Report::default()));
    let _ = std::fs::create_dir_all(&run.report_dir);
    // The frame thread may still be opening the window.
    let deadline = Instant::now() + Duration::from_secs(300);
    while genos_mcp::command::call("status", json::object([]), Duration::from_secs(5)).is_err() {
        if Instant::now() > deadline {
            report
                .borrow_mut()
                .logs
                .push("no frame served commands in 300 s".into());
            write_report(run, &report.borrow(), Err("no frame".into()), started);
            return false;
        }
    }
    let source = match std::fs::read_to_string(&run.script) {
        Ok(text) => text,
        Err(err) => {
            let message = format!("{}: {err}", run.script.display());
            write_report(run, &report.borrow(), Err(message), started);
            return false;
        }
    };
    let engine = engine(run, report.clone());
    let result = engine.run(&source).map_err(|err| err.to_string());
    let rep = report.borrow();
    let ok = result.is_ok() && rep.checks.iter().all(|(pass, _)| *pass);
    write_report(run, &rep, result, started);
    ok
}

fn engine(run: &ScriptRun, report: Rc<RefCell<Report>>) -> Engine {
    let mut engine = Engine::new();
    engine.set_max_expr_depths(128, 64);
    let trace = run.trace;
    for spec in crate::commands::specs() {
        let name = spec.name.clone();
        let r = report.clone();
        engine.register_fn(
            spec.name.as_str(),
            move |ctx: NativeCallContext, args: Map| -> Result<Dynamic, Box<EvalAltResult>> {
                call(&r, trace, ctx.call_position(), &name, map_value(args))
            },
        );
        let name = spec.name.clone();
        let r = report.clone();
        engine.register_fn(
            spec.name.as_str(),
            move |ctx: NativeCallContext| -> Result<Dynamic, Box<EvalAltResult>> {
                call(&r, trace, ctx.call_position(), &name, json::object([]))
            },
        );
    }
    let r = report.clone();
    engine.register_fn("wait", move |ctx: NativeCallContext, n: INT| {
        call(
            &r,
            trace,
            ctx.call_position(),
            "wait",
            json::object([("frames", json::int(n))]),
        )
    });
    let r = report.clone();
    engine.register_fn("step", move |ctx: NativeCallContext, n: INT| {
        call(
            &r,
            trace,
            ctx.call_position(),
            "step",
            json::object([("frames", json::int(n))]),
        )
    });
    let r = report.clone();
    engine.register_fn("view", move |ctx: NativeCallContext, mode: &str| {
        call(
            &r,
            trace,
            ctx.call_position(),
            "view",
            json::object([("mode", json::string(mode))]),
        )
    });
    let r = report.clone();
    engine.register_fn(
        "knob",
        move |ctx: NativeCallContext, name: &str, value: Dynamic| {
            let v = dynamic_value(value);
            call(
                &r,
                trace,
                ctx.call_position(),
                "knobs",
                json::object([("name", json::string(name)), ("value", v)]),
            )
            .map(|_| Dynamic::UNIT)
        },
    );
    let r = report.clone();
    engine.register_fn(
        "knob",
        move |ctx: NativeCallContext, name: &str| -> Result<Dynamic, Box<EvalAltResult>> {
            let all = call(&r, false, ctx.call_position(), "knobs", json::object([]))?;
            for k in all.into_array().unwrap_or_default() {
                if let Some(map) = k.try_cast::<Map>() {
                    if map.get("name").map(|n| n.to_string()) == Some(name.to_string()) {
                        return Ok(map.get("value").cloned().unwrap_or(Dynamic::UNIT));
                    }
                }
            }
            Err(format!("unknown knob {name}").into())
        },
    );
    let r = report.clone();
    engine.register_fn("shot", move |ctx: NativeCallContext, file: &str| {
        let path = format!("{file}.png");
        call(
            &r,
            false,
            ctx.call_position(),
            "capture",
            json::object([("path", json::string(path)), ("name", json::string(file))]),
        )
    });
    let r = report.clone();
    engine.register_fn(
        "shot",
        move |ctx: NativeCallContext, file: &str, mode: &str| {
            let path = format!("{file}.png");
            call(
                &r,
                false,
                ctx.call_position(),
                "capture",
                json::object([
                    ("path", json::string(path)),
                    ("name", json::string(file)),
                    ("mode", json::string(mode)),
                ]),
            )
        },
    );
    let r = report.clone();
    engine.register_fn("log", move |text: &str| {
        r.borrow_mut().logs.push(text.to_string());
    });
    let r = report.clone();
    engine.register_fn(
        "check",
        move |ctx: NativeCallContext, ok: bool, msg: &str| {
            note(&r, ctx.call_position(), ok, msg.to_string());
            ok
        },
    );
    let r = report.clone();
    engine.register_fn(
        "require",
        move |ctx: NativeCallContext, ok: bool, msg: &str| -> Result<(), Box<EvalAltResult>> {
            note(&r, ctx.call_position(), ok, msg.to_string());
            if ok {
                Ok(())
            } else {
                Err(format!("required: {msg}").into())
            }
        },
    );
    let r = report.clone();
    engine.register_fn(
        "check_lt",
        move |ctx: NativeCallContext, a: Dynamic, b: Dynamic, msg: &str| {
            let (a, b) = (as_f64(&a), as_f64(&b));
            let ok = a < b;
            note(
                &r,
                ctx.call_position(),
                ok,
                format!("{msg} ({a:.5} < {b:.5})"),
            );
            ok
        },
    );
    let r = report.clone();
    engine.register_fn(
        "check_gt",
        move |ctx: NativeCallContext, a: Dynamic, b: Dynamic, msg: &str| {
            let (a, b) = (as_f64(&a), as_f64(&b));
            let ok = a > b;
            note(
                &r,
                ctx.call_position(),
                ok,
                format!("{msg} ({a:.5} > {b:.5})"),
            );
            ok
        },
    );
    let r = report;
    engine.register_fn(
        "check_near",
        move |ctx: NativeCallContext, a: Dynamic, b: Dynamic, tol: Dynamic, msg: &str| {
            let (a, b, tol) = (as_f64(&a), as_f64(&b), as_f64(&tol));
            let ok = (a - b).abs() <= tol;
            note(
                &r,
                ctx.call_position(),
                ok,
                format!("{msg} (|{a:.5} - {b:.5}| <= {tol:.5})"),
            );
            ok
        },
    );
    let dir = run.report_dir.display().to_string();
    engine.register_fn("out_dir", move || dir.clone());
    engine
}

fn as_f64(v: &Dynamic) -> f64 {
    v.as_float()
        .ok()
        .or_else(|| v.as_int().ok().map(|i| i as f64))
        .unwrap_or(f64::NAN)
}

fn note(report: &Rc<RefCell<Report>>, at: Position, ok: bool, msg: String) {
    let line = at.line().unwrap_or(0);
    report
        .borrow_mut()
        .checks
        .push((ok, format!("line {line}: {msg}")));
}

fn call(
    report: &Rc<RefCell<Report>>,
    trace: bool,
    at: Position,
    name: &str,
    args: Value,
) -> Result<Dynamic, Box<EvalAltResult>> {
    let started = Instant::now();
    let timeout = genos_mcp::rpc_timeout(&args);
    let shown = short(&json::encode(&args), 160);
    let result = genos_mcp::command::call(name, args, timeout);
    let line = at.line().unwrap_or(0);
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    let mut rep = report.borrow_mut();
    match &result {
        Ok(value) => {
            collect_paths(value, &mut rep.images);
            rep.steps.push(format!(
                "| {line} | `{name}` | `{shown}` | ok {ms:.0} ms | `{}` |",
                short(&json::encode(value), 200)
            ));
        }
        Err(err) => rep.steps.push(format!(
            "| {line} | `{name}` | `{shown}` | **error** | {err} |"
        )),
    }
    drop(rep);
    let value = result.map_err(|err| -> Box<EvalAltResult> { format!("{name}: {err}").into() })?;
    if trace && CHANGES.contains(&name) {
        let index = {
            let mut rep = report.borrow_mut();
            rep.trace_index += 1;
            rep.trace_index
        };
        let path = format!("trace/{index:03}-line{line}-{name}.png");
        if let Ok(v) = genos_mcp::command::call(
            "capture",
            json::object([("path", json::string(path))]),
            Duration::from_secs(120),
        ) {
            collect_paths(&v, &mut report.borrow_mut().images);
        }
    }
    Ok(value_dynamic(&value))
}

fn collect_paths(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(pairs) => {
            for (k, v) in pairs {
                if matches!(k.as_str(), "path" | "out" | "heatmap") {
                    if let Some(p) = v.as_str() {
                        out.push(p.to_string());
                    }
                } else {
                    collect_paths(v, out);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|v| collect_paths(v, out)),
        _ => {}
    }
}

fn short(text: &str, n: usize) -> String {
    let t: String = text.chars().take(n).collect();
    let t = t.replace('|', "/");
    if text.chars().count() > n {
        t + "…"
    } else {
        t
    }
}

fn write_report(run: &ScriptRun, rep: &Report, result: Result<(), String>, started: Instant) {
    let passed = rep.checks.iter().filter(|(ok, _)| *ok).count();
    let failed = rep.checks.len() - passed;
    let ok = result.is_ok() && failed == 0;
    let mut md = format!(
        "# {} — {}\n\nScript: `{}`\nChecks: {passed} passed, {failed} failed. Time: {:.1} s.\n",
        run.script
            .file_name()
            .map_or("script".into(), |n| n.to_string_lossy().to_string()),
        if ok { "PASS" } else { "FAIL" },
        run.script.display(),
        started.elapsed().as_secs_f64(),
    );
    if let Err(err) = &result {
        md.push_str(&format!("\n**Stopped:** {err}\n"));
    }
    md.push_str("\n## Checks\n\n");
    for (ok, msg) in &rep.checks {
        md.push_str(&format!(
            "- {} {msg}\n",
            if *ok { "PASS" } else { "**FAIL**" }
        ));
    }
    if !rep.logs.is_empty() {
        md.push_str("\n## Log\n\n");
        for l in &rep.logs {
            md.push_str(&format!("- {l}\n"));
        }
    }
    md.push_str(
        "\n## Steps\n\n| line | command | args | result | reply |\n|---|---|---|---|---|\n",
    );
    for s in &rep.steps {
        md.push_str(s);
        md.push('\n');
    }
    if !rep.images.is_empty() {
        md.push_str("\n## Images\n\n");
        for p in &rep.images {
            let rel = Path::new(p)
                .strip_prefix(&run.report_dir)
                .map(|r| r.display().to_string())
                .unwrap_or_else(|_| p.clone());
            md.push_str(&format!("- [{rel}]({rel})\n"));
        }
    }
    let _ = std::fs::write(run.report_dir.join("report.md"), md);
    let summary = json::object([
        ("script", json::string(run.script.display().to_string())),
        ("pass", json::bool(ok)),
        ("checks_passed", json::int(passed as i64)),
        ("checks_failed", json::int(failed as i64)),
        ("error", result.err().map_or(Value::Null, json::string)),
        (
            "failures",
            json::array(
                rep.checks
                    .iter()
                    .filter(|(ok, _)| !ok)
                    .map(|(_, m)| json::string(m.clone()))
                    .collect(),
            ),
        ),
    ]);
    let _ = std::fs::write(run.report_dir.join("result.json"), json::encode(&summary));
    eprintln!(
        "genos-debug {}: {} ({passed} checks passed, {failed} failed) report {}",
        run.script.display(),
        if ok { "PASS" } else { "FAIL" },
        run.report_dir.join("report.md").display()
    );
}

fn map_value(map: Map) -> Value {
    Value::Object(
        map.into_iter()
            .map(|(k, v)| (k.to_string(), dynamic_value(v)))
            .collect(),
    )
}

fn dynamic_value(v: Dynamic) -> Value {
    if v.is_unit() {
        Value::Null
    } else if let Ok(b) = v.as_bool() {
        Value::Bool(b)
    } else if let Ok(i) = v.as_int() {
        json::int(i)
    } else if let Ok(f) = v.as_float() {
        Value::Number(Number::Float(f))
    } else if v.is_string() {
        json::string(v.into_string().unwrap_or_default())
    } else if v.is_array() {
        json::array(
            v.into_array()
                .unwrap_or_default()
                .into_iter()
                .map(dynamic_value)
                .collect(),
        )
    } else if v.is_map() {
        map_value(v.cast::<Map>())
    } else {
        json::string(v.to_string())
    }
}

fn value_dynamic(v: &Value) -> Dynamic {
    match v {
        Value::Null => Dynamic::UNIT,
        Value::Bool(b) => Dynamic::from(*b),
        Value::Number(Number::Int(i)) => Dynamic::from(*i as INT),
        Value::Number(Number::Float(f)) => Dynamic::from(*f),
        Value::String(s) => Dynamic::from(s.clone()),
        Value::Array(items) => Dynamic::from(items.iter().map(value_dynamic).collect::<Array>()),
        Value::Object(pairs) => {
            let mut map = Map::new();
            for (k, v) in pairs {
                map.insert(k.as_str().into(), value_dynamic(v));
            }
            Dynamic::from(map)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_round_trip_through_rhai() {
        let v = json::object([
            ("a", json::int(3)),
            (
                "b",
                json::array(vec![Value::Number(Number::Float(1.5)), json::bool(true)]),
            ),
            ("c", json::string("x")),
        ]);
        let d = value_dynamic(&v);
        let back = dynamic_value(d);
        assert_eq!(back.get("a").and_then(Value::as_i64), Some(3));
        assert_eq!(back.get("c").and_then(Value::as_str), Some("x"));
    }
}
