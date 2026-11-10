//! `genos-particles`: the stage-1 particle debug binary.
//!
//! Headless stats by default. `--screenshot` opens a window and writes one PNG.
//! See `debug/particles/README.md`.

#![expect(
    clippy::disallowed_methods,
    reason = "this binary reads GENOS_FX_* once, in main, and passes ParticleEnv down"
)]

use std::process::ExitCode;
use std::time::Duration;

use genos_mcp::command::{self, CommandSpec};
use genos_mcp::{json, Value};
use genos_render::{CpuParticles, ParticleStats, Renderer, StepInput, World};
use genos_scene::{
    Camera, Floor, LightingPreset, LightingSettings, ParticleCli, ParticleEnv, ParticlePreset,
    ParticleSettings, Scene, Vec3, SPAWN_COUNT,
};
use genos_window::{Lifecycle, Window};

const MCP_PORT: u16 = 8768;
const DT: f32 = 1.0 / 60.0;
const MCP_ROOM: &str = include_str!("../../../examples/stress/scenes/room.rhai");

const USAGE: &str = "\
genos-particles: stage-1 particle debug binary

  --list                 print every scene and exit
  --scene NAME           scene to run (default burst)
  --frames N             steps (default: the scene's own)
  --dt N                 seconds per step (default 1/60)
  --replay               run twice from step 0; fail on the first hash mismatch
  --checkpoint K         snapshot at step K, restore, rerun; fail on a mismatch
  --screenshot           draw the scene and write <out>/particles/<scene>.png
  --out DIR              screenshot directory root (default ./genos-debug)
  --mcp                  serve MCP on 127.0.0.1:8768 until particle_quit
  --mcp-port N           serve on this port instead
  --fx-preset NAME       low, medium, or high (GENOS_FX_PRESET)
  --fx-max N             buffer budget (GENOS_FX_MAX)
  --fx-count N           live cap (GENOS_FX_COUNT)
  --fx-freeze            hold positions (GENOS_FX_FREEZE)
  --fx-spawn X,Y,Z       origin, and turn the emitter on
  -h, --help             this text

A plain run prints one fx.* line from the CPU mirror. sim_ms and draw_ms are 0
here. The GPU timestamps are lavapipe, from particles_gpu, not a desktop score.
";

struct SceneInfo {
    name: &'static str,
    summary: &'static str,
    frames: u32,
}

const SCENES: &[SceneInfo] = &[
    SceneInfo {
        name: "burst",
        summary: "64 live, rate 8, life 1",
        frames: 8,
    },
    SceneInfo {
        name: "capped",
        summary: "budget 4, high rate, alive stays at 4",
        frames: 6,
    },
    SceneInfo {
        name: "frozen",
        summary: "freeze holds emit and motion",
        frames: 8,
    },
    SceneInfo {
        name: "idle",
        summary: "count 0 does not dispatch",
        frames: 4,
    },
];

struct Options {
    list: bool,
    scene: String,
    frames: Option<u32>,
    dt: f32,
    replay: bool,
    checkpoint: Option<u32>,
    screenshot: bool,
    out: String,
    mcp_port: Option<u16>,
    cli: ParticleCli,
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
    let Some(opts) = parse(std::env::args().skip(1))? else {
        print!("{USAGE}");
        return Ok(());
    };
    if opts.list {
        println!("{}", picker());
        return Ok(());
    }
    let info = scene(&opts.scene)?;
    let frames = opts.frames.unwrap_or(info.frames);
    let settings = tuned(&opts, info)?;
    if let Some(port) = opts.mcp_port {
        return serve(info, settings, opts.dt, port);
    }
    if opts.replay {
        let hash = replay(&settings, opts.dt, frames)?;
        println!(
            "replay=match scene={} steps={frames} hash={hash:016x}",
            info.name
        );
        return Ok(());
    }
    if let Some(at) = opts.checkpoint {
        let hash = checkpoint(&settings, opts.dt, at, frames)?;
        println!(
            "checkpoint=match scene={} at={at} steps={frames} hash={hash:016x}",
            info.name
        );
        return Ok(());
    }
    let sim = simulate(&settings, opts.dt, frames)?;
    if opts.screenshot {
        let path = std::path::PathBuf::from(&opts.out)
            .join("particles")
            .join(format!("{}.png", info.name));
        write_shot(&settings, opts.dt, frames, &path)?;
        eprintln!("screenshot {}", path.display());
    }
    println!("{}", stats_line(&settings, &sim));
    Ok(())
}

fn tuned(opts: &Options, info: &SceneInfo) -> Result<ParticleSettings, String> {
    let base = scene_settings(info.name);
    Ok(base.resolve(&opts.cli, &env_from_process(&opts.cli)?))
}

fn scene(name: &str) -> Result<&'static SceneInfo, String> {
    SCENES
        .iter()
        .find(|scene| scene.name == name)
        .ok_or_else(|| format!("unknown scene {name}; --list shows them"))
}

