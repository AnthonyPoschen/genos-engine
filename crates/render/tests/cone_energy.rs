#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! How much of a lamp's light the cone-traced cascades deliver, against the
//! old path's exact direct light (plan section 18, step 6: cone mode graded
//! 40% to 85% too dark). One lamp over a wide floor, no sky, no bounce: the
//! old path's picture there is the lamp's exact light. Cone mode's picture is
//! matched by drawing the old path with the lamp scaled by k; the k that
//! matches is cone mode's share of the lamp's light, whatever the tone curve.
//! A measurement: `cargo test -p genos-render --test cone_energy -- --ignored --nocapture`.

mod common;

use common::PresentedFrame;
use genos_render::{Renderer, World};
use genos_scene::{AmbientMode, Camera, Floor, Light, LightingPreset, LightingSettings, Scene, Vec3};
use genos_window::Window;

const W: u32 = 320;
const H: u32 = 180;

fn open() -> genos_window::Lifecycle<Renderer> {
    let mut window = Window::open_proof(W, H).expect("proof window");
    let frame = window.pump();
    let (renderer, permit) = Renderer::open(window.display, window.surface, frame.width, frame.height).expect("renderer");
    genos_window::Lifecycle::new(window, renderer, permit)
}

fn scene(scale: f32, cone: bool, height: f32) -> Scene {
    let mut scene = Scene::from_lists(
        Floor { position: Vec3::ZERO, half_x: 30.0, half_z: 30.0, color: [0.8, 0.8, 0.8] },
        Vec::new(),
        Vec::new(),
        vec![Light { position: Vec3::new(0.0, height, 0.0), color: [scale; 3], direction: Vec3::ZERO }],
    );
    scene.ceiling = None;
    scene.sky = None;
    let mut lighting = LightingSettings::preset(LightingPreset::Fallback);
    lighting.cone.enabled = cone;
    lighting.gi.enabled = false;
    lighting.ambient.mode = AmbientMode::Off;
    lighting.bounce.count = 0;
    scene.lighting = lighting;
    scene.hdr = genos_scene::HdrSettings::legacy_bytes();
    scene
}

/// The mean of the middle 8x8 pixels' green channel.
fn middle(frame: &PresentedFrame) -> f64 {
    let mut sum = 0.0;
    for y in H / 2 - 4..H / 2 + 4 {
        for x in W / 2 - 4..W / 2 + 4 {
            let i = usize::try_from((y * W + x) * 4 + 1).unwrap_or(0);
            sum += f64::from(frame.pixels[i]);
        }
    }
    sum / 64.0
}

fn settled(window: &mut Window, renderer: &mut Renderer, world: &World, camera: &Camera, cone: bool) -> f64 {
    let mut value = 0.0;
    for _ in 0..600 {
        let frame = PresentedFrame::from_draw(window, renderer, world, camera);
        value = middle(&frame);
        if !cone || (renderer.cone_field_fine() && renderer.cone_stats().expect("stats").first().is_some_and(|s| s.probes > 0)) {
            break;
        }
    }
    let start = std::time::Instant::now();
    while start.elapsed().as_secs_f32() < 1.0 {
        value = middle(&PresentedFrame::from_draw(window, renderer, world, camera));
    }
    value
}

#[test]
#[ignore = "a measurement, not a gate"]
fn cone_mode_delivers_the_lamps_light() {
    let mut owner = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    for height in [0.25_f32, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0] {
        let mut camera = Camera::opening();
        // Looking straight down at the floor 1.5 m beside the lamp.
        camera.set_pose(Vec3::new(1.5, height + 1.0, 0.01), 0.0, -1.55);
        // The same light on the floor at every height, in the tone curve's middle.
        let base = 0.02 * height * height;
        let cone = settled(window, renderer, &World::from_scene(scene(base, true, height)), &camera, true);
        let mut best = (f64::MAX, 0.0_f32, 0.0);
        for step in 1..=40_u16 {
            let k = 0.05 * f32::from(step);
            let old = settled(window, renderer, &World::from_scene(scene(base * k, false, height)), &camera, false);
            if (old - cone).abs() < best.0 {
                best = ((old - cone).abs(), k, old);
            }
        }
        eprintln!("cone energy: lamp {height} m over the floor: cone mode {cone:.1}; the old path matches it with the lamp at {:.2} of its light ({:.1})", best.1, best.2);
    }
}

/// A sun at 45 degrees over a floor and a wall whose face toward the camera
/// is turned from the sun: that face is lit only by the sunlit floor's
/// bounce. Cone mode against the old path with its bounce (GI on).
fn bounce_scene(scale: f32, cone: bool) -> Scene {
    let mut scene = Scene::from_lists(
        Floor { position: Vec3::ZERO, half_x: 30.0, half_z: 30.0, color: [0.8, 0.8, 0.8] },
        vec![genos_scene::Wall {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 0.1,
            half_z: 4.0,
            height: 3.0,
            base: 0.0,
            color: [0.8, 0.8, 0.8],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }],
        Vec::new(),
        // A sun: its direction is the way its light travels, from +x down to -x.
        vec![Light { position: Vec3::ZERO, color: [scale; 3], direction: Vec3::new(-1.0, -1.0, 0.0) }],
    );
    scene.ceiling = None;
    scene.sky = None;
    let mut lighting = LightingSettings::default();
    lighting.cone.enabled = cone;
    lighting.ambient.mode = AmbientMode::Off;
    scene.lighting = lighting;
    scene.hdr = genos_scene::HdrSettings::legacy_bytes();
    scene
}

