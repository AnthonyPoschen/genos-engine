//! Headless checks for the job scheduler.
//!
//! The binary writes `metrics.json` under `./genos-debug/jobs/` unless `--out`
//! names another directory. It does not open a window.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use genos_jobs::{
    hash_slots, Class, Job, JobSettings, ManualClock, Order, ScheduleError, Scheduler,
};

/// Arguments for `genos-jobs`.
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// Run the fixed serial-versus-parallel check.
    pub check: bool,
    /// Directory for `metrics.json`. `None` is `./genos-debug/jobs`.
    pub out: Option<PathBuf>,
    /// Planted bug. The check must fail while this is set.
    pub mutate: Option<String>,
    /// `--jobs-workers`. `None` means the environment, then 2.
    pub workers: Option<u32>,
    /// `--jobs-order`. `None` means the environment, then parallel.
    pub order: Option<Order>,
    /// `--jobs-budget-ms`. `None` means the environment, then 6.
    pub budget_ms: Option<f32>,
    /// Print the usage text.
    pub help: bool,
}

/// Usage text. `--help` prints this and exits 0.
pub const USAGE: &str = "\
genos-jobs: headless scheduler check

  --check                 serial and parallel hashes match; a nested run is rejected
  --jobs-workers N        worker threads, not counting this process (default 2)
  --jobs-order MODE       serial or parallel (default parallel)
  --jobs-budget-ms MS     Must ceiling in milliseconds (default 6)
  --out DIR               write metrics.json here (default ./genos-debug/jobs)
  --mutate NAME           planted bug: unordered_slots, server_runs_parallel,
                          nested_wait, slack_in_fence, or mislabeled
  -h, --help              this text

GENOS_JOBS_WORKERS, GENOS_JOBS_ORDER, and GENOS_JOBS_BUDGET_MS apply only when
the matching argument was omitted. The argument wins when both are set.

A passing --check writes metrics.json and exits 0. A mutation exits non-zero.
";

const MUTATIONS: [&str; 5] = [
    "unordered_slots",
    "server_runs_parallel",
    "nested_wait",
    "slack_in_fence",
    "mislabeled",
];

/// One check's probes.
#[derive(Clone, Debug)]
pub struct CheckStats {
    /// Hash of the serial reduce.
    pub hash: u64,
    /// Milliseconds the Must class took on the injected clock.
    pub must_wall_ms: f64,
    /// Milliseconds charged to the `physics` label.
    pub physics_ms: f64,
    /// Nested `run` calls rejected.
    pub rejected: u32,
    /// Slack jobs that ran while the seat was held. Correct code stays at 0.
    pub slack_during_fence: u32,
}

/// Parse `args`, which are the process arguments after the program name.
///
/// # Errors
///
/// An unknown flag, a missing value, or an unknown `--mutate` name.
pub fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Options, String> {
    let mut options = Options {
        check: false,
        out: None,
        mutate: None,
        workers: None,
        order: None,
        budget_ms: None,
        help: false,
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => options.help = true,
            "--check" => options.check = true,
            "--out" => {
                let Some(dir) = args.next() else {
                    return Err("--out needs a directory".into());
                };
                options.out = Some(PathBuf::from(dir));
            }
            "--jobs-workers" => {
                let Some(value) = args.next() else {
                    return Err("--jobs-workers needs an integer".into());
                };
                options.workers = Some(
                    value
                        .parse()
                        .map_err(|_| format!("--jobs-workers needs an integer, got {value}"))?,
                );
            }
            "--jobs-order" => {
                let Some(value) = args.next() else {
                    return Err("--jobs-order needs serial or parallel".into());
                };
                options.order = Some(parse_order(&value)?);
            }
            "--jobs-budget-ms" => {
                let Some(value) = args.next() else {
                    return Err("--jobs-budget-ms needs a number".into());
                };
                options.budget_ms = Some(parse_budget(&value)?);
            }
            "--mutate" => {
                let Some(name) = args.next() else {
                    return Err("--mutate needs a name".into());
                };
                if !MUTATIONS.contains(&name.as_str()) {
                    return Err(format!("unknown mutation {name}"));
                }
                options.mutate = Some(name);
            }
            other => return Err(format!("unknown argument {other}\n\n{USAGE}")),
        }
    }
    Ok(options)
}

