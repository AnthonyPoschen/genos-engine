#![cfg_attr(test, expect(clippy::bool_assert_comparison, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(test, expect(clippy::panic, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(test, expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails"))]

//! The frame-rate toggle round-trips through the settings text and through MCP.
//!
//! Config checks run before the window opens. The command thread then blocks in
//! `command::call` while this thread drains the queue in `Tools::before_draw`.

use std::time::{Duration, Instant};

use genos_debug::{Ctx, Tools};
use genos_mcp::json::{self, Value};
use genos_render::{Renderer, World};
use genos_scene::{
    Camera, EngineSettings, EventLoopSettings, Floor, FogSettings, FpsCounterSettings, Light,
    MenuSettings, ReflectionSettings, Scene, Vec3,
};
use genos_window::Window;

fn open() -> (Window, Renderer) {
    let mut window = Window::open_proof(320, 180).expect("proof window");
    let frame = window.pump();
    let renderer = Renderer::open(window.display, window.surface, frame.width, frame.height)
        .expect("renderer");
    (window, renderer)
}

fn room() -> World {
    World::from_scene(Scene {
        floor: Floor {
            position: Vec3::ZERO,
            half_x: 4.0,
            half_z: 4.0,
            color: [0.5, 0.5, 0.5],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights: vec![Light {
            position: Vec3::new(0.0, 2.0, 0.0),
            color: [1.0, 1.0, 1.0],
            direction: Vec3::new(0.0, -1.0, 0.0),
        }],
        ceiling: None,
        sky: None,
        lighting: genos_scene::LightingSettings::preset(genos_scene::LightingPreset::Fallback),
        debug: genos_scene::DebugSettings::default(),
        fps_counter: FpsCounterSettings::default(),
        event_loop: EventLoopSettings::default(),
        reflections: ReflectionSettings::default(),
        hdr: genos_scene::HdrSettings::legacy_bytes(),
        fog: FogSettings::default(),
        menu: MenuSettings::default(),
    })
}

fn call(name: &str, args: Value) -> Value {
    genos_mcp::command::call(name, args, Duration::from_secs(30))
        .unwrap_or_else(|err| panic!("{name}: {err}"))
}

fn text_of(value: &Value) -> String {
    value
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

#[test]
fn fps_counter_round_trips_through_config_and_mcp() {
    let mut settings = FpsCounterSettings::default();
    assert!(!settings.enabled);
    assert_eq!(settings.rate_hz, 4.0);
    let parsed = <FpsCounterSettings as EngineSettings>::parse(&EngineSettings::to_text(&settings))
        .expect("parse default");
    assert_eq!(parsed, settings);
    EngineSettings::set_key(&mut settings, "enabled", "on").unwrap();
    EngineSettings::set_key(&mut settings, "rate_hz", "8").unwrap();
    let again = FpsCounterSettings::parse(&settings.to_text()).unwrap();
    assert_eq!(again.enabled, true);
    assert_eq!(again.rate_hz, 8.0);
    EngineSettings::set_key(&mut settings, "enabled", "off").unwrap();
    assert!(!settings.enabled);

    let (mut window, mut renderer) = open();
    let mut world = room();
    let mut camera = Camera::opening();
    let mut tools = Tools::new();
    let started = Instant::now();
    let worker = std::thread::spawn(|| {
        let initial = call("get_fps_counter", json::object([]));
        let enabled = call(
            "set_fps_counter",
            json::object([("enabled", json::bool(true)), ("rate_hz", json::float(8.0))]),
        );
        let read = call("get_fps_counter", json::object([]));
        let cleared = call(
            "set_fps_counter",
            json::object([("text", json::string("enabled=0; rate_hz=4"))]),
        );
        let after = call("get_fps_counter", json::object([]));
        (initial, enabled, read, cleared, after)
    });
    while !worker.is_finished() {
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "fps mcp round trip timed out"
        );
        tools.before_draw(&mut Ctx {
            renderer: &mut renderer,
            camera: &mut camera,
            host: &mut world.scene,
        });
        let _ = window.pump();
    }
    let (initial, enabled, read, cleared, after) = worker.join().expect("mcp thread");
    let initial_text = text_of(&initial);
    assert!(initial_text.contains("enabled=0"), "{initial_text}");
    assert!(initial_text.contains("rate_hz=4.0"), "{initial_text}");
    assert_eq!(initial.get("enabled").and_then(Value::as_bool), Some(false));
    let enabled_text = text_of(&enabled);
    assert!(enabled_text.contains("enabled=1"), "{enabled_text}");
    assert!(enabled_text.contains("rate_hz=8"), "{enabled_text}");
    assert_eq!(enabled.get("enabled").and_then(Value::as_bool), Some(true));
    assert_eq!(read.get("enabled").and_then(Value::as_bool), Some(true));
    assert!((read.get("rate_hz").and_then(Value::as_f64).unwrap_or(0.0) - 8.0).abs() < 1.0e-3);
    let cleared_text = text_of(&cleared);
    assert!(cleared_text.contains("enabled=0"), "{cleared_text}");
    assert!(text_of(&after).contains("enabled=0"), "{}", text_of(&after));
    assert!(!world.scene.fps_counter.enabled);
    assert_eq!(world.scene.fps_counter.rate_hz, 4.0);
}