#[test]
#[ignore = "a measurement, not a gate"]
fn cone_mode_delivers_the_bounce() {
    let mut owner = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let mut camera = Camera::opening();
    // On the shaded (-x) side, looking at the wall's shaded face (+x).
    camera.set_pose(Vec3::new(-6.0, 1.5, 0.0), std::f32::consts::FRAC_PI_2, 0.0);
    let base = 12.0;
    let cone = settled(window, renderer, &World::from_scene(bounce_scene(base, true)), &camera, true);
    let mut best = (f64::MAX, 0.0_f32, 0.0);
    for step in 1..=60_u16 {
        let k = 0.05 * f32::from(step);
        let old = settled(window, renderer, &World::from_scene(bounce_scene(base * k, false)), &camera, false);
        if (old - cone).abs() < best.0 {
            best = ((old - cone).abs(), k, old);
        }
    }
    eprintln!("cone energy: the wall's shaded face, lit by the floor's bounce: cone mode {cone:.1}; the old path matches it with the sun at {:.2} of its light ({:.1})", best.1, best.2);
}

/// A floor under an open sky and nothing else: lit by the sky alone.
fn sky_scene(scale: f32, cone: bool) -> Scene {
    let mut scene = Scene::from_lists(
        Floor { position: Vec3::ZERO, half_x: 30.0, half_z: 30.0, color: [0.8, 0.8, 0.8] },
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    scene.ceiling = None;
    scene.sky = Some(genos_scene::Sky { color: [scale; 3] });
    let mut lighting = LightingSettings::default();
    lighting.cone.enabled = cone;
    lighting.ambient.mode = AmbientMode::Off;
    scene.lighting = lighting;
    scene.hdr = genos_scene::HdrSettings::legacy_bytes();
    scene
}

#[test]
#[ignore = "a measurement, not a gate"]
fn cone_mode_delivers_the_sky() {
    let mut owner = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(0.0, 2.0, 0.0), 0.0, -1.55);
    let base = 0.3;
    let cone = settled(window, renderer, &World::from_scene(sky_scene(base, true)), &camera, true);
    let mut best = (f64::MAX, 0.0_f32, 0.0);
    for step in 1..=60_u16 {
        let k = 0.05 * f32::from(step);
        let old = settled(window, renderer, &World::from_scene(sky_scene(base * k, false)), &camera, false);
        if (old - cone).abs() < best.0 {
            best = ((old - cone).abs(), k, old);
        }
    }
    eprintln!("cone energy: an open floor lit by the sky: cone mode {cone:.1}; the old path matches it with the sky at {:.2} of its light ({:.1})", best.1, best.2);
}

#[test]
#[ignore = "a measurement, not a gate"]
fn cone_mode_lights_a_ceiling() {
    let mut owner = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let mut camera = Camera::opening();
    // Under a ceiling 3 m up, a lamp at 2 m; looking up at the ceiling 1.5 m beside it.
    camera.set_pose(Vec3::new(1.5, 1.0, 0.01), 0.0, 1.55);
    let make = |scale: f32, cone: bool| {
        let mut s = scene(scale, cone, 2.0);
        s.ceiling = Some(genos_scene::Ceiling { height: 3.0, color: [0.8, 0.8, 0.8] });
        s
    };
    let base = 0.02;
    let cone = settled(window, renderer, &World::from_scene(make(base, true)), &camera, true);
    let mut best = (f64::MAX, 0.0_f32, 0.0);
    for step in 1..=60_u16 {
        let k = 0.05 * f32::from(step);
        let old = settled(window, renderer, &World::from_scene(make(base * k, false)), &camera, false);
        if (old - cone).abs() < best.0 {
            best = ((old - cone).abs(), k, old);
        }
    }
    eprintln!("cone energy: a ceiling over a lamp: cone mode {cone:.1}; the old path matches it with the lamp at {:.2} of its light ({:.1})", best.1, best.2);
}

/// Close to the lamp: the floor 0.3 m beside the point under a lamp 0.3 m or
/// 0.6 m up, where a wide cone crosses the lamp's sphere.
#[test]
#[ignore = "a measurement, not a gate"]
fn cone_mode_delivers_the_lamps_light_up_close() {
    let mut owner = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    for height in [0.3_f32, 0.6] {
        let mut camera = Camera::opening();
        camera.set_pose(Vec3::new(0.3, height + 1.0, 0.01), 0.0, -1.55);
        let d2 = 0.09 + height * height;
        let base = 0.02 * d2;
        let cone = settled(window, renderer, &World::from_scene(scene(base, true, height)), &camera, true);
        let mut best = (f64::MAX, 0.0_f32, 0.0);
        for step in 1..=60_u16 {
            let k = 0.1 * f32::from(step);
            let old = settled(window, renderer, &World::from_scene(scene(base * k, false, height)), &camera, false);
            if (old - cone).abs() < best.0 {
                best = ((old - cone).abs(), k, old);
            }
        }
        eprintln!("cone energy: lamp {height} m up, floor 0.3 m beside it: cone mode {cone:.1}; the old path matches it with the lamp at {:.2} of its light ({:.1})", best.1, best.2);
    }
}
