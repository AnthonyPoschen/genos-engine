//! Headless checks for the scene store.
//!
//! The binary writes `metrics.json` under `./genos-debug/ecs/` unless `--out`
//! names another directory. It does not open a window.

use std::fs;
use std::path::{Path, PathBuf};

use genos_ecs::{push_u32, read_u32, Component, DecodeError, Stats, StoreError, World};

/// Arguments for `genos-ecs`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// Run the fixed spawn, edit, and stale-handle check.
    pub check: bool,
    /// Directory for `metrics.json`. `None` is `./genos-debug/ecs`.
    pub out: Option<PathBuf>,
    /// Planted bug. The check must fail while this is set.
    pub mutate: Option<String>,
    /// Print the usage text.
    pub help: bool,
}

/// Usage text. `--help` prints this and exits 0.
pub const USAGE: &str = "\
genos-ecs: headless scene-store check

  --check            spawn, edit one component, reject a stale handle
  --out DIR          write metrics.json here (default ./genos-debug/ecs)
  --mutate NAME      planted bug: stale_accept or skip_tick
  -h, --help         this text

A passing --check writes metrics.json and exits 0. A mutation exits non-zero.
";

#[derive(Clone, Copy)]
struct Health(u32);

impl Component for Health {
    fn component_name() -> &'static str {
        "health"
    }

    fn encode(&self, out: &mut Vec<u8>) {
        push_u32(out, self.0);
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut input = bytes;
        let value = read_u32(&mut input)?;
        if !input.is_empty() {
            return Err(DecodeError::Trailing);
        }
        Ok(Self(value))
    }
}

/// Parse `args`, which are the process arguments after the program name.
///
/// # Errors
///
/// An unknown flag, a missing `--out` directory, or an unknown `--mutate` name.
pub fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Options, String> {
    let mut options = Options {
        check: false,
        out: None,
        mutate: None,
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
            "--mutate" => {
                let Some(name) = args.next() else {
                    return Err("--mutate needs a name".into());
                };
                if name != "stale_accept" && name != "skip_tick" {
                    return Err(format!("unknown mutation {name}"));
                }
                options.mutate = Some(name);
            }
            other => return Err(format!("unknown argument {other}\n\n{USAGE}")),
        }
    }
    Ok(options)
}

/// The directory a run writes into.
#[must_use]
pub fn output_dir(options: &Options) -> PathBuf {
    options
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from("genos-debug/ecs"))
}

/// Run the fixed check.
///
/// `--mutate skip_tick` drops the edit, so no health delta is emitted.
/// `--mutate stale_accept` treats the despawned handle as a successful write.
/// The assertions stay the ones a correct store must pass, so either planted
/// bug makes the check fail.
///
/// # Errors
///
/// The check failed, or the store refused a live write.
pub fn run_check(mutate: Option<&str>) -> Result<Stats, String> {
    let mut world = World::new();
    let handle = world.spawn_with(Health(1)).map_err(|err| err.to_string())?;
    let cursor = world.clock();
    if mutate == Some("skip_tick") {
        // Planted bug: the edit is discarded, so the cell keeps its old tick.
    } else {
        world
            .insert(handle, Health(2))
            .map_err(|err| err.to_string())?;
    }
    let deltas = world.changes_since(cursor);
    let tick_moved = deltas.len() == 1
        && deltas
            .first()
            .is_some_and(|delta| delta.component == "health" && !delta.bytes.is_empty());
    if !tick_moved {
        return Err("an edit did not emit a health delta".into());
    }
    world.despawn(handle).map_err(|err| err.to_string())?;
    let stale = if mutate == Some("stale_accept") {
        // Planted bug: a despawned handle is treated as a successful write.
        Ok(())
    } else {
        world.insert(handle, Health(3))
    };
    if stale != Err(StoreError::Stale) {
        return Err("a despawned handle accepted a write".into());
    }
    world.audit()?;
    if world.stats().stale_rejects != 1 {
        return Err("stale_rejects did not count the refused write".into());
    }
    Ok(world.stats())
}

/// `metrics.json` for one check.
#[must_use]
pub fn metrics_json(stats: &Stats, audit: &str) -> String {
    format!(
        "{{\n  \"alive\": {},\n  \"free\": {},\n  \"archetypes\": {},\n  \"clock\": {},\n  \"stale_rejects\": {},\n  \"audit\": \"{audit}\"\n}}\n",
        stats.alive, stats.free, stats.archetypes, stats.clock, stats.stale_rejects
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
    let stats = run_check(options.mutate.as_deref())?;
    let body = metrics_json(&stats, "ok");
    let path = write_metrics(&output_dir(options), &body)?;
    Ok(format!("wrote {}\n{body}", path.display()))
}
