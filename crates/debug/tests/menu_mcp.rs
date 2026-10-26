#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(test, expect(clippy::panic, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(test, expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails"))]

//! Menu visibility, page, and folds round-trip through the settings text and MCP.
//!
//! Config checks run before the window opens. The command thread then blocks in
//! `command::call` while this thread drains the queue in `Tools::before_draw`.

use std::time::{Duration, Instant};

use genos_debug::{Ctx, Tools};
use genos_mcp::json::{self, Value};
use genos_render::{Renderer, World};
use genos_scene::{Camera, EngineSettings, Floor, Light, MenuSettings, Scene, Vec3};
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
        lighting: genos_scene::LightingSettings::preset(genos_scene::LightingPreset::Potato),
        debug: genos_scene::DebugSettings::default(),
        fps_counter: genos_scene::FpsCounterSettings::default(),
        event_loop: genos_scene::EventLoopSettings::default(),
        hdr: genos_scene::HdrSettings::legacy_bytes(),
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
fn menu_settings_round_trip_through_config_and_mcp() {
    let mut settings = MenuSettings::default();
    assert!(settings.visible);
    let parsed = <MenuSettings as EngineSettings>::parse(&EngineSettings::to_text(&settings))
        .expect("parse default");
    assert_eq!(parsed, settings);
    EngineSettings::set_key(&mut settings, "visible", "hidden").unwrap();
    EngineSettings::set_key(&mut settings, "tab", "GI").unwrap();
    EngineSettings::set_key(&mut settings, "fold", "world").unwrap();
    assert!(!settings.visible);
    assert_eq!(settings.tab, "GI");
    assert_eq!(settings.collapsed, vec!["world".to_string()]);

    let (mut window, mut renderer) = open();
    let mut world = room();
    let mut camera = Camera::opening();
    let mut tools = Tools::new();
    let started = Instant::now();
    let worker = std::thread::spawn(|| {
        let initial = call("get_menu_settings", json::object([]));
        let hidden = call(
            "set_menu_settings",
            json::object([
                ("visible", json::string("hidden")),
                ("tab", json::string("GI")),
                ("collapsed", json::array(vec![json::string("world")])),
            ]),
        );
        let folded = call(
            "set_menu_settings",
            json::object([("text", json::string("fold=gi"))]),
        );
        let opened = call(
            "set_menu_settings",
            json::object([("open", json::string("world"))]),
        );
        let after = call("get_menu_settings", json::object([]));
        (initial, hidden, folded, opened, after)
    });
    while !worker.is_finished() {
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "menu mcp round trip timed out"
        );
        tools.before_draw(&mut Ctx {
            renderer: &mut renderer,
            camera: &mut camera,
            host: &mut world.scene,
        });
        let _ = window.pump();
    }
    let (initial, hidden, folded, opened, after) = worker.join().expect("mcp thread");
    let initial_text = text_of(&initial);
    assert!(initial_text.contains("visible=shown"), "{initial_text}");
    assert!(initial_text.contains("wheel=hover"), "{initial_text}");
    assert!(initial_text.contains("keys=click"), "{initial_text}");
    assert_eq!(initial.get("wheel").and_then(Value::as_str), Some("hover"));
    assert_eq!(initial.get("keys").and_then(Value::as_str), Some("click"));
    assert_eq!(initial.get("visible").and_then(Value::as_bool), Some(true));
    assert_eq!(initial.get("tab").and_then(Value::as_str), Some(""));
    let hidden_text = text_of(&hidden);
    assert!(hidden_text.contains("visible=hidden"), "{hidden_text}");
    assert!(hidden_text.contains("tab=GI"), "{hidden_text}");
    assert!(hidden_text.contains("collapsed=world"), "{hidden_text}");
    assert_eq!(hidden.get("visible").and_then(Value::as_bool), Some(false));
    let folded_text = text_of(&folded);
    assert!(folded_text.contains("collapsed=world,gi"), "{folded_text}");
    let opened_text = text_of(&opened);
    assert!(opened_text.contains("collapsed=gi"), "{opened_text}");
    assert!(!opened_text.contains("world"), "{opened_text}");
    assert_eq!(after.get("tab").and_then(Value::as_str), Some("GI"));
    assert!(!world.scene.menu.visible);
    assert_eq!(world.scene.menu.tab, "GI");
    assert_eq!(world.scene.menu.collapsed, vec!["gi".to_string()]);
}
