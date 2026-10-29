//! GPU `ReSTIR` direct candidates (ADR 0022 stages C1, C2, and C3).
//!
//! C1 resamples point lamps the way `genos-restir-ref` does on frame 1 with
//! temporal reuse off. C2 adds temporal reuse: the same PCG stream, the same
//! draw order, the balance-heuristic weight, and the CPU history-`M` clamp
//! (`m_clamp` times this frame's candidate count, default 20). `--m-cap`
//! injects the clamped-history fault and must fail `restir-ref diff`.
//! `--reset-after` clears history the way `Session::reset_temporal` does.
//! C3 adds one spatial pass: the same neighbour list and order, then the
//! balance weight over that list. History stays the post-temporal reservoir.
//! `--reverse-neighbours` is the order fault and must fail `restir-ref diff`.
//! The live lighting pass does not call this crate. Each path is off unless
//! the flag or its mirror is set, so a bare invocation writes nothing.
//!
//! Departures are the CPU reference's departures (`crates/restir-ref/src/lib.rs`).
//! One compute invocation walks the grid in order so the PCG stream stays the
//! documented draw order. The light tree is a later stage.
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
    /// Stage C3. Temporal reuse plus one spatial pass.
    Spatial,
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
    /// `0` leaves the absolute fault off. A positive cap is the fault injection.
    pub m_cap: u32,
    /// Multiple of this frame's candidate count. Default 20. `0` disables it.
    pub m_clamp: u32,
    pub candidates: u32,
    /// Neighbour count for [`Stage::Spatial`]. The CPU gate uses 4.
    pub spatial_neighbors: u32,
    /// Chebyshev radius. The CPU gate uses 2.
    pub spatial_radius: u32,
    /// Fault: reverse the neighbour list after the draws.
    pub reverse_neighbours: bool,
    pub proposal: Proposal,
    pub scene: ScenePick,
    pub out: PathBuf,
}

