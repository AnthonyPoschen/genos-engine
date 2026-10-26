//! The presented frame writes the reflection grade and shades a water quad.
//!
//! An empty reflector list does not enter the pass. This scene has a mirror,
//! a portal, and water, on the Low preset, so the stored ray count is 0.

#![expect(clippy::as_conversions, reason = "pixel bytes and extents are the API's integers")]
#![expect(clippy::cast_possible_truncation, reason = "the picture is 1280 by 720")]
#![expect(clippy::cast_precision_loss, reason = "a pixel coordinate is a small integer")]
#![expect(clippy::cast_sign_loss, reason = "a pixel coordinate is checked before the cast")]
#![expect(clippy::expect_used, reason = "a failed fixture aborts the test")]
#![expect(
    clippy::redundant_closure_for_method_calls,
    reason = "the poison handler matches the other frame tests"
)]

use genos_math::optics::{self, WATER_IOR};
use genos_math::Vec3;
use genos_render::{
    frame_ray_count, lighting_reflection_preset, read_frame_grade, shade_picture, LightingPreset,
    LightingSettings,
    PictureFault, Reflector, ReflectorKind, Renderer, World,
};
use genos_scene::{viewport_uv, Camera, Floor, ReflectionSettings, Scene, Vec3 as SceneVec3};
use genos_window::Window;

fn open() -> (std::sync::MutexGuard<'static, ()>, Window, Renderer) {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = GPU.lock().unwrap_or_else(|err| err.into_inner());
    let mut window = Window::open_proof(1280, 720).expect("proof window");
    let frame = window.pump();
    let renderer =
        Renderer::open(window.display, window.surface, frame.width, frame.height).expect("renderer");
    (guard, window, renderer)
}

fn world(preset: LightingPreset) -> World {
    let mut world = World::from_scene(Scene {
        floor: Floor {
            position: SceneVec3::new(0.0, 0.0, 0.0),
            half_x: 4.0,
            half_z: 4.0,
            color: [0.6, 0.6, 0.6],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights: Vec::new(),
        ceiling: None,
        sky: None,
        lighting: LightingSettings::preset(preset),
        debug: genos_scene::DebugSettings::default(),
        fps_counter: genos_scene::FpsCounterSettings::default(),
        event_loop: genos_scene::EventLoopSettings::default(),
        reflections: ReflectionSettings::for_preset(match preset {
            LightingPreset::Potato => genos_math::optics::ReflectionPreset::Potato,
            LightingPreset::Low => genos_math::optics::ReflectionPreset::Low,
            LightingPreset::Medium => genos_math::optics::ReflectionPreset::Medium,
            LightingPreset::DayCycle | LightingPreset::Gi1Bounce | LightingPreset::High => {
                genos_math::optics::ReflectionPreset::High
            }
        }),
        menu: genos_scene::MenuSettings::default(),
    });
    world.reflectors.push(Reflector {
        kind: ReflectorKind::Mirror,
        origin: [0.0, 1.0, 0.0],
        normal: [0.0, 0.0, 1.0],
        portal_to: [0.0; 3],
    });
    world.reflectors.push(Reflector {
        kind: ReflectorKind::Portal,
        origin: [2.0, 1.0, 0.0],
        normal: [1.0, 0.0, 0.0],
        portal_to: [10.0, 0.0, 0.0],
    });
    world.reflectors.push(Reflector {
        kind: ReflectorKind::Water,
        origin: [0.0, 1.0, 0.0],
        normal: [0.0, 1.0, 0.0],
        portal_to: [0.0; 3],
    });
    world
}

fn camera() -> Camera {
    let mut camera = Camera::new(0.0, 2.0, 0.0);
    camera.position = SceneVec3::new(0.0, 3.0, 2.0);
    camera.pitch = -0.9;
    camera
}

#[test]
fn the_frame_grades_low_at_zero_rays_and_shades_water() {
    let (_guard, mut window, mut renderer) = open();
    let mut scene = world(LightingPreset::Low);
    let camera = camera();
    let _ = window.pump();
    let pixels = renderer
        .draw(&scene, &camera, true)
        .expect("draw")
        .expect("readback");
    let grade = renderer.reflection_grade().expect("grade");
    assert!(grade.ran, "the composite did not run");
    let preset = lighting_reflection_preset(LightingPreset::Low);
    assert_eq!(grade.rays, frame_ray_count(preset));
    assert_eq!(grade.rays, 0);
    assert_eq!(grade.views, 1);
    let cpu = shade_picture(0, PictureFault::default());
    assert_eq!(grade.color.len(), cpu.color.len());
    let mut worst = 0.0f32;
    for (gpu, reference) in grade.color.iter().zip(cpu.color.iter()) {
        for channel in 0..3 {
            worst = worst.max((gpu[channel] - reference[channel]).abs());
        }
    }
    assert!(worst < 1.0e-3, "{worst}");
    let headless = read_frame_grade(preset, 0).expect("headless grade");
    assert_eq!(headless.rays, grade.rays);
    assert_eq!(headless.views, grade.views);
    assert_eq!(headless.color, grade.color);

    let shaded = optics::water_shade(
        optics::fresnel_dielectric(WATER_IOR, 1.0),
        Vec3::new(0.72, 0.68, 0.55),
        Vec3::new(0.08, 0.22, 0.72),
        0.35,
        1.0,
    );
    let aspect = renderer_aspect(&pixels);
    let uv = viewport_uv(&camera, aspect, [0.0, 1.0, 0.0]).expect("water is on screen");
    let width = 1280u32;
    let height = (pixels.len() / 4 / width as usize) as u32;
    let x = (uv[0] * width as f32).round() as i32;
    let y = (uv[1] * height as f32).round() as i32;
    assert!(x >= 0 && y >= 0 && (x as u32) < width && (y as u32) < height);
    let index = ((y as u32 * width + x as u32) * 4) as usize;
    let got = [
        f32::from(pixels[index + 2]) / 255.0,
        f32::from(pixels[index + 1]) / 255.0,
        f32::from(pixels[index]) / 255.0,
    ];
    for channel in 0..3 {
        let expect = [shaded.x, shaded.y, shaded.z][channel];
        assert!((got[channel] - expect).abs() < 0.03, "{got:?} {expect}");
    }

    scene.scene.lighting = LightingSettings::preset(LightingPreset::Potato);
    scene.scene.reflections =
        ReflectionSettings::for_preset(genos_math::optics::ReflectionPreset::Potato);
    let _ = window.pump();
    renderer.draw(&scene, &camera, true).expect("potato draw");
    let potato = renderer.reflection_grade().expect("potato grade");
    assert!(potato.ran);
    assert_eq!(potato.rays, 0);
    assert_eq!(potato.views, 0);
}

fn renderer_aspect(pixels: &[u8]) -> f32 {
    let width = 1280.0f32;
    let height = (pixels.len() / 4) as f32 / width;
    width / height
}
