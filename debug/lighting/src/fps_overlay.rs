#![expect(clippy::as_conversions, reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md")]
#![expect(clippy::cast_precision_loss, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]
#![expect(clippy::expect_used, reason = "TODO(lint-2026-10-14): pre-existing. docs/LINT_RULES.md")]

//! Lavapipe check that the frame-rate plate is in the picture.
//!
//! The plate is built on the CPU at the update rate, then drawn with the same
//! overlay path `genos-lighting` uses. Potato lighting keeps the frame on the
//! direct pass. Times below are the draw, not a GPU timestamp (lavapipe does
//! not time the GPU).

use std::time::Instant;

use genos_debug::png;
use genos_render::{Renderer, World};
use genos_scene::{
    Camera, EventLoopSettings, FogSettings, Floor, FpsCounterSettings, Light, MenuSettings, Scene, Vec3,
};
use genos_ui::FpsOverlay;
use genos_window::Window;

fn room() -> World {
    World::from_scene(Scene {
        floor: Floor {
            position: Vec3::ZERO,
            half_x: 4.0,
            half_z: 4.0,
            color: [0.45, 0.47, 0.5],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights: vec![Light {
            position: Vec3::new(0.0, 2.2, 0.0),
            color: [1.0, 0.95, 0.85],
            direction: Vec3::new(0.0, -1.0, 0.0),
        }],
        ceiling: None,
        sky: None,
        lighting: genos_scene::LightingSettings::preset(genos_scene::LightingPreset::Potato),
        debug: genos_scene::DebugSettings::default(),
        fps_counter: FpsCounterSettings {
            enabled: true,
            rate_hz: 4.0,
        },
        event_loop: EventLoopSettings::default(),
        hdr: genos_scene::HdrSettings::legacy_bytes(),
        fog: FogSettings::default(),
        menu: MenuSettings::default(),
    })
}

fn rects(overlay: &FpsOverlay) -> Vec<genos_render::ScreenRect> {
    overlay
        .paints()
        .iter()
        .map(|paint| genos_render::ScreenRect {
            x: paint.x,
            y: paint.y,
            w: paint.w,
            h: paint.h,
            color: paint.color,
        })
        .collect()
}

fn corner_diff(off: &[u8], on: &[u8], width: u32, height: u32) -> usize {
    let x0 = width.saturating_sub(180);
    let y0 = height.saturating_sub(48);
    let mut changed = 0usize;
    for y in y0..height {
        for x in x0..width {
            let i = ((y * width + x) * 4) as usize;
            if off.get(i..i + 3) != on.get(i..i + 3) {
                changed += 1;
            }
        }
    }
    changed
}

fn mean_ms(samples: &[f64]) -> f64 {
    if samples.is_empty() {
        0.0
    } else {
        samples.iter().sum::<f64>() / samples.len() as f64
    }
}

#[test]
fn fps_overlay_renders_on_lavapipe() {
    let mut overlay = FpsOverlay::new();
    let view = [320.0_f32, 180.0];
    for _ in 0..14 {
        overlay.note(true, 4.0, 1.0 / 60.0, view);
    }
    assert!(overlay.paints().is_empty());
    overlay.note(true, 4.0, 1.0 / 60.0, view);
    assert_eq!(overlay.rebuilds(), 1);
    let plate = rects(&overlay);
    assert!(plate.len() > 8, "plate rects {}", plate.len());

    let mut window = Window::open_proof(320, 180).expect("proof window");
    let frame = window.pump();
    let (renderer, permit) = Renderer::open(window.display, window.surface, frame.width, frame.height)
        .expect("renderer");
    let mut owner = genos_window::Lifecycle::new(window, renderer, permit);
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let world = room();
    let camera = Camera::opening();
    let width = renderer.width();
    let height = renderer.height();

    let _ = window.pump();
    renderer
        .draw_with_overlay(&world, &camera, &[], false, false)
        .expect("warmup");
    // The first overlay draw builds the plate vertices. Time the frames after that.
    let _ = window.pump();
    renderer
        .draw_with_overlay(&world, &camera, &plate, false, false)
        .expect("overlay warmup");

    let mut off_ms = Vec::new();
    let mut bare = None;
    for index in 0..4 {
        let _ = window.pump();
        let start = Instant::now();
        let pixels = renderer
            .draw_with_overlay(&world, &camera, &[], index == 3, false)
            .expect("draw off");
        off_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        if index == 3 {
            bare = pixels;
        }
    }
    let bare = bare.expect("off picture");

    let mut on_ms = Vec::new();
    let mut shown = None;
    for index in 0..4 {
        let _ = window.pump();
        let start = Instant::now();
        let pixels = renderer
            .draw_with_overlay(&world, &camera, &plate, index == 3, false)
            .expect("draw on");
        on_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        if index == 3 {
            shown = pixels;
        }
    }
    let shown = shown.expect("on picture");
    let changed = corner_diff(&bare, &shown, width, height);
    assert!(
        changed > 20,
        "overlay changed {changed} pixels in the bottom-right corner"
    );

    let out =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../genos-debug/fps-overlay");
    std::fs::create_dir_all(out.join("shots")).expect("shot dir");
    let path = out.join("shots").join("overlay.png");
    png::write_png(&path, width, height, &shown).expect("png");
    eprintln!(
        "fps overlay draw: off {:.2} ms, on {:.2} ms, corner pixels {changed}, shot {}",
        mean_ms(&off_ms),
        mean_ms(&on_ms),
        path.display()
    );
    let off = mean_ms(&off_ms);
    let on = mean_ms(&on_ms);
    assert!(
        on < off * 1.5 + 5.0,
        "overlay draw {on:.2} ms against {off:.2} ms off"
    );
}
