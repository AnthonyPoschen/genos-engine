//! GPU `ReSTIR` direct candidates (ADR 0022 stages C1 and C2).
//!
//! C1 resamples point lamps the way `genos-restir-ref` does on frame 1 with
//! temporal reuse off. C2 adds temporal reuse: the same PCG stream, the same
//! draw order, the balance-heuristic weight, and no `M` clamp. `--m-cap`
//! injects the clamped-history fault and must fail `restir-ref diff`.
//! `--reset-after` clears history the way `Session::reset_temporal` does.
//! The live lighting pass does not call this crate. Both paths are off unless
//! the flag or its mirror is set, so a bare invocation writes nothing.
//!
//! Departures are the CPU reference's departures (`crates/restir-ref/src/lib.rs`).
//! One compute invocation walks the grid in order so the PCG stream stays the
//! documented draw order. Spatial reuse and the light tree are later stages.
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

/// Which GPU stage a run dispatches. Off writes nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Bare invocation. The live direct pass is unchanged.
    Off,
    /// Stage C1. One frame, no temporal merge.
    Initial,
    /// Stage C2. Initial candidates plus temporal reuse.
    Temporal,
}

/// Built-in receiver grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScenePick {
    /// Unoccluded 8×8 gate.
    Gate,
    /// One receiver, eight coloured lamps.
    OpenRow,
}

/// Parsed invocation. `stage` selects C1 or C2.
#[derive(Clone, Debug)]
pub struct Options {
    pub stage: Stage,
    pub seed: u64,
    pub frames: u32,
    /// Completed frames before history is cleared. `None` never clears.
    pub reset_after: Option<u32>,
    /// `0` leaves `M` unclamped. A positive cap is the fault injection.
    pub m_cap: u32,
    pub candidates: u32,
    pub proposal: Proposal,
    pub scene: ScenePick,
    pub out: PathBuf,
}

/// Reads argv. `--initial` and `--no-initial` win over `GENOS_RESTIR_INITIAL`.
/// `--temporal` and `--no-temporal` win over `GENOS_RESTIR_TEMPORAL`.
/// `GENOS_RESTIR_OUT` applies only when `--out` was omitted.
///
/// # Errors
///
/// Returns a string on an unknown flag or a bad number.
pub fn parse_args(
    args: &[String],
    initial_env: Option<&str>,
    temporal_env: Option<&str>,
    out_env: Option<&str>,
) -> Result<Options, String> {
    let mut initial = None;
    let mut temporal = None;
    let mut seed = 1u64;
    let mut frames = 8u32;
    let mut reset_after = None;
    let mut m_cap = 0u32;
    let mut candidates = 8u32;
    let mut uniform = false;
    let mut open_row = false;
    let mut out = None;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "--initial" => initial = Some(true),
            "--no-initial" => initial = Some(false),
            "--temporal" => temporal = Some(true),
            "--no-temporal" => temporal = Some(false),
            "--seed" => {
                seed = parse_u64(&next_value(args, &mut index, "--seed")?, "--seed")?;
            }
            "--frames" => {
                frames = parse_u32(&next_value(args, &mut index, "--frames")?, "--frames")?;
            }
            "--reset-after" => {
                reset_after = Some(parse_u32(
                    &next_value(args, &mut index, "--reset-after")?,
                    "--reset-after",
                )?);
            }
            "--m-cap" => {
                m_cap = parse_u32(&next_value(args, &mut index, "--m-cap")?, "--m-cap")?;
            }
            "--candidates" => {
                candidates = parse_u32(
                    &next_value(args, &mut index, "--candidates")?,
                    "--candidates",
                )?;
            }
            "--proposal" => {
                uniform = match next_value(args, &mut index, "--proposal")?.as_str() {
                    "target" => false,
                    "uniform" => true,
                    other => return Err(format!("--proposal {other}")),
                };
            }
            "--scene" => {
                open_row = match next_value(args, &mut index, "--scene")?.as_str() {
                    "gate" => false,
                    "open-row" => true,
                    other => return Err(format!("--scene {other}")),
                };
            }
            "--out" => out = Some(PathBuf::from(next_value(args, &mut index, "--out")?)),
            "--help" | "-h" => return Err("help".to_string()),
            other => return Err(format!("unknown argument {other}")),
        }
        index += 1;
    }
    let initial = match initial {
        Some(value) => value,
        None => env_on(initial_env, "GENOS_RESTIR_INITIAL")?,
    };
    let temporal = match temporal {
        Some(value) => value,
        None => env_on(temporal_env, "GENOS_RESTIR_TEMPORAL")?,
    };
    let stage = match (initial, temporal) {
        (true, true) => return Err("pass only one of --initial and --temporal".to_string()),
        (true, false) => Stage::Initial,
        (false, true) => Stage::Temporal,
        (false, false) => Stage::Off,
    };
    let out =
        out.unwrap_or_else(|| out_env.map_or_else(|| PathBuf::from(DEFAULT_OUT), PathBuf::from));
    Ok(Options {
        stage,
        seed,
        frames,
        reset_after,
        m_cap,
        candidates,
        proposal: if uniform {
            Proposal::Uniform
        } else {
            Proposal::Target
        },
        scene: if open_row {
            ScenePick::OpenRow
        } else {
            ScenePick::Gate
        },
        out,
    })
}

