//! GPU initial candidates for `ReSTIR` direct (ADR 0022 stage C1).
//!
//! The shader resamples point lamps the way `genos-restir-ref` does on frame 1
//! with temporal reuse off and no spatial neighbours. The live lighting pass
//! does not call this crate. `--initial` is off unless the flag or its mirror
//! is set, so a bare invocation writes nothing.
//!
//! Departures are the CPU reference's departures (`crates/restir-ref/src/lib.rs`).
//! This stage adds one: a single compute invocation walks the grid in order so
//! the PCG stream stays the documented draw order. Temporal reuse, spatial
//! reuse, and the light tree are later stages.
//!
//! Replicated state: none. The reservoirs stay in the checker.
//! Authority: the checker process.
//! Prediction / rollback: none.
//! Determinism: one PCG32 stream and the draw order in `restir-ref`.
//! Headless server: this crate is headless. A game server does not run it.

use std::path::{Path, PathBuf};

use genos_math::Vec3;
use genos_restir_ref::reservoir::Reservoir;
use genos_restir_ref::restir::{Config, History, Proposal};
use genos_restir_ref::shade::{brute, luminance, shade, Scene};
use genos_restir_ref::{encode_dump, Dump};

mod gpu;

const DEFAULT_OUT: &str = "genos-debug/restir-gpu";

/// Parsed invocation. `initial` is the stage C1 path.
#[derive(Clone, Debug)]
pub struct Options {
    pub initial: bool,
    pub seed: u64,
    pub out: PathBuf,
}

/// Reads argv. `--initial` and `--no-initial` win over `GENOS_RESTIR_INITIAL`.
/// `GENOS_RESTIR_OUT` applies only when `--out` was omitted.
///
/// # Errors
///
/// Returns a string on an unknown flag or a bad number.
pub fn parse_args(args: &[String], initial_env: Option<&str>, out_env: Option<&str>) -> Result<Options, String> {
    let mut initial = None;
    let mut seed = 1u64;
    let mut out = None;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "--initial" => initial = Some(true),
            "--no-initial" => initial = Some(false),
            "--seed" => {
                seed = parse_u64(&next_value(args, &mut index, "--seed")?, "--seed")?;
            }
            "--out" => out = Some(PathBuf::from(next_value(args, &mut index, "--out")?)),
            "--help" | "-h" => return Err("help".to_string()),
            other => return Err(format!("unknown argument {other}")),
        }
        index += 1;
    }
    let initial = match initial {
        Some(value) => value,
        None => env_on(initial_env)?,
    };
    let out = out.unwrap_or_else(|| out_env.map_or_else(|| PathBuf::from(DEFAULT_OUT), PathBuf::from));
    Ok(Options { initial, seed, out })
}

fn env_on(value: Option<&str>) -> Result<bool, String> {
    match value.map(str::trim) {
        Some("1" | "on" | "true" | "yes") => Ok(true),
        None | Some("0" | "off" | "false" | "no" | "") => Ok(false),
        Some(other) => Err(format!("GENOS_RESTIR_INITIAL {other}")),
    }
}

/// Runs initial candidates when `options.initial` is set.
///
/// # Errors
///
/// Returns a string when the device or the write fails. An off path returns `Ok(None)`.
pub fn run(options: &Options) -> Result<Option<String>, String> {
    if !options.initial {
        println!("restir initial is off");
        return Ok(None);
    }
    let scene = Scene::gate();
    let config = c1_config();
    config.validate()?;
    let (reservoirs, dispatch_ms) = initial_reservoirs(&scene, config.candidates, options.seed)?;
    let estimate = shade_all(&scene, &reservoirs);
    let brute_image = scene
        .receivers
        .iter()
        .map(|receiver| brute(receiver, &scene))
        .collect::<Vec<_>>();
    let frame_luminance = vec![estimate.iter().copied().map(luminance).collect()];
    let dump = Dump {
        scene,
        config,
        seeds: vec![options.seed],
        frames: 1,
        reservoir_seed: options.seed,
        brute: brute_image,
        estimate,
        frame_luminance,
        reservoirs,
        frame_ms: Some(dispatch_ms),
    };
    write_dump(&options.out, &dump)?;
    Ok(Some(options.out.display().to_string()))
}

fn c1_config() -> Config {
    Config {
        candidates: 8,
        spatial_neighbors: 0,
        spatial_radius: 2,
        temporal: false,
        weighting: genos_restir_ref::reservoir::Weighting::Balance,
        proposal: Proposal::Target,
        history: History::Temporal,
    }
}

