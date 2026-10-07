//! Every surface type is opaque through the same visibility path: a lamp on the far
//! side of a floor, roof, wall, box or cylinder lights nothing on the near side,
//! neither directly nor through a bounce.

use genos_render::{Renderer, World};
use genos_scene::{viewport_uv, Camera, Ceiling, Floor, Light, Scene, Shape, Solid, Vec3, Wall};
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

fn draw(window: &mut Window, renderer: &mut Renderer, world: &World, camera: &Camera) -> Vec<u8> {
    let _ = window.pump();
    renderer
        .draw(world, camera, true)
        .expect("draw")
        .expect("readback")
}

fn sample(pixels: &[u8], width: u32, height: u32, camera: &Camera, world: [f32; 3]) -> f32 {
    let aspect = width as f32 / height as f32;
    let uv = viewport_uv(camera, aspect, world).expect("point is on screen");
    assert!(
        (0.01..0.99).contains(&uv[0]) && (0.01..0.99).contains(&uv[1]),
        "{world:?} is off screen at {uv:?}"
    );
    let x = (uv[0] * width as f32).round() as i32;
    let y = (uv[1] * height as f32).round() as i32;
    let mut sum = 0.0;
    let mut count = 0.0;
    for dy in -2..=2 {
        for dx in -2..=2 {
            let (px, py) = (x + dx, y + dy);
            if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
                continue;
            }
            let i = ((py as u32 * width + px as u32) * 4) as usize;
            sum += pixels[i] as f32 + pixels[i + 1] as f32 + pixels[i + 2] as f32;
            count += 1.0;
        }
    }
    sum / count
}

fn lamp(x: f32, y: f32, z: f32) -> Light {
    Light {
        position: Vec3::new(x, y, z),
        color: [8.0, 8.0, 8.0],
        direction: Vec3::ZERO,
    }
}

fn wall(x: f32, z: f32, width: f32, depth: f32, height: f32) -> Wall {
    Wall {
        position: Vec3::new(x, 0.0, z),
        half_x: width * 0.5,
        half_z: depth * 0.5,
        height,
        color: [1.0, 1.0, 1.0],
        absorption: 0.0,
        reflectance: -1.0,
        color_mix: -1.0,
    }
}

fn solid(shape: Shape, size: f32, height: f32) -> Solid {
    Solid {
        shape,
        position: Vec3::new(0.0, 0.0, 0.0),
        size,
        height,
        color: [1.0, 1.0, 1.0],
        absorption: 0.0,
        reflectance: -1.0,
        color_mix: -1.0,
    }
}

fn floor(half: f32) -> Floor {
    Floor {
        position: Vec3::new(0.0, 0.0, 0.0),
        half_x: half,
        half_z: half,
        color: [1.0, 1.0, 1.0],
    }
}

/// Draw the scene with the hidden lamp and with a control lamp the camera side sees.
/// Every sample must be black with the hidden lamp and the control must light the first.
fn check(label: &str, scene: Scene, hidden: Light, control: Light, camera: &Camera, points: &[[f32; 3]]) {
    let (_gpu, mut window, mut renderer) = open();
    let width = renderer.width();
    let height = renderer.height();
    let mut dark = scene.clone();
    dark.lights = vec![hidden];
    let dark_px = draw(&mut window, &mut renderer, &World::from_scene(dark), camera);
    if let Ok(dir) = std::env::var("OCC_DUMP") {
        let mut rgb = Vec::new();
        for px in dark_px.chunks(4) {
            rgb.extend_from_slice(&[px[2], px[1], px[0]]);
        }
        let mut file = format!("P6\n{width} {height}\n255\n").into_bytes();
        file.extend(rgb);
        std::fs::write(format!("{dir}/{label}.ppm"), file).unwrap();
    }
    for point in points {
        let value = sample(&dark_px, width, height, camera, *point);
        assert!(value < 3.0, "{label}: a hidden lamp lit {point:?} ({value})");
    }
    let mut lit = scene;
    lit.lights = vec![control];
    let lit_px = draw(&mut window, &mut renderer, &World::from_scene(lit), camera);
    let value = sample(&lit_px, width, height, camera, points[0]);
    assert!(value > 60.0, "{label}: the control lamp left {:?} dark ({value})", points[0]);
}

