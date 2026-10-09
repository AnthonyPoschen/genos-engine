//! Presented-picture antialiasing. The checks read the Vulkan draw, not a CPU bake.

use genos_render::{identity_pose, Antialias, Bounds, DrawKind, Object, Renderer, World};
use genos_scene::{viewport_uv, Camera, Floor, Light, Scene, Vec3};
use genos_window::Window;

fn open() -> (std::sync::MutexGuard<'static, ()>, Window, Renderer) {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = GPU.lock().unwrap_or_else(|err| err.into_inner());
    let mut window = Window::open_proof(1280, 720).expect("proof window");
    let frame = window.pump();
    let renderer = Renderer::open(window.display, window.surface, frame.width, frame.height)
        .expect("renderer");
    (guard, window, renderer)
}

fn silhouette_world() -> World {
    let mut world = World::from_scene(Scene {
        floor: Floor {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 8.0,
            half_z: 8.0,
            color: [0.0, 0.0, 0.0],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights: vec![Light {
            position: Vec3::new(0.0, 1.5, 1.5),
            color: [1.0, 1.0, 1.0],

            direction: Vec3::ZERO,
        }],
        ceiling: None,
        sky: None,
    });
    world.objects.push(Object {
        hidden: false,
        affects_light: false,
        bounds: Bounds {
            center: [0.0, 1.2, 0.0],
            half: [3.0, 3.0, 3.0],
        },
        kind: DrawKind::Mesh {
            vertices: vec![[-1.2, 0.3, 0.0], [1.2, 0.3, 0.0], [0.0, 2.4, 0.0]],
            color: [1.0, 1.0, 1.0],
            pose: identity_pose(),
            emission: [0.0; 3],
        },
    });
    world
}

fn draw(window: &mut Window, renderer: &mut Renderer, world: &World, camera: &Camera) -> Vec<u8> {
    let _ = window.pump();
    renderer
        .draw(world, camera, true)
        .expect("draw")
        .expect("readback")
}

fn project(camera: &Camera, width: u32, height: u32, world: [f32; 3]) -> (i32, i32) {
    let aspect = width as f32 / height as f32;
    let uv = viewport_uv(camera, aspect, world).expect("point is on screen");
    let x = (uv[0] * width as f32).round() as i32;
    let y = (uv[1] * height as f32).round() as i32;
    (x, y)
}

fn channels(pixels: &[u8], width: u32, x: i32, y: i32) -> [u8; 3] {
    let index = ((y as u32 * width + x as u32) * 4) as usize;
    [pixels[index], pixels[index + 1], pixels[index + 2]]
}

fn step_between(a: [u8; 3], b: [u8; 3]) -> u32 {
    a[0].abs_diff(b[0])
        .max(a[1].abs_diff(b[1]))
        .max(a[2].abs_diff(b[2])) as u32
}

fn max_adjacent_step(pixels: &[u8], width: u32, x0: i32, y0: i32, x1: i32, y1: i32) -> u32 {
    let mut max = 0u32;
    for y in y0..y1 {
        for x in x0..x1 {
            let here = channels(pixels, width, x, y);
            if x + 1 < x1 {
                max = max.max(step_between(here, channels(pixels, width, x + 1, y)));
            }
            if y + 1 < y1 {
                max = max.max(step_between(here, channels(pixels, width, x, y + 1)));
            }
        }
    }
    max
}

fn window_around(x: i32, y: i32, width: u32, height: u32, radius: i32) -> (i32, i32, i32, i32) {
    let x0 = (x - radius).clamp(1, width as i32 - 2);
    let y0 = (y - radius).clamp(1, height as i32 - 2);
    let x1 = (x + radius).clamp(x0 + 2, width as i32 - 1);
    let y1 = (y + radius).clamp(y0 + 2, height as i32 - 1);
    (x0, y0, x1, y1)
}

fn neighborhood_step(pixels: &[u8], width: u32, x: i32, y: i32) -> u32 {
    max_adjacent_step(pixels, width, x - 1, y - 1, x + 2, y + 2)
}

fn max_delta(a: &[u8], b: &[u8], width: u32, x0: i32, y0: i32, x1: i32, y1: i32) -> u32 {
    let mut max = 0u32;
    for y in y0..y1 {
        for x in x0..x1 {
            max = max.max(step_between(
                channels(a, width, x, y),
                channels(b, width, x, y),
            ));
        }
    }
    max
}

fn differs(a: &[u8], b: &[u8], width: u32, x0: i32, y0: i32, x1: i32, y1: i32) -> bool {
    for y in y0..y1 {
        for x in x0..x1 {
            let index = ((y as u32 * width + x as u32) * 4) as usize;
            if a[index..index + 4] != b[index..index + 4] {
                return true;
            }
        }
    }
    false
}

#[test]
fn the_picture_switches_filters_and_off_matches_again() {
    let (_gpu, mut window, mut renderer) = open();
    let width = renderer.width();
    let height = renderer.height();
    let world = silhouette_world();
    let camera = Camera::new(0.0, 4.0, 0.0);
    assert_eq!(renderer.antialias(), Antialias::Off);

    let _warmup = draw(&mut window, &mut renderer, &world, &camera);
    let off = draw(&mut window, &mut renderer, &world, &camera);

    let edge = [-0.6, 1.35, 0.0];
    let (edge_x, edge_y) = project(&camera, width, height, edge);
    let (x0, y0, x1, y1) = window_around(edge_x, edge_y, width, height, 18);
    let off_step = max_adjacent_step(&off, width, x0, y0, x1, y1);

    let face = [0.0, 1.0, 0.0];
    let (face_x, face_y) = project(&camera, width, height, face);
    let face_step = neighborhood_step(&off, width, face_x, face_y);
    let face_pixel = channels(&off, width, face_x, face_y);
    let apart = (face_x - edge_x).abs().max((face_y - edge_y).abs());
    println!(
        "aa picture off_step={off_step} face_step={face_step} face={face_pixel:?} edge=({edge_x},{edge_y}) face=({face_x},{face_y}) apart={apart} size={width}x{height}"
    );
    assert!(
        off_step >= 16,
        "the silhouette is not a hard step: {off_step} at ({edge_x},{edge_y})"
    );
    assert!(
        face_step <= 2,
        "the face interior is not flat: step {face_step} pixel {face_pixel:?}"
    );
    let face_sum = face_pixel
        .iter()
        .map(|channel| *channel as u32)
        .sum::<u32>();
    assert!(face_sum >= 40, "the face is dark: {face_pixel:?}");
    assert!(apart >= 24, "the face sample sits on the silhouette");
    assert!(face_x > 8 && face_y > 8 && face_x < width as i32 - 8 && face_y < height as i32 - 8);

    renderer.set_antialias(Antialias::Fxaa);
    let fxaa = draw(&mut window, &mut renderer, &world, &camera);
    renderer.set_antialias(Antialias::Ssaa);
    let ssaa = draw(&mut window, &mut renderer, &world, &camera);
    renderer.set_antialias(Antialias::Off);
    let again = draw(&mut window, &mut renderer, &world, &camera);

    let fxaa_step = max_adjacent_step(&fxaa, width, x0, y0, x1, y1);
    let ssaa_step = max_adjacent_step(&ssaa, width, x0, y0, x1, y1);
    let fxaa_face = max_delta(
        &off,
        &fxaa,
        width,
        face_x - 2,
        face_y - 2,
        face_x + 3,
        face_y + 3,
    );
    let ssaa_face = max_delta(
        &off,
        &ssaa,
        width,
        face_x - 2,
        face_y - 2,
        face_x + 3,
        face_y + 3,
    );
    println!(
        "aa picture fxaa_step={fxaa_step} ssaa_step={ssaa_step} fxaa_face={fxaa_face} ssaa_face={ssaa_face}"
    );
    assert!(
        fxaa_step < off_step,
        "FXAA did not soften the silhouette: {fxaa_step} vs {off_step}"
    );
    assert!(
        ssaa_step < off_step,
        "SSAA did not soften the silhouette: {ssaa_step} vs {off_step}"
    );
    assert!(fxaa_face <= 8, "FXAA changed the flat face by {fxaa_face}");
    assert!(ssaa_face <= 8, "SSAA changed the flat face by {ssaa_face}");
    assert!(
        differs(&fxaa, &ssaa, width, x0, y0, x1, y1),
        "FXAA and SSAA wrote the same silhouette"
    );
    assert_eq!(
        again, off,
        "off after a filter did not match the first off picture"
    );
}
