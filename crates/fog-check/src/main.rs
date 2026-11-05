//! `fog-check` prints one line per metric and writes `metrics.json`.

use genos_fog_check::{evaluate, exit_code, format_lines, resolve, write_metrics};

fn main() -> std::process::ExitCode {
    let code = match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("fog-check: {err}");
            match err {
                RunError::Argument(_) => 2,
                RunError::Write(_) => 1,
            }
        }
    };
    std::process::ExitCode::from(u8::try_from(code).unwrap_or(1))
}

enum RunError {
    Argument(String),
    Write(String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Argument(err) | Self::Write(err) => f.write_str(err),
        }
    }
}

fn run() -> Result<i32, RunError> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config = resolve(
        &args,
        env_mirror("GENOS_FOG_SUITE")?.as_deref(),
        env_mirror("GENOS_FOG_FIXTURE")?.as_deref(),
        env_mirror("GENOS_FOG_OUT")?.as_deref(),
    )
    .map_err(RunError::Argument)?;
    if config.help {
        print!("{}", genos_fog_check::HELP);
        return Ok(0);
    }
    let report = evaluate(config.suite, config.fixture);
    print!("{}", format_lines(&report));
    write_metrics(&config.out, &report).map_err(RunError::Write)?;
    Ok(exit_code(&report))
}

/// Process edge. The library applies a mirror only when the argument was omitted.
#[allow(
    clippy::disallowed_methods,
    reason = "process edge: GENOS_FOG_* mirrors a CLI argument and is passed down as text"
)]
fn env_mirror(name: &str) -> Result<Option<String>, RunError> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err(RunError::Argument(format!("{name} is not Unicode")))
        }
    }
}