#[test]
fn a_lamp_under_the_floor_lights_nothing_above_it() {
    let scene = genos_scene::load_path(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/camera/scene.rhai"
    )))
    .expect("shipped scene");
    let mut camera = Camera::new(0.0, -4.0, std::f32::consts::PI);
    camera.pitch = -0.3;
    check(
        "floor",
        scene,
        lamp(0.0, -0.5, 0.0),
        lamp(0.0, 3.0, 0.0),
        &camera,
        &[
            [1.5, 0.0, -1.5],
            [-1.2, 0.0, -1.8],
            [0.0, 0.6, -0.76],
            [1.79, 0.6, -1.4],
            [-3.0, 0.6, 1.29],
            [0.0, 1.6, 4.79],
        ],
    );
}

#[test]
fn a_lamp_sealed_in_walls_under_a_roof_lights_nothing_outside() {
    let walls = vec![
        wall(0.0, -1.1, 2.4, 0.2, 2.6),
        wall(0.0, 1.1, 2.4, 0.2, 2.6),
        wall(-1.1, 0.0, 0.2, 2.4, 2.6),
        wall(1.1, 0.0, 0.2, 2.4, 2.6),
    ];
    let scene = Scene {
        floor: floor(6.0),
        walls,
        solids: Vec::new(),
        lights: Vec::new(),
        ceiling: Some(Ceiling {
            height: 2.6,
            color: [1.0, 1.0, 1.0],
        }),
    };
    let mut camera = Camera::new(0.4, -4.5, std::f32::consts::PI);
    camera.pitch = 0.1;
    check(
        "wall and roof",
        scene,
        lamp(0.0, 1.3, 0.0),
        lamp(0.0, 1.3, -3.0),
        &camera,
        &[
            [0.3, 1.3, -1.21],
            [-1.7, 0.0, 0.5],
            [1.7, 0.0, 0.5],
            [0.0, 2.6, -2.5],
        ],
    );
}

#[test]
fn a_lamp_above_the_roof_lights_nothing_under_it() {
    let scene = Scene {
        floor: floor(6.0),
        walls: vec![wall(0.0, 3.0, 6.0, 0.4, 2.6)],
        solids: Vec::new(),
        lights: Vec::new(),
        ceiling: Some(Ceiling {
            height: 2.6,
            color: [1.0, 1.0, 1.0],
        }),
    };
    let mut camera = Camera::new(0.0, -3.0, std::f32::consts::PI);
    camera.pitch = 0.05;
    check(
        "roof",
        scene,
        lamp(0.0, 4.0, 0.0),
        lamp(0.0, 2.0, 0.0),
        &camera,
        &[[0.0, 1.3, 2.79], [-0.8, 0.0, 1.5], [0.0, 2.6, 0.5]],
    );
}

#[test]
fn a_lamp_inside_a_box_lights_nothing_outside() {
    let scene = Scene {
        floor: floor(6.0),
        walls: vec![wall(0.0, 3.0, 6.0, 0.4, 3.0)],
        solids: vec![solid(Shape::Square, 1.5, 1.2)],
        lights: Vec::new(),
        ceiling: None,
    };
    let mut camera = Camera::new(0.0, -4.0, std::f32::consts::PI);
    camera.pitch = -0.2;
    check(
        "box",
        scene,
        lamp(0.0, 0.6, 0.0),
        lamp(0.0, 3.0, -1.5),
        &camera,
        &[[1.5, 0.0, -1.5], [-1.4, 0.0, -1.0], [0.0, 1.0, 2.79]],
    );
}

#[test]
fn a_lamp_inside_a_cylinder_lights_nothing_outside_even_over_its_cap() {
    let scene = Scene {
        floor: floor(6.0),
        walls: vec![wall(2.0, 0.0, 0.4, 6.0, 3.0)],
        solids: vec![solid(Shape::Circle, 1.4, 1.2)],
        lights: Vec::new(),
        ceiling: None,
    };
    let mut camera = Camera::new(-0.5, -4.0, 2.7);
    camera.pitch = 0.0;
    check(
        "cylinder",
        scene,
        lamp(0.0, 1.1, 0.0),
        lamp(0.0, 2.2, 0.0),
        &camera,
        &[[1.79, 2.0, 0.0], [1.79, 1.6, -0.6], [1.0, 0.0, -1.2]],
    );
}
