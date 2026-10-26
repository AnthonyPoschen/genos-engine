//! `fog-check` prints one line per metric and writes `metrics.json`.

use genos_fog_check::{evaluate, exit_code, format_lines, resolve, write_metrics};

fn main() {
    let code = match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("fog-check: {err}");
            1
        }
    };
    if code != 0 {
        std::process::exit(code);
    }
}

fn run() -> Result<i32, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config = resolve(
        &args,
        env_mirror("GENOS_FOG_SUITE")?.as_deref(),
        env_mirror("GENOS_FOG_FIXTURE")?.as_deref(),
        env_mirror("GENOS_FOG_OUT")?.as_deref(),
    )?;
    if config.help {
        print!("{}", genos_fog_check::HELP);
        return Ok(0);
    }
    let report = evaluate(config.suite, config.fixture);
    print!("{}", format_lines(&report));
    write_metrics(&config.out, &report)?;
    Ok(exit_code(&report))
}

/// Process edge. The library applies a mirror only when the argument was omitted.
#[allow(
    clippy::disallowed_methods,
    reason = "process edge: GENOS_FOG_* mirrors a CLI argument and is passed down as text"
)]
fn env_mirror(name: &str) -> Result<Option<String>, String> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not Unicode")),
    }
}
