//! Headless GPU initial `ReSTIR` DI candidates.
//!
//! Off unless `--initial` is passed. `GENOS_RESTIR_INITIAL` is the mirror and
//! applies only when `--initial` and `--no-initial` were omitted.
//! `GENOS_RESTIR_OUT` mirrors `--out`.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print!("{text}", text = genos_restir_gpu::help_text());
        return ExitCode::from(0);
    }
    let initial_env = env_mirror("GENOS_RESTIR_INITIAL");
    let out_env = env_mirror("GENOS_RESTIR_OUT");
    match genos_restir_gpu::parse_args(&args, initial_env.as_deref(), out_env.as_deref())
        .and_then(|options| genos_restir_gpu::run(&options))
    {
        Ok(_) => ExitCode::from(0),
        Err(err) => {
            if err == "help" {
                print!("{text}", text = genos_restir_gpu::help_text());
                ExitCode::from(0)
            } else {
                eprintln!("restir-gpu: {err}");
                ExitCode::from(2)
            }
        }
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "GENOS_RESTIR_INITIAL and GENOS_RESTIR_OUT are read once in main"
)]
fn env_mirror(name: &str) -> Option<String> {
    std::env::var(name).ok()
}