fn picker() -> String {
    SCENES
        .iter()
        .map(|scene| format!("{:<16} {:>4} steps  {}", scene.name, scene.frames, scene.summary))
        .collect::<Vec<_>>()
        .join("\n")
}

fn scene_settings(name: &str) -> ParticleSettings {
    let mut settings = ParticleSettings::preset(ParticlePreset::Low);
    settings.enabled = true;
    settings.count = SPAWN_COUNT;
    settings.rate = 8.0;
    settings.life = 1.0;
    match name {
        "capped" => {
            settings.max_particles = 4;
            settings.count = 4;
            settings.rate = 240.0;
            settings.life = 5.0;
            settings.preset = ParticlePreset::Custom;
        }
        "frozen" => settings.freeze = true,
        "idle" => {
            settings.enabled = false;
            settings.count = 0;
        }
        _ => {}
    }
    settings
}

fn simulate(settings: &ParticleSettings, dt: f32, frames: u32) -> Result<CpuParticles, String> {
    let mut sim = CpuParticles::new();
    let input = StepInput::from_settings(settings, dt, [0.0; 16], 0, 0, Vec::new());
    for _ in 0..frames {
        sim.step(&input);
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
    }
    Ok(sim)
}

fn replay(settings: &ParticleSettings, dt: f32, frames: u32) -> Result<u64, String> {
    let mut first = CpuParticles::new();
    let mut second = CpuParticles::new();
    let input = StepInput::from_settings(settings, dt, [0.0; 16], 0, 0, Vec::new());
    for step in 0..frames {
        first.step(&input);
        second.step(&input);
        if first.fingerprint() != second.fingerprint() {
            return Err(format!("replay=diverged step={}", step + 1));
        }
    }
    Ok(first.fingerprint())
}

fn checkpoint(settings: &ParticleSettings, dt: f32, at: u32, frames: u32) -> Result<u64, String> {
    let input = StepInput::from_settings(settings, dt, [0.0; 16], 0, 0, Vec::new());
    let mut original = CpuParticles::new();
    for _ in 0..at.min(frames) {
        original.step(&input);
    }
    let saved = original.clone();
    for _ in at.min(frames)..frames {
        original.step(&input);
    }
    let mut restored = saved;
    let mut fresh = CpuParticles::new();
    for _ in 0..at.min(frames) {
        fresh.step(&input);
    }
    if fresh.fingerprint() != restored.fingerprint() {
        return Err(format!("checkpoint=diverged step={at}"));
    }
    for _ in at.min(frames)..frames {
        restored.step(&input);
        fresh.step(&input);
    }
    if restored.fingerprint() != original.fingerprint() || fresh.fingerprint() != original.fingerprint()
    {
        return Err("checkpoint=diverged after restore".into());
    }
    Ok(original.fingerprint())
}

