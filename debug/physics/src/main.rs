//! `genos-physics`: the physics debug binary. Headless; stats only, no draw.
//! See `debug/physics/README.md`.

use std::process::ExitCode;
use std::time::Duration;

use genos_mcp::command::{self, CommandSpec};
use genos_mcp::{json, Value};
use genos_physics_debug::{checkpoint, find, replay, SceneInfo, Session, SCENES};

/// Default MCP port. Lighting uses 8766; the editor uses 8765.
const MCP_PORT: u16 = 8767;

/// `genos-mcp` serves a lighting scene resource next to the commands. Physics
/// has none, so it serves the shared room; the physics commands ignore it.
const MCP_ROOM: &str = include_str!("../../../examples/stress/scenes/room.rhai");

const USAGE: &str = "genos-physics: headless physics debug binary (stats only, no draw)

  --list                 scene picker: print every scene and exit
  --scene NAME           scene to run (default ship_lanes)
  --steps N              fixed 1/60 s steps (default: the scene's own)
  --replay               run twice from step 0; fail on the first hash mismatch
  --checkpoint K         snapshot at step K, restore, rerun; fail on a mismatch
  --mcp-port N           serve MCP and POST /cmd/<name> on 127.0.0.1:N
                         (--mcp alone uses 8767); runs until physics_quit
  -h, --help             this text

A plain run prints one JSON line per step: step, step_us, bodies, ccd_applied,
toi_failed, toi_deferred, pair_slices, dynamic_pair_impulses, linear_energy,
rotational_energy, max_speed, min_y, a bitwise state hash, contact_points
and sleeping_bodies.";

struct Options {
    list: bool,
    scene: String,
    steps: Option<u32>,
    replay: bool,
    checkpoint: Option<u32>,
    mcp_port: Option<u16>,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Option<Options>, String> {
    let mut opts = Options {
        list: false,
        scene: "ship_lanes".into(),
        steps: None,
        replay: false,
        checkpoint: None,
        mcp_port: None,
    };
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--list" => opts.list = true,
            "--replay" => opts.replay = true,
            "--scene" => opts.scene = value("--scene")?,
            "--steps" => opts.steps = Some(number(&value("--steps")?)?),
            "--checkpoint" => opts.checkpoint = Some(number(&value("--checkpoint")?)?),
            "--mcp" => opts.mcp_port = Some(MCP_PORT),
            "--mcp-port" => {
                let port: u16 = number(&value("--mcp-port")?)?;
                if port == 0 {
                    return Err("--mcp-port needs 1 to 65535".into());
                }
                opts.mcp_port = Some(port);
            }
            other => return Err(format!("unknown argument {other}\n\n{USAGE}")),
        }
    }
    Ok(Some(opts))
}

fn number<T: std::str::FromStr>(text: &str) -> Result<T, String> {
    text.parse().map_err(|_| format!("not a number: {text}"))
}

fn scene(name: &str) -> Result<&'static SceneInfo, String> {
    find(name).ok_or_else(|| format!("unknown scene {name}; --list shows them"))
}

fn picker() -> String {
    SCENES
        .iter()
        .map(|s| format!("{:<16} {:>4} steps  {}", s.name, s.steps, s.summary))
        .collect::<Vec<_>>()
        .join("\n")
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let Some(opts) = parse_args(std::env::args().skip(1))? else {
        println!("{USAGE}");
        return Ok(());
    };
    if opts.list {
        println!("{}", picker());
        return Ok(());
    }
    let info = scene(&opts.scene)?;
    let steps = opts.steps.unwrap_or(info.steps);
    if let Some(port) = opts.mcp_port {
        return serve(info, port);
    }
    if opts.replay {
        let hash = replay(info, steps)
            .map_err(|at| format!("replay=diverged scene={} step={at}", info.name))?;
        println!(
            "replay=match scene={} steps={steps} hash={hash:016x}",
            info.name
        );
        return Ok(());
    }
    if let Some(at) = opts.checkpoint {
        let hash = checkpoint(info, at, steps)
            .map_err(|step| format!("checkpoint=diverged scene={} step={step}", info.name))?;
        println!(
            "checkpoint=match scene={} at={at} steps={steps} hash={hash:016x}",
            info.name
        );
        return Ok(());
    }
    let mut session = Session::load(info);
    for _ in 0..steps {
        println!("{}", session.step().json(info.name));
    }
    Ok(())
}

fn spec(name: &str, description: &str, schema: Value) -> CommandSpec {
    CommandSpec {
        name: name.into(),
        description: description.into(),
        schema,
    }
}

fn no_args() -> Value {
    json::object([("type", json::string("object"))])
}

fn specs() -> Vec<CommandSpec> {
    let scene_arg = json::object([
        ("type", json::string("object")),
        (
            "properties",
            json::object([("scene", json::object([("type", json::string("string"))]))]),
        ),
    ]);
    let steps_arg = json::object([
        ("type", json::string("object")),
        (
            "properties",
            json::object([("steps", json::object([("type", json::string("integer"))]))]),
        ),
    ]);
    let index_arg = json::object([
        ("type", json::string("object")),
        (
            "properties",
            json::object([("index", json::object([("type", json::string("integer"))]))]),
        ),
    ]);
    vec![
        spec(
            "physics_scenes",
            "List the physics debug scenes.",
            no_args(),
        ),
        spec("physics_load", "Load a scene at step 0.", scene_arg),
        spec(
            "physics_reset",
            "Reload the current scene at step 0.",
            no_args(),
        ),
        spec(
            "physics_step",
            "Advance N fixed steps; returns their stats lines.",
            steps_arg.clone(),
        ),
        spec("physics_stats", "The last step's stats.", no_args()),
        spec(
            "physics_body",
            "Dump one body's pose, velocities and mass.",
            index_arg,
        ),
        spec(
            "physics_replay",
            "Rerun the current scene twice from step 0 for N steps and compare hashes.",
            steps_arg,
        ),
        spec("physics_quit", "Stop serving and exit.", no_args()),
    ]
}

