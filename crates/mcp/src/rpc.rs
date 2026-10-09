//! MCP methods for the live scene. The socket layer does not edit the scene.

use crate::json::{self, Value};
use crate::live::{Host, ToolResult, SCENE_URI};

const VERSIONS: &[&str] = &["2025-03-26", "2025-06-18"];

pub enum Reply {
    Result(Value),
    Error { code: i64, message: String },
}

pub fn negotiate(params: &Value) -> String {
    let requested = params.get("protocolVersion").and_then(Value::as_str);
    if let Some(version) = requested {
        if VERSIONS.contains(&version) {
            return version.to_string();
        }
    }
    "2025-06-18".to_string()
}

pub fn known_version(version: &str) -> bool {
    VERSIONS.contains(&version)
}

pub fn initialize_result(params: &Value) -> Value {
    json::object([
        ("protocolVersion", json::string(negotiate(params))),
        (
            "capabilities",
            json::object([
                ("tools", json::object([("listChanged", json::bool(false))])),
                (
                    "resources",
                    json::object([
                        ("subscribe", json::bool(true)),
                        ("listChanged", json::bool(false)),
                    ]),
                ),
            ]),
        ),
        (
            "serverInfo",
            json::object([
                ("name", json::string("genos")),
                ("version", json::string("0.1.0")),
            ]),
        ),
        (
            "instructions",
            json::string(
                "Read and edit the live scene at genos://scene. A handle stays the same until that object is removed.",
            ),
        ),
    ])
}

pub fn dispatch(host: &Host, session: &str, method: &str, params: &Value) -> Reply {
    match method {
        "ping" => Reply::Result(json::object([])),
        "tools/list" => Reply::Result(json::object([("tools", tools())])),
        "tools/call" => call_tool(host, params),
        "resources/list" => Reply::Result(json::object([("resources", resources())])),
        "resources/read" => read_resource(host, params),
        "resources/subscribe" => subscribe(host, session, params, true),
        "resources/unsubscribe" => subscribe(host, session, params, false),
        _ => Reply::Error {
            code: -32601,
            message: format!("unknown method {method}"),
        },
    }
}

fn call_tool(host: &Host, params: &Value) -> Reply {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return Reply::Error {
            code: -32602,
            message: "missing tool name".into(),
        };
    };
    let empty = json::object([]);
    let arguments = params.get("arguments").unwrap_or(&empty);
    if name == "lighting_status" {
        return Reply::Result(tool_content(crate::lighting::current(), false));
    }
    if name == "take_screenshot" {
        return match crate::control::wait_shot(std::time::Duration::from_secs(2)) {
            Ok(path) => Reply::Result(tool_content(path.display().to_string(), false)),
            Err(text) => Reply::Result(tool_content(text, true)),
        };
    }
    if name == "set_lighting" {
        return match settings_text(arguments) {
            Ok(text) => match crate::control::post_settings(&text) {
                Ok(body) => Reply::Result(tool_content(body, false)),
                Err(body) => Reply::Result(tool_content(body, true)),
            },
            Err(text) => Reply::Result(tool_content(text, true)),
        };
    }
    if crate::command::known(name) {
        return Reply::Result(
            match crate::command::call(name, arguments.clone(), command_timeout(arguments)) {
                Ok(value) => tool_content(json::encode(&value), false),
                Err(text) => tool_content(text, true),
            },
        );
    }
    match host.call_tool(name, arguments) {
        ToolResult::Unknown => Reply::Error {
            code: -32602,
            message: format!("unknown tool {name}"),
        },
        ToolResult::Done(Ok(text)) => Reply::Result(tool_content(text, false)),
        ToolResult::Done(Err(text)) => Reply::Result(tool_content(text, true)),
    }
}

/// How long a command may take: `timeout_s` in its arguments, else two minutes
/// (a wait for the light to settle on a software GPU takes tens of seconds).
pub fn command_timeout(arguments: &Value) -> std::time::Duration {
    let seconds = arguments
        .get("timeout_s")
        .and_then(Value::as_f64)
        .unwrap_or(120.0);
    std::time::Duration::from_secs_f64(seconds.clamp(0.1, 3600.0) + 1.0)
}

fn read_resource(host: &Host, params: &Value) -> Reply {
    match params.get("uri").and_then(Value::as_str) {
        Some(SCENE_URI) => Reply::Result(json::object([(
            "contents",
            json::array(vec![json::object([
                ("uri", json::string(SCENE_URI)),
                ("mimeType", json::string("application/json")),
                ("text", json::string(host.scene_text())),
            ])]),
        )])),
        Some(uri) => Reply::Error {
            code: -32602,
            message: format!("unknown resource {uri}"),
        },
        None => Reply::Error {
            code: -32602,
            message: "missing uri".into(),
        },
    }
}

fn subscribe(host: &Host, session: &str, params: &Value, on: bool) -> Reply {
    let Some(uri) = params.get("uri").and_then(Value::as_str) else {
        return Reply::Error {
            code: -32602,
            message: "missing uri".into(),
        };
    };
    let result = if on {
        host.subscribe(session, uri)
    } else {
        host.unsubscribe(session, uri)
    };
    match result {
        Ok(()) => Reply::Result(json::object([])),
        Err(message) => Reply::Error {
            code: -32602,
            message,
        },
    }
}