/// Fill empty CLI slots from the environment. A set argument is left alone.
///
/// # Errors
///
/// A set variable that does not parse.
#[allow(
    clippy::disallowed_methods,
    reason = "GENOS_JOBS_WORKERS, GENOS_JOBS_ORDER, and GENOS_JOBS_BUDGET_MS mirror the CLI and are read only when that argument was omitted"
)]
pub fn apply_env(options: &mut Options) -> Result<(), String> {
    if options.workers.is_none() {
        if let Ok(value) = std::env::var("GENOS_JOBS_WORKERS") {
            options.workers = Some(
                value
                    .parse()
                    .map_err(|_| format!("GENOS_JOBS_WORKERS needs an integer, got {value}"))?,
            );
        }
    }
    if options.order.is_none() {
        if let Ok(value) = std::env::var("GENOS_JOBS_ORDER") {
            options.order = Some(parse_order(&value)?);
        }
    }
    if options.budget_ms.is_none() {
        if let Ok(value) = std::env::var("GENOS_JOBS_BUDGET_MS") {
            options.budget_ms = Some(parse_budget(&value)?);
        }
    }
    Ok(())
}

/// The directory a run writes into.
#[must_use]
pub fn output_dir(options: &Options) -> PathBuf {
    options
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from("genos-debug/jobs"))
}

/// Run the fixed check.
///
/// `--mutate` plants the bug in the scheduler. The assertions are the ones
/// correct code must pass, so the check fails while the mutation is on.
///
/// # Errors
///
/// The check failed, or a scheduler call failed.
pub fn run_check(options: &Options) -> Result<CheckStats, String> {
    let workers = options.workers.unwrap_or(2);
    let budget_ms = options.budget_ms.unwrap_or(6.0);
    let mutate = options.mutate.as_deref();
    let serial = run_grid(
        Order::Serial,
        0,
        budget_ms,
        plant_named(mutate, "mislabeled"),
    )?;
    let parallel = run_grid(
        Order::Parallel,
        workers,
        budget_ms,
        plant_named(mutate, "unordered_slots"),
    )?;
    let again = run_grid(
        Order::Serial,
        0,
        budget_ms,
        plant_named(mutate, "server_runs_parallel"),
    )?;
    if serial.hash != parallel.hash {
        return Err("parallel hash diverged from serial".into());
    }
    if serial.hash != again.hash {
        return Err("two serial runs diverged".into());
    }
    let rejected = nested_is_rejected(mutate == Some("nested_wait"))?;
    if !rejected {
        return Err("a nested run was accepted".into());
    }
    let slack = slack_during_seat(mutate == Some("slack_in_fence"))?;
    if slack.ran || slack.during_fence != 0 {
        return Err("slack ran during the seat".into());
    }
    let physics_ms = serial.physics_ms;
    if physics_ms <= 0.0 || serial.must_wall_ms <= 0.0 {
        return Err("physics_ms or must_wall_ms was zero".into());
    }
    let hash = match options.order.unwrap_or(Order::Parallel) {
        Order::Serial => serial.hash,
        Order::Parallel => parallel.hash,
    };
    Ok(CheckStats {
        hash,
        must_wall_ms: serial.must_wall_ms,
        physics_ms,
        rejected: u32::from(rejected),
        slack_during_fence: slack.during_fence,
    })
}

struct Grid {
    hash: u64,
    must_wall_ms: f64,
    physics_ms: f64,
}

fn plant_named<'a>(mutate: Option<&'a str>, name: &str) -> Option<&'a str> {
    mutate.filter(|value| *value == name)
}

fn run_grid(
    order: Order,
    workers: u32,
    budget_ms: f32,
    plant: Option<&str>,
) -> Result<Grid, String> {
    let clock = Arc::new(ManualClock::new(0.0));
    let scheduler = Scheduler::new(
        JobSettings {
            workers,
            order,
            frame_budget_ms: budget_ms,
            slack: true,
        },
        ManualClock::handle(&clock),
    );
    if let Some(name) = plant {
        scheduler.plant(name)?;
    }
    let moved = Arc::clone(&clock);
    let mut jobs = vec![Job {
        stage: 0,
        index: 0,
        class: Class::Must,
        label: "physics",
        access: Vec::new(),
        work: Box::new(move || {
            moved.set(1.0);
            0
        }),
    }];
    for index in 1..4u32 {
        jobs.push(Job {
            stage: 0,
            index,
            class: Class::Must,
            label: "move",
            access: Vec::new(),
            work: Box::new(move || u64::from(index)),
        });
    }
    let report = scheduler.run(jobs).map_err(|err| err.to_string())?;
    let physics_ms = report
        .stats
        .labels
        .iter()
        .find(|(label, _)| *label == "physics")
        .map_or(0.0, |(_, elapsed)| *elapsed);
    Ok(Grid {
        hash: hash_slots(&report.stages),
        must_wall_ms: report.stats.must_wall_ms,
        physics_ms,
    })
}

