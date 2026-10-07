//! Sky light: a ray that leaves the scene brings the sky's radiance, and a pixel that
//! meets nothing shows it. Pixel checks use the presented readback.

use genos_render::{Renderer, World};
use genos_scene::{viewport_uv, Camera, Floor, Scene, Sky, Vec3, Wall};
use genos_window::Window;

fn open() -> (Window, Renderer) {
    let mut window = Window::open_proof(1280, 720).expect("proof window");
    let frame = window.pump();
    let renderer = Renderer::open(window.display, window.surface, frame.width, frame.height)
        .expect("renderer");
    (window, renderer)
}

fn open_floor(sky: Option<Sky>, walls: Vec<Wall>) -> World {
    World::from_scene(Scene {
        floor: Floor {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 30.0,
            half_z: 30.0,
            color: [1.0, 1.0, 1.0],
        },
        walls,
        solids: Vec::new(),
        lights: Vec::new(),
        ceiling: None,
        sky,
    })
}

fn draw(window: &mut Window, renderer: &mut Renderer, world: &World, camera: &Camera) -> Vec<u8> {
    let _ = window.pump();
    renderer
        .draw(world, camera, true)
        .expect("draw")
        .expect("readback")
}

fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [f32; 3] {
    let i = ((y * width + x) * 4) as usize;
    [pixels[i + 2] as f32, pixels[i + 1] as f32, pixels[i] as f32]
}

fn sample(pixels: &[u8], width: u32, height: u32, camera: &Camera, world: [f32; 3]) -> [f32; 3] {
    let uv = viewport_uv(camera, width as f32 / height as f32, world).expect("point is on screen");
    let x = (uv[0] * width as f32).round() as u32;
    let y = (uv[1] * height as f32).round() as u32;
    let mut sum = [0.0; 3];
    for dy in 0..5 {
        for dx in 0..5 {
            let p = pixel(pixels, width, x + dx - 2, y + dy - 2);
            for c in 0..3 {
                sum[c] += p[c] / 25.0;
            }
        }
    }
    sum
}

const SKY: [f32; 3] = [0.2, 0.25, 0.3];

#[test]
fn an_open_floor_under_the_sky_sends_back_its_albedo_of_the_sky() {
    let (mut window, mut renderer) = open();
    let (width, height) = (renderer.width(), renderer.height());
    let mut camera = Camera::new(0.0, 0.0, 0.0);
    camera.set_pose(Vec3::new(0.0, 3.0, 4.0), 0.0, -0.6);
    let lit = draw(
        &mut window,
        &mut renderer,
        &open_floor(Some(Sky { color: SKY }), Vec::new()),
        &camera,
    );
    let dark = draw(
        &mut window,
        &mut renderer,
        &open_floor(None, Vec::new()),
        &camera,
    );
    // The floor sees the whole sky: irradiance pi × L, radiance 0.8 / pi of that.
    let floor = sample(&lit, width, height, &camera, [0.0, 0.0, 0.0]);
    for c in 0..3 {
        let want = 0.8 * SKY[c] * 255.0;
        assert!(
            (floor[c] - want).abs() < want * 0.15 + 3.0,
            "floor channel {c}: {} against {want}",
            floor[c]
        );
    }
    let unlit = sample(&dark, width, height, &camera, [0.0, 0.0, 0.0]);
    assert!(
        unlit.iter().all(|c| *c < 2.0),
        "no sky, no light: {unlit:?}"
    );
    // Above the horizon the picture shows the sky itself; without one, black.
    let top = pixel(&lit, width, width / 2, 4);
    for c in 0..3 {
        assert!(
            (top[c] - SKY[c] * 255.0).abs() < 3.0,
            "sky channel {c}: {}",
            top[c]
        );
    }
    assert!(pixel(&dark, width, width / 2, 4).iter().all(|c| *c < 1.0));
}

#[test]
fn a_slab_overhead_shades_the_floor_from_the_sky() {
    let (mut window, mut renderer) = open();
    let (width, height) = (renderer.width(), renderer.height());
    // A raised 6 m square slab 0.5 m thick, 1 m above the floor: under its middle the
    // floor sees sky only in a band about 20 degrees above the horizon.
    let slab = Wall {
        position: Vec3::new(0.0, 0.0, 0.0),
        half_x: 3.0,
        half_z: 3.0,
        height: 0.5,
        base: 1.0,
        color: [1.0, 1.0, 1.0],
        absorption: 0.0,
        reflectance: -1.0,
        color_mix: -1.0,
    };
    let mut camera = Camera::new(0.0, 0.0, 0.0);
    camera.set_pose(Vec3::new(0.0, 0.6, 6.0), 0.0, -0.1);
    let world = open_floor(Some(Sky { color: SKY }), vec![slab]);
    let pixels = draw(&mut window, &mut renderer, &world, &camera);
    let under = sample(&pixels, width, height, &camera, [0.0, 0.0, 0.0]);
    let open = sample(&pixels, width, height, &camera, [0.0, 0.0, 4.5]);
    assert!(
        under[2] < open[2] * 0.6,
        "the slab should hide most of the sky: under {under:?}, open {open:?}"
    );
    assert!(
        under[2] > 2.0,
        "the sky at the sides still reaches under the slab: {under:?}"
    );
}
