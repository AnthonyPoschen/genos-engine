//! `genos-particles`: the stage-1 particle debug binary.
//!
//! Headless. It runs the CPU mirror and prints the `fx.*` stats line.
//! Times stay 0 here. The GPU timestamps and the picture proof are
//! `cargo test -p genos-render --test particles_gpu`.

use std::process::ExitCode;

use genos_render::{CpuParticles, ParticleStats, StepInput};
use genos_scene::{ParticleCli, ParticleEnv, ParticleSettings};

const USAGE: &str = "\
genos-particles: stage-1 particle oracle (CPU mirror, no window)

  --frames N             steps to run (default 8)
  --dt N                 seconds per step (default 1/60)
  --fx-preset NAME       low, medium, or high (quality knobs)
  --fx-max N             buffer budget
  --fx-count N           live cap. Omitted: this binary uses 64 and turns the emitter on
  --fx-freeze            hold positions
  --fx-spawn X,Y,Z       origin, and turn the emitter on
  -h, --help             this text

A plain run prints one fx.* line. sim_ms, sort_ms, and draw_ms are 0.
sort_ms and overdraw_px stay 0. This is not a desktop-GPU score.
The scene default stays off with count 0. The 64 is only this binary's omitted count.
";

struct Options {
    help: bool,
    frames: u32,
    dt: f32,
    cli: ParticleCli,
    count_set: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("genos-particles: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let opts = parse(std::env::args().skip(1))?;
    if opts.help {
        print!("{USAGE}");
        return Ok(());
    }
    let mut settings = ParticleSettings::default().resolve(&opts.cli, &ParticleEnv::default());
    if !opts.count_set {
        settings.count = 64;
    }
    if settings.count > 0 {
        settings.enabled = true;
    }
    let mut sim = CpuParticles::new();
    let input = StepInput::from_settings(&settings, opts.dt, [0.0; 16], 0, 0, Vec::new());
    for _ in 0..opts.frames {
        sim.step(&input);
    }
    let spawned = u64::from(sim.spawned());
    let died = u64::from(sim.died());
    if spawned.saturating_sub(died) != u64::from(sim.alive()) {
        return Err(format!(
            "conservation failed: spawned {spawned} died {died} alive {}",
            sim.alive()
        ));
    }
    if sim.alive() > settings.live_cap() {
        return Err(format!(
            "alive {} passed the cap {}",
            sim.alive(),
            settings.live_cap()
        ));
    }
    let mut stats = ParticleStats::from_settings(&settings);
    stats.alive = sim.alive();
    stats.spawned = sim.spawned();
    stats.draw_instances = sim.draw_instances();
    println!("{}", stats.line());
    Ok(())
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut opts = Options {
        help: false,
        frames: 8,
        dt: 1.0 / 60.0,
        cli: ParticleCli::default(),
        count_set: false,
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => opts.help = true,
            "--frames" => opts.frames = parse_u32("frames", &need(&mut args, "--frames")?)?,
            "--dt" => opts.dt = parse_f32("dt", &need(&mut args, "--dt")?)?,
            other if genos_scene::particle_flag_needs_value(other).is_some() => {
                if other.split('=').next() == Some("--fx-count") {
                    opts.count_set = true;
                }
                let needs = genos_scene::particle_flag_needs_value(other).unwrap_or(false);
                let next = needs.then(|| need(&mut args, other)).transpose()?;
                genos_scene::apply_particle_arg(&mut opts.cli, other, next.as_deref())?;
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(opts)
}

fn need(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next()
        .filter(|value| !value.starts_with('-'))
        .ok_or_else(|| format!("{flag} needs a value"))
}

fn parse_u32(key: &str, value: &str) -> Result<u32, String> {
    value
        .parse::<u32>()
        .map_err(|_| format!("{key} is a whole number, got {value}"))
}

fn parse_f32(key: &str, value: &str) -> Result<f32, String> {
    value
        .parse::<f32>()
        .map_err(|_| format!("{key} is a number, got {value}"))
}