fn nested_is_rejected(plant: bool) -> Result<bool, String> {
    let scheduler = Arc::new(Scheduler::serial());
    if plant {
        scheduler.plant("nested_wait")?;
    }
    let nested = Arc::clone(&scheduler);
    let report = scheduler
        .run(vec![Job {
            stage: 0,
            index: 0,
            class: Class::Must,
            label: "nested",
            access: Vec::new(),
            work: Box::new(move || {
                u64::from(matches!(
                    nested.run(Vec::<Job<u64>>::new()),
                    Err(ScheduleError::Rejected)
                ))
            }),
        }])
        .map_err(|err| err.to_string())?;
    Ok(report
        .stages
        .first()
        .and_then(|stage| stage.slots.first().copied())
        .flatten()
        == Some(1))
}

struct SlackProbe {
    ran: bool,
    during_fence: u32,
}

fn slack_during_seat(plant: bool) -> Result<SlackProbe, String> {
    let scheduler = Scheduler::new(
        JobSettings {
            workers: 0,
            order: Order::Serial,
            frame_budget_ms: 6.0,
            slack: true,
        },
        Arc::new(ManualClock::new(0.0)),
    );
    if plant {
        scheduler.plant("slack_in_fence")?;
    }
    scheduler.suspend_slack();
    let report = scheduler
        .run(vec![Job {
            stage: 0,
            index: 0,
            class: Class::Slack,
            label: "stream",
            access: Vec::new(),
            work: Box::new(|| 5),
        }])
        .map_err(|err| err.to_string())?;
    let ran = report
        .stages
        .first()
        .and_then(|stage| stage.slots.first())
        .is_some_and(Option::is_some);
    Ok(SlackProbe {
        ran,
        during_fence: report.stats.slack_during_fence,
    })
}

/// `metrics.json` for one check.
#[must_use]
pub fn metrics_json(stats: &CheckStats) -> String {
    format!(
        "{{\n  \"hash\": {},\n  \"must_wall_ms\": {},\n  \"physics_ms\": {},\n  \"rejected\": {},\n  \"slack_during_fence\": {}\n}}\n",
        stats.hash, stats.must_wall_ms, stats.physics_ms, stats.rejected, stats.slack_during_fence
    )
}

/// Write `metrics.json` under `dir`.
///
/// # Errors
///
/// The directory or the file could not be created.
pub fn write_metrics(dir: &Path, body: &str) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|err| err.to_string())?;
    let path = dir.join("metrics.json");
    fs::write(&path, body).map_err(|err| err.to_string())?;
    Ok(path)
}

/// Run one invocation. The string is the text to print.
///
/// # Errors
///
/// The check failed. The message is the reason.
pub fn execute(options: &Options) -> Result<String, String> {
    if options.help {
        return Ok(USAGE.to_string());
    }
    if !options.check {
        return Err("pass --check or --help".into());
    }
    let stats = run_check(options)?;
    let body = metrics_json(&stats);
    let path = write_metrics(&output_dir(options), &body)?;
    Ok(format!("wrote {}\n{body}", path.display()))
}

fn parse_order(value: &str) -> Result<Order, String> {
    match value {
        "serial" => Ok(Order::Serial),
        "parallel" => Ok(Order::Parallel),
        _ => Err(format!("order is serial or parallel, got {value}")),
    }
}

fn parse_budget(value: &str) -> Result<f32, String> {
    let budget: f32 = value
        .parse()
        .map_err(|_| format!("budget needs a number, got {value}"))?;
    if !budget.is_finite() || budget < 0.0 {
        return Err("budget must be finite and >= 0".into());
    }
    Ok(budget)
}