fn initial_reservoirs(scene: &Scene, candidates: u32, seed: u64) -> Result<(Vec<Reservoir>, f32), String> {
    let lamp_count = u32::try_from(scene.lamps.len()).map_err(|_| "lamp count")?;
    let receiver_count = u32::try_from(scene.receivers.len()).map_err(|_| "receiver count")?;
    if lamp_count > 64 {
        return Err("C1 shader holds at most 64 lamps".to_string());
    }
    let mut words = Vec::new();
    for lamp in &scene.lamps {
        push_vec3(&mut words, lamp.position);
        push_vec3(&mut words, lamp.intensity);
    }
    let recv_base = u32::try_from(words.len()).map_err(|_| "receiver offset")?;
    for receiver in &scene.receivers {
        push_vec3(&mut words, receiver.position);
        push_vec3(&mut words, receiver.normal);
        push_vec3(&mut words, receiver.albedo);
    }
    let out_base = u32::try_from(words.len()).map_err(|_| "output offset")?;
    let out_words = usize::try_from(receiver_count).unwrap_or(0).saturating_mul(4);
    words.extend(std::iter::repeat_n(0, out_words));
    let push = gpu::Push {
        seed,
        receivers: receiver_count,
        lamps: lamp_count,
        candidates,
        proposal: 1,
        lamp_base: 0,
        recv_base,
        out_base,
    };
    let ran = gpu::dispatch(&words, push)?;
    println!(
        "device {} dispatch_ms {:.3}",
        ran.device, ran.dispatch_ms
    );
    let mut reservoirs = Vec::new();
    let base = usize::try_from(out_base).unwrap_or(0);
    for index in 0..usize::try_from(receiver_count).unwrap_or(0) {
        let slot = base.saturating_add(index.saturating_mul(4));
        let lamp = *ran.words.get(slot).ok_or("reservoir lamp")?;
        let w_sum = f32::from_bits(*ran.words.get(slot + 1).ok_or("reservoir w_sum")?);
        let sample_count = *ran.words.get(slot + 2).ok_or("reservoir m")?;
        let target = f32::from_bits(*ran.words.get(slot + 3).ok_or("reservoir target")?);
        reservoirs.push(Reservoir {
            lamp,
            w_sum,
            m: sample_count,
            target,
        });
    }
    Ok((reservoirs, ran.dispatch_ms))
}

fn push_vec3(words: &mut Vec<u32>, value: Vec3) {
    words.push(value.x.to_bits());
    words.push(value.y.to_bits());
    words.push(value.z.to_bits());
}

fn shade_all(scene: &Scene, reservoirs: &[Reservoir]) -> Vec<Vec3> {
    scene
        .receivers
        .iter()
        .zip(reservoirs)
        .map(|(receiver, reservoir)| shade(receiver, *reservoir, scene))
        .collect()
}

fn write_dump(dir: &Path, dump: &Dump) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|err| format!("create {}: {err}", dir.display()))?;
    let path = dir.join("candidate.json");
    std::fs::write(&path, encode_dump(dump)).map_err(|err| format!("write {}: {err}", path.display()))?;
    println!("wrote {}", path.display());
    Ok(())
}

fn next_value(args: &[String], index: &mut usize, flag: &str) -> Result<String, String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value"))
}

fn parse_u64(text: &str, flag: &str) -> Result<u64, String> {
    text.parse::<u64>()
        .map_err(|_| format!("{flag} wants an integer"))
}

/// Help text.
#[must_use]
pub fn help_text() -> &'static str {
    "\
restir-gpu — GPU initial ReSTIR DI candidates (stage C1)

The path is off unless --initial is set. A bare run writes nothing.
Temporal reuse and spatial reuse are not in this stage.

Usage:
  restir-gpu --initial [options]
  restir-gpu --help

Options:
  --initial             dispatch initial candidates on lavapipe
  --no-initial          stay off (wins over GENOS_RESTIR_INITIAL)
  --seed N              PCG seed (default 1, the reference reservoir seed)
  --out DIR             output directory (default ./genos-debug/restir-gpu)

Writes candidate.json in the restir-ref dump schema.
GENOS_RESTIR_INITIAL mirrors --initial and applies only when both
--initial and --no-initial were omitted. 1, on, true, and yes enable it.
GENOS_RESTIR_OUT mirrors --out and applies only when --out was omitted.
Nothing is written under $HOME.
"
}

#[cfg(test)]
mod tests {
    use super::parse_args;

    #[test]
    fn the_path_stays_off_without_the_flag_or_the_mirror() -> Result<(), String> {
        let options = parse_args(&[], None, None)?;
        assert!(!options.initial);
        Ok(())
    }

    #[test]
    fn the_flag_wins_over_the_mirror() -> Result<(), String> {
        let on = parse_args(&["--initial".to_string()], Some("0"), None)?;
        assert!(on.initial);
        let off = parse_args(&["--no-initial".to_string()], Some("1"), None)?;
        assert!(!off.initial);
        Ok(())
    }
}
