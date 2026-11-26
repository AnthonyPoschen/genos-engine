#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]

//! In cone mode a metal shows what it reflects (`cone_cascades.specular`):
//! a full metal's diffuse albedo is black, so without the specular light it
//! is drawn black (`docs/systems/lighting/cone-traced-cascades-plan.md`,
//! section 22). `scripts/ci-gate-knockout.sh` knocks out `cone_specular`
//! and this goes red.

mod common;

use common::PresentedFrame;
use genos_render::{identity_pose, Bounds, DrawKind, Object, Renderer, Surface, World, CONE_SPECULAR, MAT_BASE};
use genos_scene::{Camera, Floor, Light, LightingPreset, LightingSettings, Scene, Vec3, Wall};
use genos_window::Window;

/// Frames drawn before a frame is read, and the least real time they take
/// (the bounce keeps a short history that steps by frame time).
const SETTLE: u32 = 3;
const SETTLE_SECONDS: f32 = 2.0;
/// A pixel the specular light changed: by this many display codes or more.
const SPECULAR_CHANGE: u8 = 4;
/// The least pixels of the 320 by 180 frame the metal panel's specular
/// light changes (the panel covers about 6,000).
const SPECULAR_PIXELS: usize = 1000;

fn open() -> (std::sync::MutexGuard<'static, ()>, genos_window::Lifecycle<Renderer>) {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = GPU.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut window = Window::open_proof(320, 180).expect("proof window");
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height).expect("renderer");
    (guard, genos_window::Lifecycle::new(window, renderer, permit))
}

/// A floor, a wall and one lamp in cone mode, and a full-metal panel that
/// faces the camera: untextured, base colour 0.9, roughness 0.3.
fn world() -> World {
    let mut scene = Scene::from_lists(
        Floor { position: Vec3::ZERO, half_x: 4.0, half_z: 4.0, color: [0.8, 0.8, 0.8] },
        vec![Wall {
            position: Vec3::new(0.0, 0.0, -1.0),
            half_x: 2.0,
            half_z: 0.1,
            height: 2.0,
            base: 0.0,
            color: [0.8, 0.7, 0.6],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }],
        Vec::new(),
        vec![Light { position: Vec3::new(0.5, 1.5, 0.5), color: [1.0, 0.9, 0.8], direction: Vec3::ZERO }],
    );
    scene.ceiling = None;
    scene.sky = None;
    let mut lighting = LightingSettings::preset(LightingPreset::Fallback);
    lighting.cone.enabled = true;
    scene.lighting = lighting;
    scene.hdr = genos_scene::HdrSettings::legacy_bytes();
    let mut world = World::from_scene(scene);
    world.surfaces.push(Surface {
        color: [0, 0, 0, 0],
        normal: [0, 0, 0, 0],
        cutoff: 0.0,
        two_sided: false,
        roughness: 0.3,
        metallic: 1.0,
        material_flags: MAT_BASE,
    });
    let (left, right, low, high, z) = (-1.0, 1.0, 0.3, 1.5, -0.5);
    world.objects.push(Object {
        hidden: false,
        affects_light: false,
        bounds: Bounds { center: [0.0, 0.9, z], half: [1.0, 0.6, 0.1] },
        kind: DrawKind::Mesh {
            vertices: vec![[left, low, z], [right, low, z], [right, high, z], [left, low, z], [right, high, z], [left, high, z]],
            normals: Vec::new(),
            texcoords: Vec::new(),
            color: [0.9, 0.9, 0.9],
            colors: Vec::new(),
            pose: identity_pose(),
            emission: [0.0; 3],
            surface: Some(0),
        },
    });
    world
}

fn camera() -> Camera {
    let mut camera = Camera::opening();
    camera.set_pose(Vec3::new(0.0, 1.4, 3.0), 0.0, -0.25);
    camera
}

fn draw(window: &mut Window, renderer: &mut Renderer, world: &World, camera: &Camera) -> PresentedFrame {
    let mut frame = None;
    let started = std::time::Instant::now();
    let mut drawn = 0;
    while drawn < SETTLE || started.elapsed().as_secs_f32() < SETTLE_SECONDS {
        frame = Some(PresentedFrame::from_draw(window, renderer, world, camera));
        drawn += 1;
    }
    frame.expect("a frame")
}

/// Draw until the field with its fine levels is resident and the cascades
/// have run.
fn resident(window: &mut Window, renderer: &mut Renderer, world: &World, camera: &Camera) {
    let mut ready = false;
    for _ in 0..3000 {
        let _ = PresentedFrame::from_draw(window, renderer, world, camera);
        let stats = renderer.cone_stats().expect("cone stats");
        if stats.first().is_some_and(|s| s.probes > 0) && renderer.cone_field_fine() {
            ready = true;
            break;
        }
    }
    assert!(ready, "the cone field never became resident");
}

/// Pixels of `lit` brighter than `dark` by [`SPECULAR_CHANGE`] codes or more
/// in some channel.
fn brighter(lit: &PresentedFrame, dark: &PresentedFrame) -> usize {
    lit.pixels
        .chunks(4)
        .zip(dark.pixels.chunks(4))
        .filter(|(p, q)| (0..3).any(|k| p[k] >= q[k].saturating_add(SPECULAR_CHANGE)))
        .count()
}

/// The metal panel is lit by what it reflects: the presented frame with the
/// specular light on is brighter over the panel than with it off, and the
/// panel's pixels were marched a specular cone. Fails when `cone_specular`
/// is knocked out: the panel stays black either way.
#[test]
fn a_metal_shows_what_it_reflects() {
    let (_gpu, mut owner) = open();
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let mut world = world();
    let camera = camera();
    resident(window, renderer, &world, &camera);
    let on = draw(window, renderer, &world, &camera);
    let marched = renderer.cone_specular_pixels().expect("specular pixels");
    world.scene.lighting.cone.specular = false;
    let off = draw(window, renderer, &world, &camera);
    let unmarched = renderer.cone_specular_pixels().expect("specular pixels");
    let lit = brighter(&on, &off);
    let dimmed = brighter(&off, &on);
    eprintln!("cone specular gate: {lit} pixels brighter with the specular light, {dimmed} dimmer; {marched} specular cones on, {unmarched} off");
    assert!(lit >= SPECULAR_PIXELS, "the specular light brightened {lit} pixels, under {SPECULAR_PIXELS} (guard {CONE_SPECULAR})");
    assert!(usize::try_from(marched).unwrap_or(usize::MAX) >= SPECULAR_PIXELS, "{marched} pixels were marched a specular cone, under {SPECULAR_PIXELS} (guard {CONE_SPECULAR})");
    assert_eq!(unmarched, 0, "{unmarched} specular cones were marched with cone_cascades.specular off (guard {CONE_SPECULAR})");
    assert_eq!(dimmed, 0, "the specular light dimmed {dimmed} pixels (guard {CONE_SPECULAR})");
}
