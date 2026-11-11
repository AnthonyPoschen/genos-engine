//! `genos-ecs`: the scene-store debug binary. Headless; no window.

use std::process::ExitCode;

use genos_ecs_debug::{execute, parse_args, USAGE};

fn main() -> ExitCode {
    let options = match parse_args(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(2);
        }
    };
    match execute(&options) {
        Ok(text) => {
            print!("{text}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("{err}\n\n{USAGE}");
            ExitCode::from(1)
        }
    }
}
