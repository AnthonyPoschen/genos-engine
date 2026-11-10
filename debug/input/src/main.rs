//! `genos-input`: headless input debug binary.
//!
//! Prints the glyph set, actions, devices, touch points, and gestures.
//! `--mcp` serves `get_input`, `set_input`, and `get_glyphs`.

use std::process::ExitCode;
use std::time::Duration;

use genos_input_debug::{Session, COMMANDS};
use genos_mcp::command::{self, CommandSpec};
use genos_mcp::json::{self, Value};

/// Lighting uses 8766. Physics uses 8767.
const MCP_PORT: u16 = 8768;

/// A scene the MCP host can load. The input commands do not read it.
const MCP_ROOM: &str = include_str!("../../../examples/stress/scenes/room.rhai");

const USAGE: &str = "genos-input: headless input debug binary

  --frames N             poll gamepads and print N views (default 1)
  --script PATH          run a script of device writes and `frame` lines
  --rebind KEY=VALUE     overlay one settings line before the run (repeatable)
  --mcp                  serve MCP and POST /cmd/<name> on 127.0.0.1:8768
  --mcp-port N           same, on port N
  -h, --help             this text

Script lines: `key CODE down|up`, `button CODE down|up`, `mouse DX DY`,
`pad SLOT connect NAME`, `pad SLOT disconnect`, `stick SLOT left|right X Y`,
`padbutton SLOT CODE down|up`, `trigger SLOT left|right VALUE`,
`touch ID X Y began|moved|ended|cancelled`, `rebind KEY=VALUE`, `frame`.
Touch `X` and `Y` are points (pixels).

The window backends (Wayland now; Windows, macOS, and X11 in the windowing
work) call set_key, set_mouse_button, add_mouse_delta, connect_gamepad,
set_gamepad_stick, set_gamepad_button, set_gamepad_trigger, and
set_touch_points. They do not recognize gestures. This binary does not
open those platform devices except the Linux joystick poll.
";

struct Options {
    frames: u32,
    script: Option<String>,
    rebinds: Vec<(String, String)>,
    mcp_port: Option<u16>,
}

fn main() -> ExitCode {
    match run(std::env::args().skip(1)) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("genos-input: {err}");
            ExitCode::from(1)
        }
    }
}

fn run(args: impl Iterator<Item = String>) -> Result<ExitCode, String> {
    let Some(options) = parse_args(args)? else {
        print!("{USAGE}");
        return Ok(ExitCode::from(0));
    };
    let mut session = Session::new();
    for (key, value) in &options.rebinds {
        session.rebind(key, value)?;
    }
    if let Some(path) = &options.script {
        let text = std::fs::read_to_string(path).map_err(|err| format!("{path}: {err}"))?;
        print!("{}", session.run_script(&text)?);
    } else if options.mcp_port.is_none() {
        for _ in 0..options.frames {
            session.input.poll_gamepads();
            println!("{}", session.finish_frame());
        }
    }
    if let Some(port) = options.mcp_port {
        serve(&mut session, port)?;
    }
    Ok(ExitCode::from(0))
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Option<Options>, String> {
    let mut options = Options {
        frames: 1,
        script: None,
        rebinds: Vec::new(),
        mcp_port: None,
    };
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--frames" => options.frames = number(&value("--frames")?)?,
            "--script" => options.script = Some(value("--script")?),
            "--rebind" => {
                let text = value("--rebind")?;
                let Some((key, value)) = text.split_once('=') else {
                    return Err("--rebind needs key=value".into());
                };
                options
                    .rebinds
                    .push((key.trim().to_string(), value.trim().to_string()));
            }
            "--mcp" => options.mcp_port = Some(MCP_PORT),
            "--mcp-port" => {
                let port: u16 = number(&value("--mcp-port")?)?;
                if port == 0 {
                    return Err("--mcp-port needs 1 to 65535".into());
                }
                options.mcp_port = Some(port);
            }
            other => return Err(format!("unknown argument {other}\n\n{USAGE}")),
        }
    }
    Ok(Some(options))
}

fn number<T: std::str::FromStr>(text: &str) -> Result<T, String> {
    text.parse().map_err(|_| format!("not a number: {text}"))
}

fn specs() -> Vec<CommandSpec> {
    COMMANDS
        .iter()
        .map(|(name, description)| CommandSpec {
            name: (*name).to_string(),
            description: (*description).to_string(),
            schema: json::object([
                ("type", json::string("object")),
                ("additionalProperties", json::bool(true)),
            ]),
        })
        .collect()
}

fn serve(session: &mut Session, port: u16) -> Result<(), String> {
    command::register(specs());
    let host = genos_mcp::Host::new(
        genos_scene::load_str(MCP_ROOM)?,
        genos_scene::Camera::new(0.0, 0.0, 0.0),
    );
    let server = genos_mcp::Server::start_on(host, port)?;
    eprintln!("mcp {}", server.url());
    loop {
        session.input.poll_gamepads();
        for pending in command::take_wait(Duration::from_millis(200)) {
            let result = handle(session, &pending.name, &pending.args);
            command::reply(pending.id, result);
        }
    }
}

fn handle(session: &mut Session, name: &str, args: &Value) -> Result<Value, String> {
    let key = args.get("key").and_then(Value::as_str);
    let value = args.get("value").and_then(Value::as_str);
    let text = args.get("text").and_then(Value::as_str);
    let body = session.command(name, key, value, text)?;
    Ok(json::object([
        ("text", json::string(body)),
        ("glyphs", json::string(session.input.active_glyphs().name())),
    ]))
}
