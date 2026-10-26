//! Arguments. An environment variable is applied only when its argument was omitted.

use std::path::PathBuf;

use crate::{Fixture, Suite};

/// Resolved invocation. `help` skips the metrics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// Which metrics to score.
    pub suite: Suite,
    /// Analytic fixture. Ignored by the feature suite.
    pub fixture: Fixture,
    /// Directory for `metrics.json`.
    pub out: PathBuf,
    /// `--help` was set.
    pub help: bool,
}

/// Parse arguments, then apply mirrors for the slots the arguments left empty.
///
/// # Errors
///
/// Returns an error on an unknown flag, a missing value, or a bad token.
pub fn resolve(
    args: &[String],
    env_suite: Option<&str>,
    env_fixture: Option<&str>,
    env_out: Option<&str>,
) -> Result<Config, String> {
    let parsed = parse(args)?;
    if parsed.help {
        return Ok(Config {
            suite: Suite::All,
            fixture: Fixture::Good,
            out: default_out(),
            help: true,
        });
    }
    let suite = if let Some(suite) = parsed.suite {
        suite
    } else {
        match env_suite {
            Some(text) => parse_suite(text)?,
            None => Suite::All,
        }
    };
    let fixture = if let Some(fixture) = parsed.fixture {
        fixture
    } else {
        match env_fixture {
            Some(text) => parse_fixture(text)?,
            None => Fixture::Good,
        }
    };
    let out = if let Some(out) = parsed.out {
        out
    } else {
        match env_out {
            Some(text) if !text.is_empty() => PathBuf::from(text),
            Some(_) => return Err("GENOS_FOG_OUT is empty".to_string()),
            None => default_out(),
        }
    };
    Ok(Config {
        suite,
        fixture,
        out,
        help: false,
    })
}

struct Parsed {
    suite: Option<Suite>,
    fixture: Option<Fixture>,
    out: Option<PathBuf>,
    help: bool,
}

fn parse(args: &[String]) -> Result<Parsed, String> {
    let mut suite = None;
    let mut fixture = None;
    let mut out = None;
    let mut help = false;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--help" || arg == "-h" {
            help = true;
            index += 1;
            continue;
        }
        if let Some(value) = arg.strip_prefix("--suite=") {
            suite = Some(parse_suite(value)?);
            index += 1;
            continue;
        }
        if arg == "--suite" {
            suite = Some(parse_suite(required(args, index, "--suite")?)?);
            index += 2;
            continue;
        }
        if let Some(value) = arg.strip_prefix("--fixture=") {
            fixture = Some(parse_fixture(value)?);
            index += 1;
            continue;
        }
        if arg == "--fixture" {
            fixture = Some(parse_fixture(required(args, index, "--fixture")?)?);
            index += 2;
            continue;
        }
        if let Some(value) = arg.strip_prefix("--out=") {
            out = Some(parse_out(value)?);
            index += 1;
            continue;
        }
        if arg == "--out" {
            out = Some(parse_out(required(args, index, "--out")?)?);
            index += 2;
            continue;
        }
        return Err(format!("unknown argument {arg}"));
    }
    Ok(Parsed {
        suite,
        fixture,
        out,
        help,
    })
}

fn required<'a>(args: &'a [String], index: usize, flag: &str) -> Result<&'a str, String> {
    args.get(index + 1)
        .map(String::as_str)
        .ok_or_else(|| format!("{flag} needs a value"))
}

fn parse_suite(text: &str) -> Result<Suite, String> {
    match text {
        "analytic" => Ok(Suite::Analytic),
        "feature" => Ok(Suite::Feature),
        "all" => Ok(Suite::All),
        _ => Err(format!("bad suite {text} (analytic, feature, or all)")),
    }
}

fn parse_fixture(text: &str) -> Result<Fixture, String> {
    match text {
        "good" => Ok(Fixture::Good),
        "broken" => Ok(Fixture::Broken),
        _ => Err(format!("bad fixture {text} (good or broken)")),
    }
}

fn parse_out(text: &str) -> Result<PathBuf, String> {
    if text.is_empty() {
        return Err("--out needs a directory".to_string());
    }
    Ok(PathBuf::from(text))
}

fn default_out() -> PathBuf {
    PathBuf::from("genos-debug/fog")
}

pub const HELP: &str = "\
fog-check — fog and volumetric lighting metrics

Arguments:
  --suite analytic|feature|all   metrics to score (default all)
  --fixture good|broken          analytic input (default good)
  --out DIR                      directory for metrics.json (default genos-debug/fog)
  --help                         this text

GENOS_FOG_SUITE, GENOS_FOG_FIXTURE, and GENOS_FOG_OUT apply only when the
matching argument was omitted. The argument wins. Output stays under the
working directory unless --out names another path.

Exit status: 0 when no metric failed, including NOT-IMPLEMENTED rows.
1 when any metric failed. 2 on a bad argument.
";