fn env_on(value: Option<&str>, name: &str) -> Result<bool, String> {
    match value.map(str::trim) {
        Some("1" | "on" | "true" | "yes") => Ok(true),
        None | Some("0" | "off" | "false" | "no" | "") => Ok(false),
        Some(other) => Err(format!("{name} {other}")),
    }
}

/// Runs initial candidates when `options.initial` is set.
///
/// # Errors
///
/// Returns a string when the device or the write fails. An off path returns `Ok(None)`.
pub fn run(options: &Options) -> Result<Option<String>, String> {
    match options.stage {
        Stage::Off => {
            println!("restir initial is off");
            return Ok(None);
        }
        Stage::Temporal => return run_temporal(options),
        Stage::Initial => {}
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

fn initial_reservoirs(
    scene: &Scene,
    candidates: u32,
    seed: u64,
) -> Result<(Vec<Reservoir>, f32), String> {
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
    let out_words = usize::try_from(receiver_count)
        .unwrap_or(0)
        .saturating_mul(4);
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
    println!("device {} dispatch_ms {:.3}", ran.device, ran.dispatch_ms);
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

fn run_temporal(options: &Options) -> Result<Option<String>, String> {
    if options.frames == 0 || options.frames > 64 {
        return Err("frames must be from 1 to 64".to_string());
    }
    let scene = match options.scene {
        ScenePick::OpenRow => Scene::open_row(),
        ScenePick::Gate => Scene::gate(),
    };
    let config = Config {
        candidates: options.candidates,
        spatial_neighbors: 0,
        spatial_radius: 2,
        temporal: true,
        weighting: genos_restir_ref::reservoir::Weighting::Balance,
        proposal: options.proposal,
        history: History::Temporal,
    };
    config.validate()?;
    let (by_frame, dispatch_ms) = temporal_reservoirs(&scene, &config, options)?;
    let shaded: Vec<Vec<Vec3>> = by_frame
        .iter()
        .map(|frame| shade_all(&scene, frame))
        .collect();
    let estimate = shaded.last().cloned().unwrap_or_default();
    let reservoirs = by_frame.last().cloned().unwrap_or_default();
    let brute_image = scene
        .receivers
        .iter()
        .map(|receiver| brute(receiver, &scene))
        .collect::<Vec<_>>();
    let frame_luminance = shaded
        .iter()
        .map(|frame| frame.iter().copied().map(luminance).collect())
        .collect();
    let dump = Dump {
        scene,
        config,
        seeds: vec![options.seed],
        frames: options.frames,
        reservoir_seed: options.seed,
        brute: brute_image,
        estimate,
        frame_luminance,
        reservoirs,
        frame_ms: Some(dispatch_ms),
    };
    write_dump(&options.out, &dump)?;
    if let Some(sample) = dump.reservoirs.first() {
        println!("history_m {}", sample.m);
    }
    Ok(Some(options.out.display().to_string()))
}

fn temporal_reservoirs(
    scene: &Scene,
    config: &Config,
    options: &Options,
) -> Result<(Vec<Vec<Reservoir>>, f32), String> {
    let lamp_count = u32::try_from(scene.lamps.len()).map_err(|_| "lamp count")?;
    let receiver_count = u32::try_from(scene.receivers.len()).map_err(|_| "receiver count")?;
    if lamp_count > 64 {
        return Err("temporal shader holds at most 64 lamps".to_string());
    }
    if receiver_count > 256 {
        return Err("temporal shader holds at most 256 receivers".to_string());
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
    let receivers = usize::try_from(receiver_count).unwrap_or(0);
    let frames = usize::try_from(options.frames).unwrap_or(0);
    let out_words = receivers.saturating_mul(4).saturating_mul(frames);
    words.extend(std::iter::repeat_n(0, out_words));
    let push = gpu::TemporalPush {
        base: gpu::Push {
            seed: options.seed,
            receivers: receiver_count,
            lamps: lamp_count,
            candidates: config.candidates,
            proposal: u32::from(options.proposal == Proposal::Target),
            lamp_base: 0,
            recv_base,
            out_base,
        },
        frames: options.frames,
        reset_after: options.reset_after.unwrap_or(u32::MAX),
        m_cap: options.m_cap,
    };
    let ran = gpu::dispatch_temporal(&words, push)?;
    println!("device {} dispatch_ms {:.3}", ran.device, ran.dispatch_ms);
    let base = usize::try_from(out_base).unwrap_or(0);
    let mut by_frame = Vec::with_capacity(frames);
    for frame in 0..frames {
        let mut reservoirs = Vec::with_capacity(receivers);
        for index in 0..receivers {
            let slot = base
                .saturating_add(frame.saturating_mul(receivers).saturating_mul(4))
                .saturating_add(index.saturating_mul(4));
            reservoirs.push(read_reservoir(&ran.words, slot)?);
        }
        by_frame.push(reservoirs);
    }
    Ok((by_frame, ran.dispatch_ms))
}

fn read_reservoir(words: &[u32], slot: usize) -> Result<Reservoir, String> {
    let lamp = *words.get(slot).ok_or("reservoir lamp")?;
    let w_sum = f32::from_bits(*words.get(slot + 1).ok_or("reservoir w_sum")?);
    let sample_count = *words.get(slot + 2).ok_or("reservoir m")?;
    let target = f32::from_bits(*words.get(slot + 3).ok_or("reservoir target")?);
    Ok(Reservoir {
        lamp,
        w_sum,
        m: sample_count,
        target,
    })
}

fn write_dump(dir: &Path, dump: &Dump) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|err| format!("create {}: {err}", dir.display()))?;
    let path = dir.join("candidate.json");
    std::fs::write(&path, encode_dump(dump))
        .map_err(|err| format!("write {}: {err}", path.display()))?;
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

fn parse_u32(text: &str, flag: &str) -> Result<u32, String> {
    text.parse::<u32>()
        .map_err(|_| format!("{flag} wants an integer"))
}

/// Help text.
#[must_use]
pub fn help_text() -> &'static str {
    "\
restir-gpu — GPU ReSTIR DI candidates (stages C1 and C2)

Both paths are off unless a flag is set. A bare run writes nothing.
The live direct pass does not call this binary. Spatial reuse is not here.

Usage:
  restir-gpu --initial [options]
  restir-gpu --temporal [options]
  restir-gpu --help

Options:
  --initial             one frame of initial candidates (stage C1)
  --no-initial          stay off (wins over GENOS_RESTIR_INITIAL)
  --temporal            initial candidates plus temporal reuse (stage C2)
  --no-temporal         stay off (wins over GENOS_RESTIR_TEMPORAL)
  --frames N            temporal frames (default 8, at most 64)
  --reset-after N       clear history after N completed frames
  --m-cap N             fault: clamp history M to N (0 leaves M unclamped)
  --candidates N        candidates per receiver (default 8)
  --proposal NAME       target (default) or uniform
  --scene NAME          gate (default) or open-row
  --seed N              PCG seed (default 1, the reference reservoir seed)
  --out DIR             output directory (default ./genos-debug/restir-gpu)

Writes candidate.json in the restir-ref dump schema.
M is not clamped. That matches the CPU reference. --m-cap is the fault
that must fail restir-ref diff. --reset-after drops history the way
Session::reset_temporal does: the next frame's M is the candidate count.
GENOS_RESTIR_INITIAL mirrors --initial. GENOS_RESTIR_TEMPORAL mirrors
--temporal. Each applies only when its flag and its --no form were omitted.
1, on, true, and yes enable a mirror. GENOS_RESTIR_OUT mirrors --out and
applies only when --out was omitted. Nothing is written under $HOME.
"
}

#[cfg(test)]
mod tests {
    use super::{parse_args, Stage};

    #[test]
    fn the_path_stays_off_without_the_flag_or_the_mirror() -> Result<(), String> {
        let options = parse_args(&[], None, None, None)?;
        assert_eq!(options.stage, Stage::Off);
        assert_eq!(options.m_cap, 0);
        Ok(())
    }

    #[test]
    fn the_flag_wins_over_the_mirror() -> Result<(), String> {
        let on = parse_args(&["--initial".to_string()], Some("0"), None, None)?;
        assert_eq!(on.stage, Stage::Initial);
        let off = parse_args(&["--no-initial".to_string()], Some("1"), None, None)?;
        assert_eq!(off.stage, Stage::Off);
        let temporal = parse_args(&["--temporal".to_string()], None, Some("0"), None)?;
        assert_eq!(temporal.stage, Stage::Temporal);
        let no_temporal = parse_args(&["--no-temporal".to_string()], None, Some("1"), None)?;
        assert_eq!(no_temporal.stage, Stage::Off);
        Ok(())
    }
}