fn serve(info: &'static SceneInfo, port: u16) -> Result<(), String> {
    command::register(specs());
    let host = genos_mcp::Host::new(
        genos_scene::load_str(MCP_ROOM)?,
        genos_scene::Camera::new(0.0, 0.0, 0.0),
    );
    let _server = genos_mcp::Server::start_on(host, port)?;
    eprintln!("mcp http://127.0.0.1:{port}/mcp scene={}", info.name);
    let mut session = Session::load(info);
    loop {
        for pending in command::take_wait(Duration::from_secs(1)) {
            let quit = pending.name == "physics_quit";
            let result = handle(&mut session, &pending.name, &pending.args);
            command::reply(pending.id, result);
            if quit {
                return Ok(());
            }
        }
    }
}

fn lines(text: &[String]) -> Result<Value, String> {
    json::parse(&format!("[{}]", text.join(",")))
}

fn handle(session: &mut Session, name: &str, args: &Value) -> Result<Value, String> {
    let steps = || {
        args.get("steps")
            .and_then(Value::as_u64)
            .map_or(Ok(1), u32::try_from)
            .map_err(|_| "steps out of range".to_string())
    };
    match name {
        "physics_scenes" => Ok(json::array(
            SCENES
                .iter()
                .map(|s| {
                    json::object([
                        ("name", json::string(s.name)),
                        ("summary", json::string(s.summary)),
                        ("steps", json::int(i64::from(s.steps))),
                    ])
                })
                .collect(),
        )),
        "physics_load" => {
            let name = args
                .get("scene")
                .and_then(Value::as_str)
                .ok_or("scene is required")?;
            *session = Session::load(scene(name)?);
            Ok(json::string(format!("loaded {name}")))
        }
        "physics_reset" => {
            session.reset();
            Ok(json::string(format!("reset {}", session.scene.name)))
        }
        "physics_step" => {
            let scene = session.scene.name;
            lines(
                &session
                    .run(steps()?)
                    .iter()
                    .map(|r| r.json(scene))
                    .collect::<Vec<_>>(),
            )
        }
        "physics_stats" => match session.records.last() {
            Some(record) => json::parse(&record.json(session.scene.name)),
            None => Ok(Value::Null),
        },
        "physics_body" => {
            let index = args.get("index").and_then(Value::as_u64).unwrap_or(0);
            let body = usize::try_from(index)
                .ok()
                .and_then(|i| session.world.bodies.get(i))
                .ok_or("no such body")?;
            let v = |v: genos_math::Vec3| {
                json::array(vec![
                    json::float(f64::from(v.x)),
                    json::float(f64::from(v.y)),
                    json::float(f64::from(v.z)),
                ])
            };
            let o = body.orientation;
            Ok(json::object([
                ("position", v(body.position)),
                ("velocity", v(body.velocity)),
                ("angular_velocity", v(body.angular_velocity)),
                (
                    "orientation",
                    json::array(
                        [o.x, o.y, o.z, o.w]
                            .iter()
                            .map(|c| json::float(f64::from(*c)))
                            .collect(),
                    ),
                ),
                ("inverse_mass", json::float(f64::from(body.inverse_mass))),
            ]))
        }
        "physics_replay" => match replay(session.scene, steps()?) {
            Ok(hash) => Ok(json::string(format!("replay=match hash={hash:016x}"))),
            Err(step) => Err(format!("replay=diverged step={step}")),
        },
        "physics_quit" => Ok(json::string("bye")),
        other => Err(format!("unknown command {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Option<Options>, String> {
        parse_args(list.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn parses_the_cli() {
        let opts = args(&["--scene", "resting_pile", "--steps", "5", "--replay"]);
        let opts = opts.ok().flatten().unwrap_or_else(|| unreachable!());
        assert_eq!(opts.scene, "resting_pile");
        assert_eq!(opts.steps, Some(5));
        assert!(opts.replay);
        assert!(args(&["--bogus"]).is_err());
        assert!(args(&["--mcp-port", "0"]).is_err());
        assert!(matches!(args(&["--help"]), Ok(None)));
    }

    #[test]
    fn commands_drive_a_session() {
        let mut session = Session::load(&SCENES[0]);
        assert!(handle(
            &mut session,
            "physics_load",
            &json::object([("scene", json::string("hull_edges"))])
        )
        .is_ok());
        let out = handle(
            &mut session,
            "physics_step",
            &json::object([("steps", json::int(3))]),
        );
        assert_eq!(
            out.ok().and_then(|v| v.as_array().map(<[Value]>::len)),
            Some(3)
        );
        assert!(handle(
            &mut session,
            "physics_body",
            &json::object([("index", json::int(0))])
        )
        .is_ok());
        assert!(handle(
            &mut session,
            "physics_replay",
            &json::object([("steps", json::int(5))])
        )
        .is_ok());
        assert!(handle(&mut session, "physics_reset", &no_args()).is_ok());
        assert_eq!(session.steps, 0);
        assert!(handle(&mut session, "nope", &no_args()).is_err());
    }
}
