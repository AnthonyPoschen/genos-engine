#![cfg_attr(test, expect(clippy::disallowed_methods, reason = "TODO(lint-2026-10-14): publish intent with the CLI argument, not set_var. docs/LINT_RULES.md"))]
#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(test, expect(clippy::panic, reason = "a test aborts when a fixture fails"))]

//! MCP round trip for the lighting settings: list, apply, partial set, read back.
//!
//! The command thread blocks in `command::call`. The test thread drains the
//! queue in `Tools::before_draw` and draws between commands, so the get sees
//! the settings and the GPU times from the frames after the set.

use std::time::{Duration, Instant};

use genos_debug::{Ctx, Tools};
use genos_mcp::json::{self, Value};
use genos_render::{Renderer, World};
use genos_scene::{Camera, Floor, Light, Scene, Vec3};
use genos_window::Window;

fn open() -> genos_window::Lifecycle<Renderer> {
    std::env::set_var("GENOS_GPU_TIMES", "30");
    let mut window = Window::open_proof(320, 180).expect("proof window");
    let frame = window.pump();
    let (renderer, permit) = Renderer::open(window.display, window.surface, frame.width, frame.height)
        .expect("renderer");
    genos_window::Lifecycle::new(window, renderer, permit)
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
        lighting: genos_scene::LightingSettings::default(),
        debug: genos_scene::DebugSettings::default(),
        fps_counter: genos_scene::FpsCounterSettings::default(),
        event_loop: genos_scene::EventLoopSettings::default(),
        menu: genos_scene::MenuSettings::default(),
    })
}

fn call(name: &str, args: Value) -> Value {
    genos_mcp::command::call(name, args, Duration::from_secs(60))
        .unwrap_or_else(|err| panic!("{name}: {err}"))
}

fn text_of(value: &Value) -> String {
    value.get("text").and_then(Value::as_str).unwrap_or("").to_string()
}

#[test]
fn lighting_settings_round_trip_over_mcp() {
    let mut owner = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let mut world = room();
    let mut camera = Camera::opening();
    let mut tools = Tools::new();
    let started = Instant::now();
    let worker = std::thread::spawn(|| {
        let listed = call("list_presets", json::object([]));
        let applied = call("apply_preset", json::object([("name", json::string("potato"))]));
        let day = call("apply_preset", json::object([("name", json::string("day-cycle"))]));
        let bounce = call("apply_preset", json::object([("name", json::string("gi-1bounce"))]));
        let updated = call(
            "set_lighting_settings",
            json::object([("gi", json::bool(true)), ("bounces", json::string("1"))]),
        );
        let read = call("get_lighting_settings", json::object([]));
        (listed, applied, day, bounce, updated, read)
    });
    while !worker.is_finished() {
        assert!(started.elapsed() < Duration::from_secs(180), "mcp round trip timed out");
        tools.before_draw(&mut Ctx {
            renderer,
            camera: &mut camera,
            host: &mut world.scene,
        });
        for _ in 0..6 {
            let _ = window.pump();
            renderer.draw(&world, &camera, false).expect("draw");
        }
    }
    let (listed, applied, day, bounce, updated, read) = worker.join().expect("mcp thread");
    let names: Vec<_> = listed
        .get("presets")
        .and_then(Value::as_array)
        .unwrap_or(&[])
        .iter()
        .filter_map(|preset| preset.get("name").and_then(Value::as_str))
        .collect();
    assert_eq!(
        names,
        ["potato", "day-cycle", "gi-1bounce", "low", "medium", "high"]
    );
    assert!(text_of(&applied).contains("preset=potato"), "{}", text_of(&applied));
    assert!(text_of(&applied).contains("gi=0"), "{}", text_of(&applied));
    let day_text = text_of(&day);
    assert!(day_text.contains("preset=day-cycle"), "{day_text}");
    assert!(day_text.contains("gi=0"), "{day_text}");
    assert!(day_text.contains("ambient=curve"), "{day_text}");
    assert!(day_text.contains("shadows=1"), "{day_text}");
    let bounce_text = text_of(&bounce);
    assert!(bounce_text.contains("preset=gi-1bounce"), "{bounce_text}");
    assert!(bounce_text.contains("bounces=1"), "{bounce_text}");
    assert!(bounce_text.contains("world_cache=0"), "{bounce_text}");
    assert!(bounce_text.contains("denoise=0"), "{bounce_text}");
    assert!(bounce_text.contains("prefill=0"), "{bounce_text}");
    assert!(text_of(&updated).contains("gi=1"), "{}", text_of(&updated));
    assert!(text_of(&updated).contains("bounces=1"), "{}", text_of(&updated));
    let read_text = text_of(&read);
    assert!(read_text.contains("gi=1"), "{read_text}");
    assert!(read_text.contains("bounces=1"), "{read_text}");
    let passes = read.get("passes").and_then(Value::as_array).expect("passes");
    assert!(passes.len() >= 12, "pass list {}", passes.len());
    let compose = passes
        .iter()
        .find(|pass| pass.get("name").and_then(Value::as_str) == Some("compose"))
        .expect("compose pass");
    let dispatches = compose.get("dispatches").and_then(Value::as_f64).unwrap_or(0.0);
    assert!(dispatches > 0.0, "compose dispatches {dispatches}");
    let timed = passes.iter().any(|pass| pass.get("ms").and_then(Value::as_f64).unwrap_or(0.0) > 0.0);
    assert!(timed, "get_lighting_settings returned no GPU time");
}
