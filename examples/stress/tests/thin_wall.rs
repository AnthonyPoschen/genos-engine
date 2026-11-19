#![cfg_attr(
    test,
    expect(clippy::expect_used, reason = "a test aborts when a fixture fails")
)]

//! Bounce does not cross a wall thinner than a cell.
//!
//! The lamp is on the west side of a 4 cm wall. After the bounce field
//! settles, the east face stays dark and the west face does not. A parent
//! cell that fails the plane or facing gate must not fill the dark side.

use std::sync::Mutex;

use genos_render::{linear_from_display, luminance, DebugView, Renderer, ViewMode, World};
use genos_scene::{viewport_uv, Camera, Floor, Light, Scene, Vec3, Wall};
use genos_window::Window;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const WARMUP: u32 = 24;

fn open() -> (MutexGuard, genos_window::Lifecycle<Renderer>) {
    static GPU: Mutex<()> = Mutex::new(());
    let guard = GPU
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut window = Window::open_proof(WIDTH, HEIGHT).expect("proof window");
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height)
            .expect("renderer");
    (
        guard,
        genos_window::Lifecycle::new(window, renderer, permit),
    )
}

type MutexGuard = std::sync::MutexGuard<'static, ()>;

fn world() -> World {
    World::from_scene(Scene {
        floor: Floor {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 8.0,
            half_z: 8.0,
            color: [1.0, 1.0, 1.0],
        },
        walls: vec![Wall {
            base: 0.0,
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 0.02,
            half_z: 3.0,
            height: 3.0,
            color: [0.8, 0.8, 0.8],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }],
        solids: Vec::new(),
        lights: vec![Light {
            position: Vec3::new(-2.0, 1.2, 0.0),
            color: [8.0, 8.0, 8.0],
            direction: Vec3::ZERO,
        }],
        ceiling: None,
        sky: None,
        lighting: genos_scene::LightingSettings::default(),
        debug: genos_scene::DebugSettings::default(),
        fps_counter: genos_scene::FpsCounterSettings::default(),
        event_loop: genos_scene::EventLoopSettings::default(),
        reflections: genos_scene::ReflectionSettings::default(),
        hdr: genos_scene::HdrSettings::legacy_bytes(),
        fog: genos_scene::FogSettings::default(),
        particles: genos_scene::ParticleSettings::default(),
        menu: genos_scene::MenuSettings::default(),
    })
}

fn draw(window: &mut Window, renderer: &mut Renderer, world: &World, camera: &Camera) -> Vec<u8> {
    let _ = window.pump();
    renderer
        .draw(world, camera, true)
        .expect("draw")
        .expect("presented image")
}

fn nearest_u32(v: f32, limit: u32) -> Option<u32> {
    match v.partial_cmp(&0.0) {
        Some(std::cmp::Ordering::Less) | None => return None,
        Some(std::cmp::Ordering::Equal | std::cmp::Ordering::Greater) => {}
    }
    let mut n = 0u32;
    let mut left = v + 0.5;
    while left >= 1.0 {
        if n >= limit {
            return None;
        }
        n += 1;
        left -= 1.0;
    }
    if n >= limit { None } else { Some(n) }
}

fn luma_at(pixels: &[u8], camera: &Camera, at: [f32; 3]) -> f64 {
    let aspect = 320.0 / 180.0;
    let uv = viewport_uv(camera, aspect, at).expect("point is on screen");
    let x = nearest_u32(uv[0] * 320.0, WIDTH).expect("x");
    let y = nearest_u32(uv[1] * 180.0, HEIGHT).expect("y");
    let mut sum = 0.0;
    let mut count = 0.0;
    for dy in 0..5u32 {
        for dx in 0..5u32 {
            let Some(px) = x.checked_add(dx).and_then(|v| v.checked_sub(2)) else {
                continue;
            };
            let Some(py) = y.checked_add(dy).and_then(|v| v.checked_sub(2)) else {
                continue;
            };
            if px >= WIDTH || py >= HEIGHT {
                continue;
            }
            let index = usize::try_from((py * WIDTH + px) * 4).expect("pixel");
            let red = linear_from_display(f32::from(pixels[index + 2]) / 255.0);
            let green = linear_from_display(f32::from(pixels[index + 1]) / 255.0);
            let blue = linear_from_display(f32::from(pixels[index]) / 255.0);
            sum += f64::from(luminance([red, green, blue]));
            count += 1.0;
        }
    }
    sum / count
}

#[test]
fn bounce_does_not_cross_a_thin_wall() {
    let (_gpu, mut life) = open();
    let window = &mut *life.window;
    let renderer = &mut *life.device;
    renderer.apply_lighting_preset("high").expect("high preset");
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Bounce,
        lamp: 0,
    });
    let world = world();
    let lit_cam = Camera::new(-2.4, 0.0, std::f32::consts::FRAC_PI_2);
    let shadow_cam = Camera::new(2.4, 0.0, -std::f32::consts::FRAC_PI_2);
    let mut lit = 0.0;
    let mut shadow = 0.0;
    for _ in 0..WARMUP {
        let frame = draw(window, renderer, &world, &lit_cam);
        lit = luma_at(&frame, &lit_cam, [-0.08, 1.2, 0.0]);
    }
    for _ in 0..WARMUP {
        let frame = draw(window, renderer, &world, &shadow_cam);
        shadow = luma_at(&frame, &shadow_cam, [0.08, 1.2, 0.0]);
    }
    assert!(
        lit > shadow * 4.0 && lit > 0.02,
        "bounce crossed the thin wall or the lit face stayed dark: lit {lit} shadow {shadow}"
    );
}