/// Reads argv. `--initial` and `--no-initial` win over `GENOS_RESTIR_INITIAL`.
/// `--temporal` and `--no-temporal` win over `GENOS_RESTIR_TEMPORAL`.
/// `--spatial` and `--no-spatial` win over `GENOS_RESTIR_SPATIAL`.
/// `GENOS_RESTIR_OUT` applies only when `--out` was omitted.
///
/// # Errors
///
/// Returns a string on an unknown flag or a bad number.
#[expect(clippy::too_many_lines, reason = "one flag table for the three stages")]
pub fn parse_args(
    args: &[String],
    initial_env: Option<&str>,
    temporal_env: Option<&str>,
    spatial_env: Option<&str>,
    out_env: Option<&str>,
) -> Result<Options, String> {
    let mut initial = None;
    let mut temporal = None;
    let mut spatial = None;
    let mut spatial_neighbors = 4u32;
    let mut spatial_radius = 2u32;
    let mut reverse_neighbours = false;
    let mut seed = 1u64;
    let mut frames = 8u32;
    let mut reset_after = None;
    let mut m_cap = 0u32;
    let mut m_clamp = genos_restir_ref::DEFAULT_M_CLAMP;
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
            "--spatial" => {
                spatial = Some(true);
                if let Some(value) = peek_value(args, index) {
                    spatial_neighbors = parse_u32(value, "--spatial")?;
                    index += 1;
                }
            }
            "--no-spatial" => spatial = Some(false),
            "--radius" => {
                spatial_radius =
                    parse_u32(&next_value(args, &mut index, "--radius")?, "--radius")?;
            }
            "--reverse-neighbours" => reverse_neighbours = true,
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
            "--m-clamp" => {
                m_clamp = parse_u32(&next_value(args, &mut index, "--m-clamp")?, "--m-clamp")?;
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
    let spatial = match spatial {
        Some(value) => value,
        None => env_on(spatial_env, "GENOS_RESTIR_SPATIAL")?,
    };
    let stage = match (initial, temporal, spatial) {
        (true, false, false) => Stage::Initial,
        (false, true, false) => Stage::Temporal,
        (false, false, true) => Stage::Spatial,
        (false, false, false) => Stage::Off,
        _ => {
            return Err(
                "pass only one of --initial, --temporal, and --spatial".to_string(),
            );
        }
    };
    let out =
        out.unwrap_or_else(|| out_env.map_or_else(|| PathBuf::from(DEFAULT_OUT), PathBuf::from));
    Ok(Options {
        stage,
        seed,
        frames,
        reset_after,
        m_cap,
        m_clamp,
        candidates,
        spatial_neighbors,
        spatial_radius,
        reverse_neighbours,
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
        Stage::Spatial => return run_spatial(options),
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
        m_clamp: genos_restir_ref::DEFAULT_M_CLAMP,
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
        m_clamp: options.m_clamp,
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
        m_clamp: config.m_clamp,
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

fn run_spatial(options: &Options) -> Result<Option<String>, String> {
    if options.frames == 0 || options.frames > 64 {
        return Err("frames must be from 1 to 64".to_string());
    }
    if options.spatial_neighbors == 0 || options.spatial_neighbors > 16 {
        return Err("spatial neighbours must be from 1 to 16".to_string());
    }
    let scene = match options.scene {
        ScenePick::OpenRow => Scene::open_row(),
        ScenePick::Gate => Scene::gate(),
    };
    let config = Config {
        candidates: options.candidates,
        spatial_neighbors: options.spatial_neighbors,
        spatial_radius: options.spatial_radius,
        temporal: true,
        weighting: genos_restir_ref::reservoir::Weighting::Balance,
        proposal: options.proposal,
        history: History::Temporal,
        m_clamp: options.m_clamp,
    };
    config.validate()?;
    let (by_frame, dispatch_ms) = spatial_reservoirs(&scene, &config, options)?;
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

fn spatial_reservoirs(
    scene: &Scene,
    config: &Config,
    options: &Options,
) -> Result<(Vec<Vec<Reservoir>>, f32), String> {
    let lamp_count = u32::try_from(scene.lamps.len()).map_err(|_| "lamp count")?;
    let receiver_count = u32::try_from(scene.receivers.len()).map_err(|_| "receiver count")?;
    let width = u32::try_from(scene.width).map_err(|_| "width")?;
    if lamp_count > 64 {
        return Err("spatial shader holds at most 64 lamps".to_string());
    }
    if receiver_count > 256 {
        return Err("spatial shader holds at most 256 receivers".to_string());
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
    let push = gpu::SpatialPush {
        temporal: gpu::TemporalPush {
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
            m_clamp: config.m_clamp,
        },
        width,
        spatial_neighbors: config.spatial_neighbors,
        spatial_radius: config.spatial_radius,
        reverse: u32::from(options.reverse_neighbours),
    };
    let ran = gpu::dispatch_spatial(&words, push)?;
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

fn peek_value(args: &[String], index: usize) -> Option<&str> {
    let value = args.get(index + 1)?.as_str();
    if value.starts_with('-') {
        None
    } else {
        Some(value)
    }
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
restir-gpu — GPU ReSTIR DI candidates (stages C1, C2, and C3)

Each path is off unless a flag is set. A bare run writes nothing.
The live direct pass does not call this binary.

Usage:
  restir-gpu --initial [options]
  restir-gpu --temporal [options]
  restir-gpu --spatial [N] [options]
  restir-gpu --help

Options:
  --initial             one frame of initial candidates (stage C1)
  --no-initial          stay off (wins over GENOS_RESTIR_INITIAL)
  --temporal            initial candidates plus temporal reuse (stage C2)
  --no-temporal         stay off (wins over GENOS_RESTIR_TEMPORAL)
  --spatial [N]         one spatial pass with N neighbours (default 4, stage C3)
  --no-spatial          stay off (wins over GENOS_RESTIR_SPATIAL)
  --radius N            Chebyshev neighbour radius (default 2)
  --reverse-neighbours  fault: reverse the neighbour list after the draws
  --frames N            temporal frames (default 8, at most 64)
  --reset-after N       clear history after N completed frames
  --m-cap N             fault: clamp history M to the absolute count N (0 off)
  --m-clamp N           clamp history M to N times this frame's candidate count
                        (default 20, matching restir-ref; 0 leaves that cap off)
  --candidates N        candidates per receiver (default 8)
  --proposal NAME       target (default) or uniform
  --scene NAME          gate (default) or open-row
  --seed N              PCG seed (default 1, the reference reservoir seed)
  --out DIR             output directory (default ./genos-debug/restir-gpu)

Writes candidate.json in the restir-ref dump schema.
History M is clamped to --m-clamp times this frame's candidate count,
the same cap as restir-ref (default 20). --m-cap is the absolute fault
that must fail restir-ref diff. --reset-after drops history the way
Session::reset_temporal does: the next frame's M is the candidate count.
GENOS_RESTIR_INITIAL mirrors --initial. GENOS_RESTIR_TEMPORAL mirrors
--temporal. GENOS_RESTIR_SPATIAL mirrors --spatial and enables 4 neighbours.
Each applies only when its flag and its --no form were omitted.
1, on, true, and yes enable a mirror. GENOS_RESTIR_OUT mirrors --out and
applies only when --out was omitted. Nothing is written under $HOME.
--reverse-neighbours keeps the PCG draws and changes the merge order, so
restir-ref diff must fail. The product grid_worst of a multi-seed spatial
run is not this stage's gate. The reservoir diff is.
"
}

#[cfg(test)]
mod tests {
    use super::{parse_args, Stage};

    #[test]
    fn the_path_stays_off_without_the_flag_or_the_mirror() -> Result<(), String> {
        let options = parse_args(&[], None, None, None, None)?;
        assert_eq!(options.stage, Stage::Off);
        assert_eq!(options.m_cap, 0);
        assert_eq!(options.m_clamp, genos_restir_ref::DEFAULT_M_CLAMP);
        Ok(())
    }

    #[test]
    fn the_flag_wins_over_the_mirror() -> Result<(), String> {
        let on = parse_args(&["--initial".to_string()], Some("0"), None, None, None)?;
        assert_eq!(on.stage, Stage::Initial);
        let off = parse_args(&["--no-initial".to_string()], Some("1"), None, None, None)?;
        assert_eq!(off.stage, Stage::Off);
        let temporal = parse_args(&["--temporal".to_string()], None, Some("0"), None, None)?;
        assert_eq!(temporal.stage, Stage::Temporal);
        let no_temporal = parse_args(&["--no-temporal".to_string()], None, Some("1"), None, None)?;
        assert_eq!(no_temporal.stage, Stage::Off);
        let spatial = parse_args(
            &["--spatial".to_string(), "4".to_string()],
            None,
            None,
            Some("0"),
            None,
        )?;
        assert_eq!(spatial.stage, Stage::Spatial);
        assert_eq!(spatial.spatial_neighbors, 4);
        let no_spatial = parse_args(&["--no-spatial".to_string()], None, None, Some("1"), None)?;
        assert_eq!(no_spatial.stage, Stage::Off);
        Ok(())
    }
}
