#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! A dim lamp walks out from behind a wall. The bright lamp stays, so the
//! mover is the cascade sample. The shadow on the floor must move toward the
//! new pose within a few frames, and the first frame must not already be the
//! new picture.

use genos_render::{DebugView, Renderer, ViewMode, World};
use genos_scene::{AmbientMode, Camera, LampPickMode, LightingPreset, LightingSettings, Scene};
use genos_window::Window;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const WARMUP: u32 = 24;
const FOLLOW_FRAMES: usize = 4;
/// First displayed frame still shows the previous mean. A new key would
/// already match the new pose.
const POP_LIMIT: f64 = 0.45;
/// Four frames later. A 4096-count mean is still on the old shadow.
const FOLLOW_AT: f64 = 0.35;

fn open() -> (
    std::sync::MutexGuard<'static, ()>,
    genos_window::Lifecycle<Renderer>,
) {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
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

fn settings(restir: bool) -> LightingSettings {
    let mut settings = LightingSettings::preset(LightingPreset::Fallback);
    settings.direct.max_lights = 0;
    settings.direct.refresh_frames = 1;
    settings.direct.resolution_scale = 1;
    settings.sky = false;
    settings.ambient.mode = AmbientMode::Off;
    settings.shadows.enabled = true;
    settings.gi.enabled = false;
    settings.bounce.count = 0;
    settings.probes.enabled = false;
    settings.world_cache.enabled = false;
    settings.lamp_pick.mode = LampPickMode::Off;
    settings.lamp_pick.radius = 0;
    settings.compose_reuse = false;
    settings.restir_di = restir;
    settings
}

fn lamp(x: f32, y: f32, z: f32, color: [f32; 3]) -> genos_scene::Light {
    genos_scene::Light {
        position: genos_scene::Vec3::new(x, y, z),
        color,
        direction: genos_scene::Vec3::ZERO,
    }
}

fn world(mover: Option<[f32; 3]>) -> World {
    // The stationary lamp stays brighter at the graded floor, so the mover is
    // the cascade sample. Its colour is still high enough that the opened
    // shadow is larger than one disc sample of the stationary lamp.
    let mut lights = vec![lamp(0.4, 2.2, 2.8, [6.0, 6.0, 6.0])];
    if let Some(at) = mover {
        lights.push(lamp(at[0], at[1], at[2], [2.0, 2.0, 2.0]));
    }
    World::from_scene({
        let mut __scene = Scene::from_lists(
            genos_scene::Floor {
                position: genos_scene::Vec3::ZERO,
                half_x: 8.0,
                half_z: 8.0,
                color: [0.72, 0.72, 0.72],
            },
            vec![genos_scene::Wall {
                position: genos_scene::Vec3::new(0.0, 0.0, 1.0),
                half_x: 2.0,
                half_z: 0.05,
                height: 1.5,
                base: 0.0,
                color: [0.62, 0.62, 0.62],
                absorption: 0.0,
                reflectance: 0.8,
                color_mix: -1.0,
            }],
            Vec::new(),
            lights,
        );
        __scene.ceiling = None;
        __scene.sky = None;
        __scene.lighting = settings(true);
        __scene.debug = genos_scene::DebugSettings::default();
        __scene.fps_counter = genos_scene::FpsCounterSettings::default();
        __scene.event_loop = genos_scene::EventLoopSettings::default();
        __scene.reflections = genos_scene::ReflectionSettings::default();
        __scene.hdr = genos_scene::HdrSettings::for_preset(LightingPreset::High);
        __scene.fog = genos_scene::FogSettings::default();
        __scene.menu = genos_scene::MenuSettings::default();
        __scene.particles = genos_scene::ParticleSettings::default();
        __scene
    })
}

fn camera() -> Camera {
    let mut camera = Camera::opening();
    camera.set_pose(genos_scene::Vec3::new(0.0, 1.8, 5.0), 0.0, -0.55);
    camera
}

/// Both lamp poses sit in the 2–4 m shell of this floor point, inside the
/// inner nine tenths, so the cascade key does not change and the coarser
/// shell is not blended in.
fn same_shell(point: [f32; 3], hidden: [f32; 3], shown: [f32; 3]) -> bool {
    let span = |lamp: [f32; 3]| {
        let d = [
            point[0] - lamp[0],
            point[1] - lamp[1],
            point[2] - lamp[2],
        ];
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    };
    let ok = |dist: f32| (2.15..=3.50).contains(&dist);
    ok(span(hidden)) && ok(span(shown))
}

fn n32(i: u32) -> f32 {
    f32::from(u16::try_from(i).unwrap_or(0))
}

fn pixel_index(v: f64, limit: u32) -> Option<usize> {
    if !v.is_finite() {
        return None;
    }
    let want = v.round();
    let mut i = 0u32;
    while i < limit {
        if (f64::from(i) - want).abs() < 0.5 {
            return usize::try_from(i).ok();
        }
        i += 1;
    }
    None
}

fn luma(rgb: &[f64]) -> Vec<f64> {
    let pixels = rgb.len() / 3;
    let mut out = Vec::with_capacity(pixels);
    for pixel in 0..pixels {
        let i = pixel.saturating_mul(3);
        let r = rgb.get(i).copied().unwrap_or(0.0);
        let g = rgb.get(i.saturating_add(1)).copied().unwrap_or(0.0);
        let b = rgb.get(i.saturating_add(2)).copied().unwrap_or(0.0);
        out.push(0.2126 * r + 0.7152 * g + 0.0722 * b);
    }
    out
}

fn grab(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
) -> Vec<f64> {
    let _ = window.pump();
    renderer
        .draw(world, camera, true)
        .expect("draw")
        .expect("readback");
    luma(&renderer.read_scene().expect("scene radiance").rgb)
}

/// Mean of several exact frames. One disc sample moves the stationary lamp.
/// The mean is the shadow the live frames are graded against.
fn average(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
) -> Vec<f64> {
    const N: usize = 8;
    for _ in 0..2 {
        let _ = grab(window, renderer, world, camera);
    }
    let mut acc = Vec::new();
    for _ in 0..N {
        let frame = grab(window, renderer, world, camera);
        if acc.is_empty() {
            acc = frame;
            continue;
        }
        for (slot, value) in acc.iter_mut().zip(frame) {
            *slot += value;
        }
    }
    let scale = 1.0 / f64::from(u32::try_from(N).unwrap_or(1));
    for value in &mut acc {
        *value *= scale;
    }
    acc
}

fn progress(live: &[f64], before: &[f64], after: &[f64], pixels: &[usize]) -> f64 {
    let mut sum = 0.0;
    for &index in pixels {
        let old = before.get(index).copied().unwrap_or(0.0);
        let new = after.get(index).copied().unwrap_or(old);
        let now = live.get(index).copied().unwrap_or(old);
        let span = new - old;
        if span.abs() > 1.0e-6 {
            sum += (now - old) / span;
        }
    }
    let count = u32::try_from(pixels.len()).unwrap_or(1).max(1);
    sum / f64::from(count)
}

#[test]
fn the_wall_blocks_the_hidden_lamp() {
    let n = [0.0, 1.0, 0.0];
    let y = |mover: Option<[f32; 3]>, at: [f32; 3]| {
        genos_render::luminance(genos_render::direct_light_samples(&world(mover).scene, at, n, 4))
    };
    let mut blocked = 0u32;
    let mut opened = 0u32;
    for iz in 0u32..8 {
        for ix in 0u32..8 {
            let at = [-0.14 + n32(ix) * 0.04, 0.02, 1.40 + n32(iz) * 0.04];
            if !same_shell(at, [0.0, 3.0, 0.0], [0.0, 3.0, 3.2]) {
                continue;
            }
            let bright = y(None, at);
            let hidden = y(Some([0.0, 3.0, 0.0]), at);
            let shown = y(Some([0.0, 3.0, 3.2]), at);
            if hidden - bright < 0.05 {
                blocked += 1;
            }
            if shown - bright > 1.0 {
                opened += 1;
            }
        }
    }
    assert!(
        blocked >= 30 && opened >= 30,
        "the fixture does not hide then show the lamp ({blocked} hidden, {opened} shown)"
    );
}

#[test]
fn a_moved_lamp_shadow_follows_without_popping() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let camera = camera();
    let hidden_at = [0.0, 3.0, 0.0];
    let shown_at = [0.0, 3.0, 3.2];
    let hidden = world(Some(hidden_at));
    let shown = world(Some(shown_at));
    let bright_only = world(None);
    renderer.set_debug_view(DebugView {
        mode: ViewMode::Direct,
        lamp: 0,
    });
    // Exact pictures first, each pose settled on its own, so a later live
    // frame is graded against the shadow rather than against a stale mean.
    renderer.set_lighting_settings(settings(false));
    let exact_old = average(window, renderer, &hidden, &camera);
    let exact_new = average(window, renderer, &shown, &camera);
    let base = average(window, renderer, &bright_only, &camera);
    renderer.set_lighting_settings(settings(true));
    let mut live_old = Vec::new();
    for _ in 0..WARMUP {
        live_old = grab(window, renderer, &hidden, &camera);
    }
    let mut live = Vec::with_capacity(FOLLOW_FRAMES);
    for _ in 0..FOLLOW_FRAMES {
        live.push(grab(window, renderer, &shown, &camera));
    }
    let aspect = n32(WIDTH) / n32(HEIGHT);
    let mut opened = Vec::new();
    let mut seen = Vec::new();
    let mut sum_base = 0.0;
    let mut sum_old = 0.0;
    let mut sum_new = 0.0;
    let mut sum_held = 0.0;
    for iz in 0u32..16 {
        for ix in 0u32..16 {
            let point = [-0.16 + n32(ix) * 0.02, 0.0, 1.40 + n32(iz) * 0.02];
            if !same_shell(point, hidden_at, shown_at) {
                continue;
            }
            let Some(uv) = genos_scene::viewport_uv(&camera, aspect, point) else {
                continue;
            };
            let Some(px) = pixel_index(f64::from(uv[0]) * f64::from(WIDTH), WIDTH) else {
                continue;
            };
            let Some(py) = pixel_index(f64::from(uv[1]) * f64::from(HEIGHT), HEIGHT) else {
                continue;
            };
            let index = py
                .saturating_mul(usize::try_from(WIDTH).unwrap_or(0))
                .saturating_add(px);
            if seen.contains(&index) {
                continue;
            }
            seen.push(index);
            let old = exact_old.get(index).copied().unwrap_or(0.0);
            let new = exact_new.get(index).copied().unwrap_or(0.0);
            let floor = base.get(index).copied().unwrap_or(0.0);
            let held = live_old.get(index).copied().unwrap_or(99.0);
            sum_base += floor;
            sum_old += old;
            sum_new += new;
            sum_held += held;
            // The far pose is the wall's penumbra, not a black shadow. The near
            // pose is lit. The live frame before the move matches that penumbra.
            if new - old > 0.50 && (held - old).abs() < 0.20 {
                opened.push(index);
            }
        }
    }
    let n = f64::from(u32::try_from(seen.len().max(1)).unwrap_or(1));
    assert!(
        opened.len() >= 30,
        "no opened shadow: {} of {} (base {:.3} old {:.3} new {:.3} held {:.3})",
        opened.len(),
        seen.len(),
        sum_base / n,
        sum_old / n,
        sum_new / n,
        sum_held / n
    );
    let first = live.first().expect("frame 1");
    let last = live.get(FOLLOW_FRAMES - 1).expect("frame 4");
    let at_one = progress(first, &live_old, &exact_new, &opened);
    let at_four = progress(last, &live_old, &exact_new, &opened);
    eprintln!(
        "cascade move: {pixels} pixels, frame1 {at_one:.3}, frame4 {at_four:.3}",
        pixels = opened.len()
    );
    assert!(
        at_one < POP_LIMIT,
        "frame 1 already matches the new shadow ({at_one:.3}), a history reset"
    );
    assert!(
        at_four > FOLLOW_AT,
        "frame 4 is still the old shadow ({at_four:.3})"
    );
    assert!(
        at_four > at_one + 0.20,
        "the shadow did not keep moving ({at_one:.3} then {at_four:.3})"
    );
}