fn tool_content(text: String, is_error: bool) -> Value {
    json::object([
        (
            "content",
            json::array(vec![json::object([
                ("type", json::string("text")),
                ("text", json::string(text)),
            ])]),
        ),
        ("isError", json::bool(is_error)),
    ])
}

fn tools() -> Value {
    let mut list = builtin_tools();
    for spec in crate::command::specs() {
        list.push(tool(&spec.name, &spec.description, spec.schema));
    }
    json::array(list)
}

fn builtin_tools() -> Vec<Value> {
    vec![
        tool(
            "read_scene",
            "Read the live floor, walls, solids, lights, and camera.",
            schema(&[], vec![]),
        ),
        tool(
            "lighting_status",
            "Read the live probe report. stable is true when no brick is still updating.",
            schema(&[], vec![]),
        ),
        tool(
            "take_screenshot",
            "Save the live picture and return its path. The light is not settled first.",
            schema(&[], vec![]),
        ),
        tool(
            "set_lighting",
            "Change the sun, the sky, the moving boxes, the moving-lamp share, or the time of day.",
            schema(
                &[],
                vec![
                    ("sun", enum_schema(&["freeze", "run"])),
                    ("sky", enum_schema(&["on", "off"])),
                    ("boxes", enum_schema(&["still", "move"])),
                    ("dynamic", number_schema()),
                    ("day", number_schema()),
                ],
            ),
        ),
        tool(
            "set_object",
            "Change stored fields on the object with this handle.",
            schema(
                &["handle"],
                vec![
                    ("handle", string_schema()),
                    ("position", vec3_schema()),
                    ("half_x", number_schema()),
                    ("half_z", number_schema()),
                    ("height", number_schema()),
                    ("size", number_schema()),
                    ("shape", enum_schema(&["square", "circle"])),
                    ("color", vec3_schema()),
                    ("absorption", number_schema()),
                ],
            ),
        ),
        tool(
            "spawn_object",
            "Add a wall, a solid, or a light. The result names the new handle.",
            schema(
                &["kind"],
                vec![
                    ("kind", enum_schema(&["wall", "solid", "light"])),
                    ("position", vec3_schema()),
                    ("half_x", number_schema()),
                    ("half_z", number_schema()),
                    ("height", number_schema()),
                    ("size", number_schema()),
                    ("shape", enum_schema(&["square", "circle"])),
                    ("color", vec3_schema()),
                    ("absorption", number_schema()),
                ],
            ),
        ),
        tool(
            "remove_object",
            "Remove the wall, solid, or light with this handle. The floor stays.",
            schema(&["handle"], vec![("handle", string_schema())]),
        ),
        tool(
            "set_camera",
            "Set the eye position, yaw, pitch, or look direction.",
            schema(
                &[],
                vec![
                    ("position", vec3_schema()),
                    ("yaw", number_schema()),
                    ("pitch", number_schema()),
                    ("look", vec3_schema()),
                ],
            ),
        ),
    ]
}

fn resources() -> Value {
    json::array(vec![json::object([
        ("uri", json::string(SCENE_URI)),
        ("name", json::string("scene")),
        (
            "description",
            json::string("Live floor, walls, solids, lights, and camera."),
        ),
        ("mimeType", json::string("application/json")),
    ])])
}

fn settings_text(arguments: &Value) -> Result<String, String> {
    let mut lines = Vec::new();
    for key in ["sun", "sky", "boxes", "dynamic", "day"] {
        let Some(value) = arguments.get(key) else {
            continue;
        };
        let text = value
            .as_str()
            .map(str::to_string)
            .or_else(|| value.as_f64().map(|n| n.to_string()))
            .ok_or_else(|| format!("{key} is a string or a number"))?;
        lines.push(format!("{key}={text}"));
    }
    Ok(lines.join("\n"))
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json::object([
        ("name", json::string(name)),
        ("description", json::string(description)),
        ("inputSchema", input_schema),
    ])
}

fn schema(required: &[&str], properties: Vec<(&str, Value)>) -> Value {
    json::object([
        ("type", json::string("object")),
        (
            "properties",
            Value::Object(
                properties
                    .into_iter()
                    .map(|(key, value)| (key.to_string(), value))
                    .collect(),
            ),
        ),
        (
            "required",
            json::array(required.iter().copied().map(json::string).collect()),
        ),
        ("additionalProperties", json::bool(false)),
    ])
}

fn string_schema() -> Value {
    json::object([("type", json::string("string"))])
}

fn number_schema() -> Value {
    json::object([("type", json::string("number"))])
}

fn vec3_schema() -> Value {
    json::object([
        ("type", json::string("array")),
        ("minItems", json::int(3)),
        ("maxItems", json::int(3)),
        ("items", number_schema()),
    ])
}

fn enum_schema(values: &[&str]) -> Value {
    json::object([
        ("type", json::string("string")),
        (
            "enum",
            json::array(values.iter().copied().map(json::string).collect()),
        ),
    ])
}