fn stats_line(settings: &ParticleSettings, sim: &CpuParticles) -> String {
    let mut stats = ParticleStats::from_settings(settings);
    stats.alive = if settings.dispatches() { sim.alive() } else { 0 };
    stats.spawned = if settings.dispatches() { sim.spawned() } else { 0 };
    stats.draw_instances = if settings.dispatches() {
        sim.draw_instances()
    } else {
        0
    };
    stats.line()
}

fn write_shot(
    settings: &ParticleSettings,
    dt: f32,
    frames: u32,
    path: &std::path::Path,
) -> Result<(), String> {
    let mut window = Window::open_proof(128, 128)?;
    let frame = window.pump();
    let (renderer, permit) = Renderer::open(
        window.display,
        window.surface,
        frame.width,
        frame.height,
    )?;
    let mut life = Lifecycle::new(window, renderer, permit);
    let world = proof_world(settings);
    let camera = Camera::new(0.0, 2.0, 0.0);
    let mut pixels = None;
    let frames = frames.max(1);
    for index in 0..frames {
        let _ = life.window.pump();
        life.device.set_particle_dt(dt);
        let read = index + 1 == frames;
        pixels = life.device.draw(&world, &camera, read)?;
    }
    let pixels = pixels.ok_or_else(|| "screenshot draw returned no picture".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    genos_debug::png::write_png(path, life.device.width(), life.device.height(), &pixels)
}

fn proof_world(particles: &ParticleSettings) -> World {
    World::from_scene(Scene {
        floor: Floor {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 4.0,
            half_z: 4.0,
            color: [0.2, 0.2, 0.2],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights: Vec::new(),
        ceiling: None,
        sky: None,
        lighting: LightingSettings::preset(LightingPreset::Potato),
        debug: genos_scene::DebugSettings::default(),
        fps_counter: genos_scene::FpsCounterSettings::default(),
        event_loop: genos_scene::EventLoopSettings::default(),
        reflections: genos_scene::ReflectionSettings::default(),
        hdr: genos_scene::HdrSettings::legacy_bytes(),
        fog: genos_scene::FogSettings::default(),
        menu: genos_scene::MenuSettings::default(),
        particles: particles.clone(),
    })
}

fn env_from_process(cli: &ParticleCli) -> Result<ParticleEnv, String> {
    Ok(ParticleEnv {
        preset: mirror_preset(cli.preset.is_some())?,
        max_particles: mirror_u32(cli.max_particles.is_some(), "GENOS_FX_MAX")?,
        count: mirror_u32(cli.count.is_some(), "GENOS_FX_COUNT")?,
        freeze: mirror_switch(cli.freeze.is_some(), "GENOS_FX_FREEZE")?,
    })
}

fn mirror_preset(argument_set: bool) -> Result<Option<ParticlePreset>, String> {
    if argument_set {
        return Ok(None);
    }
    match std::env::var("GENOS_FX_PRESET") {
        Err(_) => Ok(None),
        Ok(value) if value.trim().is_empty() => Ok(None),
        Ok(value) => ParticlePreset::parse(value.trim())
            .map(Some)
            .ok_or_else(|| format!("GENOS_FX_PRESET is low, medium, or high, got {value}")),
    }
}

fn mirror_u32(argument_set: bool, name: &str) -> Result<Option<u32>, String> {
    if argument_set {
        return Ok(None);
    }
    match std::env::var(name) {
        Err(_) => Ok(None),
        Ok(value) if value.trim().is_empty() => Ok(None),
        Ok(value) => value
            .trim()
            .parse::<u32>()
            .map(Some)
            .map_err(|_| format!("{name} is a whole number, got {value}")),
    }
}

fn mirror_switch(argument_set: bool, name: &str) -> Result<Option<bool>, String> {
    if argument_set {
        return Ok(None);
    }
    match std::env::var(name) {
        Err(_) => Ok(None),
        Ok(value) if value.trim().is_empty() => Ok(None),
        Ok(value) => genos_scene::parse_fog_switch(value.trim())
            .map(Some)
            .map_err(|err| format!("{name}: {err}")),
    }
}

fn parse(args: impl Iterator<Item = String>) -> Result<Option<Options>, String> {
    let mut opts = Options {
        list: false,
        scene: "burst".into(),
        frames: None,
        dt: DT,
        replay: false,
        checkpoint: None,
        screenshot: false,
        out: "genos-debug".into(),
        mcp_port: None,
        cli: ParticleCli::default(),
    };
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--list" => opts.list = true,
            "--replay" => opts.replay = true,
            "--screenshot" => opts.screenshot = true,
            "--scene" => opts.scene = need(&mut args, "--scene")?,
            "--frames" => opts.frames = Some(parse_u32("frames", &need(&mut args, "--frames")?)?),
            "--dt" => opts.dt = parse_f32("dt", &need(&mut args, "--dt")?)?,
            "--checkpoint" => {
                opts.checkpoint = Some(parse_u32("checkpoint", &need(&mut args, "--checkpoint")?)?);
            }
            "--out" => opts.out = need(&mut args, "--out")?,
            "--mcp" => opts.mcp_port = Some(MCP_PORT),
            "--mcp-port" => {
                let port = parse_u32("mcp-port", &need(&mut args, "--mcp-port")?)?;
                let port = u16::try_from(port).map_err(|_| "--mcp-port needs 1 to 65535".to_string())?;
                if port == 0 {
                    return Err("--mcp-port needs 1 to 65535".into());
                }
                opts.mcp_port = Some(port);
            }
            other if genos_scene::particle_flag_needs_value(other).is_some() => {
                let needs = genos_scene::particle_flag_needs_value(other).unwrap_or(false);
                let inline = other.split_once('=').map(|(_, value)| value.to_string());
                let next = if inline.is_some() {
                    None
                } else if needs {
                    Some(need(&mut args, other)?)
                } else {
                    None
                };
                genos_scene::apply_particle_arg(
                    &mut opts.cli,
                    other,
                    inline.as_deref().or(next.as_deref()),
                )?;
            }
            other => return Err(format!("unknown argument {other}\n\n{USAGE}")),
        }
    }
    Ok(Some(opts))
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

fn spec(name: &str, description: &str) -> CommandSpec {
    CommandSpec {
        name: name.into(),
        description: description.into(),
        schema: json::object([("type", json::string("object"))]),
    }
}

fn specs() -> Vec<CommandSpec> {
    vec![
        spec("particle_scenes", "List the particle debug scenes."),
        spec("particle_load", "Load a scene at step 0."),
        spec("particle_reset", "Reload the current scene at step 0."),
        spec("particle_step", "Advance N steps and return the fx.* lines."),
        spec("particle_stats", "The last step's fx.* line."),
        spec("particle_replay", "Rerun from step 0 and compare fingerprints."),
        spec("particle_checkpoint", "Snapshot, restore, and compare fingerprints."),
        spec("fx_stats", "The last step's fx.* line."),
        spec("fx_list", "The one stage-1 emitter, named stage1."),
        spec("particle_quit", "Stop serving and exit."),
    ]
}

struct Session {
    info: &'static SceneInfo,
    settings: ParticleSettings,
    dt: f32,
    sim: CpuParticles,
    frames: u32,
    last: String,
}

fn serve(
    info: &'static SceneInfo,
    settings: ParticleSettings,
    dt: f32,
    port: u16,
) -> Result<(), String> {
    command::register(specs());
    let host = genos_mcp::Host::new(genos_scene::load_str(MCP_ROOM)?, Camera::new(0.0, 0.0, 0.0));
    let _server = genos_mcp::Server::start_on(host, port)?;
    eprintln!("mcp http://127.0.0.1:{port}/mcp scene={}", info.name);
    let mut session = Session {
        info,
        settings,
        dt,
        sim: CpuParticles::new(),
        frames: 0,
        last: String::new(),
    };
    loop {
        for pending in command::take_wait(Duration::from_secs(1)) {
            let quit = pending.name == "particle_quit";
            let result = handle(&mut session, &pending.name, &pending.args);
            command::reply(pending.id, result);
            if quit {
                return Ok(());
            }
        }
    }
}

fn handle(session: &mut Session, name: &str, args: &Value) -> Result<Value, String> {
    let steps = args
        .get("steps")
        .and_then(Value::as_u64)
        .map_or(1, |value| u32::try_from(value).unwrap_or(1));
    match name {
        "particle_scenes" => Ok(json::array(
            SCENES
                .iter()
                .map(|scene| {
                    json::object([
                        ("name", json::string(scene.name)),
                        ("summary", json::string(scene.summary)),
                        ("steps", json::int(i64::from(scene.frames))),
                    ])
                })
                .collect(),
        )),
        "particle_load" => {
            let name = args
                .get("scene")
                .and_then(Value::as_str)
                .ok_or("scene is required")?;
            session.info = scene(name)?;
            session.settings = scene_settings(name);
            session.sim = CpuParticles::new();
            session.frames = 0;
            session.last.clear();
            Ok(json::string(format!("loaded {name}")))
        }
        "particle_reset" => {
            session.sim = CpuParticles::new();
            session.frames = 0;
            session.last.clear();
            Ok(json::string(format!("reset {}", session.info.name)))
        }
        "particle_step" => {
            let mut lines = Vec::new();
            let input = StepInput::from_settings(
                &session.settings,
                session.dt,
                [0.0; 16],
                0,
                0,
                Vec::new(),
            );
            for _ in 0..steps {
                session.sim.step(&input);
                session.frames = session.frames.saturating_add(1);
                session.last = stats_line(&session.settings, &session.sim);
                lines.push(session.last.clone());
            }
            Ok(json::array(lines.into_iter().map(json::string).collect()))
        }
        "particle_stats" | "fx_stats" => Ok(json::string(session.last.clone())),
        "fx_list" => Ok(json::array(vec![json::string("stage1")])),
        "particle_replay" => {
            let hash = replay(&session.settings, session.dt, steps)?;
            Ok(json::string(format!("replay=match hash={hash:016x}")))
        }
        "particle_checkpoint" => {
            let at = args
                .get("at")
                .and_then(Value::as_u64)
                .map_or(1, |value| u32::try_from(value).unwrap_or(1));
            let hash = checkpoint(&session.settings, session.dt, at, steps.max(at))?;
            Ok(json::string(format!("checkpoint=match hash={hash:016x}")))
        }
        "particle_quit" => Ok(json::string("bye")),
        other => Err(format!("unknown command {other}")),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test aborts when a fixture fails")]
mod tests {
    use super::*;

    #[test]
    fn parses_list_replay_and_checkpoint() {
        let opts = parse(
            ["--scene", "capped", "--replay", "--checkpoint", "2"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap()
        .unwrap();
        assert_eq!(opts.scene, "capped");
        assert!(opts.replay);
        assert_eq!(opts.checkpoint, Some(2));
        assert!(parse(["--nope"].into_iter().map(str::to_string)).is_err());
    }

    #[test]
    fn replay_and_checkpoint_match_on_the_burst() {
        let settings = scene_settings("burst");
        assert!(replay(&settings, DT, 8).is_ok());
        assert!(checkpoint(&settings, DT, 3, 8).is_ok());
        let idle = simulate(&scene_settings("idle"), DT, 4).unwrap();
        assert_eq!(idle.alive(), 0);
        let capped = simulate(&scene_settings("capped"), DT, 6).unwrap();
        assert!(capped.alive() <= 4);
    }

    #[test]
    fn spawn_flag_sets_a_count_when_the_scene_is_idle() {
        let mut opts = parse(["--scene", "idle", "--fx-spawn", "1,2,3"].into_iter().map(str::to_string))
            .unwrap()
            .unwrap();
        opts.cli.count = None;
        let settings = tuned(&opts, scene("idle").unwrap()).unwrap();
        assert!(settings.dispatches());
        assert_eq!(settings.count, SPAWN_COUNT);
    }
}
