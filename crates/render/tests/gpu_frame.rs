//! The real Vulkan draw. Pixel checks use the presented readback, not the CPU bake.

use genos_render::{Renderer, World};
use genos_scene::{viewport_uv, Camera, Floor, Light, Scene, Shape, Solid, Vec3, Wall};
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

fn scene(lights: Vec<Light>, walls: Vec<Wall>, solids: Vec<Solid>) -> World {
    World::from_scene(Scene {
        floor: Floor {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 8.0,
            half_z: 8.0,
            color: [1.0, 1.0, 1.0],
        },
        walls,
        solids,
        lights,
    })
}

fn draw(window: &mut Window, renderer: &mut Renderer, world: &World, camera: &Camera) -> Vec<u8> {
    let _ = window.pump();
    renderer
        .draw(world, camera, true)
        .expect("draw")
        .expect("readback")
}

fn sample(pixels: &[u8], width: u32, height: u32, camera: &Camera, world: [f32; 3]) -> [f32; 3] {
    let aspect = width as f32 / height as f32;
    let uv = viewport_uv(camera, aspect, world).expect("point is on screen");
    let x = (uv[0] * width as f32).round() as i32;
    let y = (uv[1] * height as f32).round() as i32;
    let mut sum = [0.0; 3];
    let mut count = 0.0;
    for dy in -2..=2 {
        for dx in -2..=2 {
            let px = x + dx;
            let py = y + dy;
            if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
                continue;
            }
            let i = ((py as u32 * width + px as u32) * 4) as usize;
            sum[0] += pixels[i + 2] as f32;
            sum[1] += pixels[i + 1] as f32;
            sum[2] += pixels[i] as f32;
            count += 1.0;
        }
    }
    [sum[0] / count, sum[1] / count, sum[2] / count]
}

fn brightness(color: [f32; 3]) -> f32 {
    color[0] + color[1] + color[2]
}

#[test]
fn the_gpu_frame_keeps_the_learned_light() {
    let (_gpu, mut window, mut renderer) = open();
    let width = renderer.width();
    let height = renderer.height();
    let camera = Camera::opening();

    let dark = scene(Vec::new(), Vec::new(), Vec::new());
    let black = draw(&mut window, &mut renderer, &dark, &camera);
    let floor = sample(&black, width, height, &camera, [0.0, 0.0, 0.0]);
    assert!(
        brightness(floor) < 15.0,
        "a scene with no lamp is not black: {floor:?}"
    );

    let blocked = scene(
        vec![Light {
            position: Vec3::new(-4.0, 3.0, 0.0),
            color: [1.0, 1.0, 1.0],
        }],
        vec![Wall {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 0.2,
            half_z: 2.0,
            height: 2.4,
            color: [0.85, 0.85, 0.85],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }],
        Vec::new(),
    );
    let picture = draw(&mut window, &mut renderer, &blocked, &camera);
    let lit_floor = sample(&picture, width, height, &camera, [-1.2, 0.0, 0.0]);
    let shadow_floor = sample(&picture, width, height, &camera, [1.2, 0.0, 0.0]);
    let lit_face = sample(&picture, width, height, &camera, [-0.25, 1.2, 0.0]);
    let east = Camera::new(6.0, 0.0, -std::f32::consts::FRAC_PI_2);
    let from_east = draw(&mut window, &mut renderer, &blocked, &east);
    let far_face = sample(&from_east, width, height, &east, [0.25, 1.2, 0.0]);
    assert!(
        brightness(lit_floor) > brightness(shadow_floor) + 20.0,
        "the wall did not darken the floor: lit {lit_floor:?} shadow {shadow_floor:?}"
    );
    assert!(
        brightness(lit_face) > brightness(far_face) + 15.0,
        "the far face is not darker: lit {lit_face:?} far {far_face:?}"
    );

    let overhead = scene(
        vec![Light {
            position: Vec3::new(0.0, 6.0, 0.0),
            color: [1.0, 1.0, 1.0],
        }],
        Vec::new(),
        vec![Solid {
            shape: Shape::Square,
            position: Vec3::new(0.0, 0.0, 0.0),
            size: 1.2,
            height: 1.0,
            color: [1.0, 0.0, 0.0],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }],
    );
    let tinted = draw(&mut window, &mut renderer, &overhead, &camera);
    let beside = sample(&tinted, width, height, &camera, [1.3, 0.0, 0.0]);
    assert!(
        beside[0] > beside[1] + 4.0 && beside[0] > beside[2] + 4.0,
        "an overhead lamp did not color the floor: {beside:?}"
    );

    let side = scene(
        vec![Light {
            position: Vec3::new(-4.0, 3.0, 0.0),
            color: [1.0, 1.0, 1.0],
        }],
        Vec::new(),
        vec![Solid {
            shape: Shape::Square,
            position: Vec3::new(0.0, 0.0, 0.0),
            size: 1.4,
            height: 1.2,
            color: [1.0, 0.0, 0.0],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }],
    );
    let sided = draw(&mut window, &mut renderer, &side, &camera);
    let far_floor = sample(&sided, width, height, &camera, [1.6, 0.0, 0.0]);
    assert!(
        far_floor[0] < far_floor[1] + 8.0 && far_floor[0] < far_floor[2] + 8.0,
        "a side lamp colored the far floor: {far_floor:?}"
    );

    let bounce = scene(
        vec![Light {
            position: Vec3::new(-5.0, 4.0, 0.0),
            color: [1.0, 1.0, 1.0],
        }],
        vec![Wall {
            position: Vec3::new(3.0, 0.0, 0.0),
            half_x: 0.2,
            half_z: 2.5,
            height: 2.6,
            color: [1.0, 1.0, 1.0],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }],
        vec![Solid {
            shape: Shape::Square,
            position: Vec3::new(0.0, 0.0, 0.0),
            size: 1.2,
            height: 1.4,
            color: [0.7, 0.7, 0.7],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }],
    );
    let filled = draw(&mut window, &mut renderer, &bounce, &camera);
    let near_shadow = sample(&filled, width, height, &camera, [2.2, 0.0, 0.0]);
    let far_shadow = sample(&filled, width, height, &camera, [1.1, 0.0, 0.0]);
    let behind = sample(&filled, width, height, &camera, [3.8, 0.0, 0.0]);
    assert!(
        brightness(near_shadow) > brightness(far_shadow) + 8.0,
        "the wall bounce did not lift the near shadow: near {near_shadow:?} far {far_shadow:?}"
    );
    assert!(
        brightness(behind) + 8.0 < brightness(near_shadow),
        "the floor behind the wall is bright: behind {behind:?} near {near_shadow:?}"
    );
}

#[test]
fn a_normal_submit_returns_while_the_gpu_is_outstanding() {
    let (_gpu, mut window, mut renderer) = open();
    let world = scene(
        vec![Light {
            position: Vec3::new(0.0, 5.0, 0.0),
            color: [1.0, 1.0, 1.0],
        }],
        Vec::new(),
        Vec::new(),
    );
    let camera = Camera::opening();
    let _ = window.pump();
    renderer.draw(&world, &camera, false).expect("first submit");
    assert!(
        renderer.submit_was_pending(),
        "the submit waited for the fence"
    );
    let _ = window.pump();
    renderer
        .draw(&world, &camera, false)
        .expect("second submit");
    let _ = window.pump();
    let pixels = renderer
        .draw(&world, &camera, true)
        .expect("readback submit")
        .expect("pixels");
    assert!(pixels.len() > 1000);
    let floor = sample(
        &pixels,
        renderer.width(),
        renderer.height(),
        &camera,
        [0.0, 0.0, 0.0],
    );
    assert!(
        brightness(floor) > 20.0,
        "readback frame is black: {floor:?}"
    );
}

#[test]
fn a_far_miss_and_a_world_solid_tint_the_floor() {
    let (_gpu, mut window, mut renderer) = open();
    let width = renderer.width();
    let height = renderer.height();
    let camera = Camera::new(0.0, -6.0, std::f32::consts::PI);
    let lamp = Light {
        position: Vec3::new(0.0, 5.0, -2.0),
        color: [1.0, 1.0, 1.0],
    };
    let plain = scene(vec![lamp.clone()], Vec::new(), Vec::new());
    let far = scene(
        vec![lamp.clone()],
        vec![Wall {
            position: Vec3::new(0.0, 0.0, 4.0),
            half_x: 3.0,
            half_z: 0.2,
            height: 2.4,
            color: [1.0, 0.0, 0.0],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }],
        Vec::new(),
    );
    let bare = draw(&mut window, &mut renderer, &plain, &camera);
    let tinted = draw(&mut window, &mut renderer, &far, &camera);
    let spot = [0.0, 0.0, 0.5];
    let before = sample(&bare, width, height, &camera, spot);
    let after = sample(&tinted, width, height, &camera, spot);
    assert!(
        after[0] > before[0] + 4.0,
        "a far wall did not tint the floor: before {before:?} after {after:?}"
    );

    let world_solid = scene(
        vec![lamp],
        vec![Wall {
            position: Vec3::new(0.0, 0.0, 22.0),
            half_x: 12.0,
            half_z: 0.3,
            height: 4.0,
            color: [0.0, 1.0, 0.0],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }],
        Vec::new(),
    );
    let carried = draw(&mut window, &mut renderer, &world_solid, &camera);
    let edge = sample(&carried, width, height, &camera, [0.0, 0.0, 6.5]);
    let open = sample(&bare, width, height, &camera, [0.0, 0.0, 6.5]);
    assert!(
        edge[1] > open[1] + 1.0,
        "an off-floor solid did not tint the world range: open {open:?} edge {edge:?}"
    );
}

#[test]
fn moving_a_middle_solid_moves_it_on_the_gpu() {
    let (_gpu, mut window, mut renderer) = open();
    let width = renderer.width();
    let height = renderer.height();
    let camera = Camera::opening();
    let mut world = scene(
        vec![Light {
            position: Vec3::new(0.0, 6.0, 0.0),
            color: [1.0, 1.0, 1.0],
        }],
        Vec::new(),
        vec![
            Solid {
                shape: Shape::Square,
                position: Vec3::new(-2.0, 0.0, 0.0),
                size: 1.0,
                height: 1.0,
                color: [0.2, 0.2, 0.2],
                absorption: 0.0,
                reflectance: -1.0,
                color_mix: -1.0,
            },
            Solid {
                shape: Shape::Square,
                position: Vec3::new(0.0, 0.0, 0.0),
                size: 1.0,
                height: 1.2,
                color: [1.0, 0.0, 0.0],
                absorption: 0.0,
                reflectance: -1.0,
                color_mix: -1.0,
            },
            Solid {
                shape: Shape::Square,
                position: Vec3::new(2.0, 0.0, 0.0),
                size: 1.0,
                height: 1.0,
                color: [0.2, 0.2, 0.2],
                absorption: 0.0,
                reflectance: -1.0,
                color_mix: -1.0,
            },
        ],
    );
    let first = draw(&mut window, &mut renderer, &world, &camera);
    let on_red = sample(&first, width, height, &camera, [0.0, 1.15, 0.0]);
    let lead = |color: [f32; 3]| color[0] - color[2];
    assert!(
        lead(on_red) > 15.0,
        "the middle solid is not red: {on_red:?}"
    );
    world.scene.solids[1].position.x = 1.6;
    let second = draw(&mut window, &mut renderer, &world, &camera);
    let old_spot = sample(&second, width, height, &camera, [0.0, 1.15, 0.0]);
    let new_spot = sample(&second, width, height, &camera, [1.6, 1.15, 0.0]);
    assert!(
        lead(old_spot) + 8.0 < lead(on_red),
        "the old place is still the red solid: {old_spot:?} was {on_red:?}"
    );
    assert!(
        lead(new_spot) > 15.0,
        "the solid did not appear at the new place: {new_spot:?}"
    );
}

#[test]
fn an_earlier_frame_keeps_its_lamp_after_the_next_submit() {
    let (_gpu, mut window, mut renderer) = open();
    let width = renderer.width();
    let height = renderer.height();
    let camera = Camera::opening();
    let lit = scene(
        vec![Light {
            position: Vec3::new(0.0, 6.0, 0.0),
            color: [1.0, 1.0, 1.0],
        }],
        Vec::new(),
        Vec::new(),
    );
    let dark = scene(Vec::new(), Vec::new(), Vec::new());
    let _ = window.pump();
    renderer.draw(&lit, &camera, false).expect("lamp frame");
    let _ = window.pump();
    renderer.draw(&dark, &camera, false).expect("dark frame");
    let earlier = renderer.read_earlier_frame().expect("earlier readback");
    let floor = sample(&earlier, width, height, &camera, [0.0, 0.0, 0.0]);
    assert!(
        brightness(floor) > 30.0,
        "the earlier frame lost its lamp: {floor:?}"
    );
}

#[test]
fn a_close_lamp_does_not_cross_the_wall() {
    let (_gpu, mut window, mut renderer) = open();
    let width = renderer.width();
    let height = renderer.height();
    let world = scene(
        vec![Light {
            position: Vec3::new(-0.45, 0.4, 1.85),
            color: [1.0, 1.0, 1.0],
        }],
        vec![Wall {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 0.2,
            half_z: 2.0,
            height: 2.6,
            color: [0.8, 0.8, 0.8],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }],
        Vec::new(),
    );
    let west = Camera::new(-3.5, 1.5, std::f32::consts::FRAC_PI_2);
    let east = Camera::new(3.5, 1.5, -std::f32::consts::FRAC_PI_2);
    let lit = draw(&mut window, &mut renderer, &world, &west);
    let back = draw(&mut window, &mut renderer, &world, &east);
    let lit_face = sample(&lit, width, height, &west, [-0.25, 1.0, 1.7]);
    let lit_floor = sample(&lit, width, height, &west, [-0.5, 0.0, 1.7]);
    let back_corner = sample(&back, width, height, &east, [0.28, 1.2, 1.7]);
    let back_low = sample(&back, width, height, &east, [0.28, 0.2, 1.7]);
    let shadow_floor = sample(&back, width, height, &east, [0.45, 0.0, 1.7]);
    assert!(
        brightness(lit_face) > 400.0,
        "the lit face lost the close lamp: {lit_face:?}"
    );
    assert!(
        brightness(back_corner) < 45.0,
        "the close lamp crossed onto the back face: {back_corner:?}"
    );
    assert!(
        brightness(back_low) < 45.0,
        "the close lamp crossed the base of the back face: {back_low:?}"
    );
    assert!(
        brightness(shadow_floor) < 60.0 && brightness(lit_floor) > 400.0,
        "light bleeds under the wall: shadow {shadow_floor:?} lit {lit_floor:?}"
    );
}

#[test]
fn the_wall_has_no_bright_dashes_and_the_cube_shadow_keeps_some_light() {
    let (_gpu, mut window, mut renderer) = open();
    let width = renderer.width();
    let height = renderer.height();
    let world = scene(
        vec![Light {
            position: Vec3::new(-2.5, 1.6, -1.0),
            color: [1.0, 1.0, 1.0],
        }],
        vec![
            Wall {
                position: Vec3::new(2.0, 0.0, 5.0),
                half_x: 4.0,
                half_z: 0.2,
                height: 2.6,
                color: [1.0, 1.0, 1.0],
                absorption: 0.0,
                reflectance: -1.0,
                color_mix: -1.0,
            },
            Wall {
                position: Vec3::new(6.0, 0.0, 1.5),
                half_x: 0.2,
                half_z: 3.5,
                height: 2.6,
                color: [1.0, 1.0, 1.0],
                absorption: 0.0,
                reflectance: -1.0,
                color_mix: -1.0,
            },
        ],
        vec![Solid {
            shape: Shape::Square,
            position: Vec3::new(0.0, 0.0, 0.0),
            size: 1.5,
            height: 1.2,
            color: [1.0, 0.0, 0.0],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }],
    );
    let camera = Camera::new(0.2, -3.2, std::f32::consts::PI);
    let pixels = draw(&mut window, &mut renderer, &world, &camera);
    let mut ridges = 0u32;
    let w = width as i32;
    let h = height as i32;
    let at = |x: i32, y: i32| {
        let i = ((y as u32 * width + x as u32) * 4) as usize;
        pixels[i + 2] as i32
    };
    for y in 2..h - 2 {
        for x in 2..w - 2 {
            let c = at(x, y);
            if c > 80 && c < 250 && c >= at(x - 1, y) + 15 && c >= at(x + 1, y) + 15 {
                ridges += 1;
            }
        }
    }
    assert!(ridges < 40, "the wall has bright dashes: {ridges}");
    let shadow = sample(&pixels, width, height, &camera, [1.15, 0.0, 0.15]);
    let lit = sample(&pixels, width, height, &camera, [-1.4, 0.0, 0.4]);
    assert!(
        brightness(lit) > brightness(shadow) + 40.0,
        "the cube did not shadow the floor: lit {lit:?} shadow {shadow:?}"
    );
    assert!(
        brightness(shadow) > 18.0,
        "the cube shadow is a black hole: {shadow:?}"
    );
}

fn bumps(values: &[f32]) -> u32 {
    let mut count = 0;
    if values.len() < 5 {
        return 0;
    }
    for i in 2..values.len() - 2 {
        let left = values[i - 2].min(values[i - 1]);
        let right = values[i + 1].min(values[i + 2]);
        let left_hi = values[i - 2].max(values[i - 1]);
        let right_hi = values[i + 1].max(values[i + 2]);
        if values[i] + 35.0 < left && values[i] + 35.0 < right {
            count += 1;
        }
        if values[i] > left_hi + 35.0 && values[i] > right_hi + 35.0 {
            count += 1;
        }
    }
    count
}

fn wall_line(pixels: &[u8], width: u32, height: u32, camera: &Camera) -> Vec<f32> {
    (0..11)
        .map(|step| {
            let x = -2.0 + step as f32 * 0.4;
            brightness(sample(pixels, width, height, camera, [x, 1.25, -0.2]))
        })
        .collect()
}

fn floor_line(pixels: &[u8], width: u32, height: u32, camera: &Camera) -> Vec<f32> {
    (0..9)
        .map(|step| {
            let z = -2.4 + step as f32 * 0.2;
            brightness(sample(pixels, width, height, camera, [0.4, 0.0, z]))
        })
        .collect()
}

#[test]
fn lit_views_stay_smooth_and_low_lamps_stop_at_the_wall() {
    let (_gpu, mut window, mut renderer) = open();
    let width = renderer.width();
    let height = renderer.height();
    let wall = Wall {
        position: Vec3::new(0.0, 0.0, 0.0),
        half_x: 4.0,
        half_z: 0.2,
        height: 2.6,
        color: [0.85, 0.85, 0.85],
        absorption: 0.0,
        reflectance: -1.0,
        color_mix: -1.0,
    };
    let lamps = [Vec3::new(-1.2, 1.3, -3.0), Vec3::new(0.7, 0.45, -2.1)];
    let cameras = [
        Camera::new(0.0, -6.5, std::f32::consts::PI),
        Camera::new(-2.4, -6.2, std::f32::consts::PI + 0.28),
    ];
    for lamp in lamps {
        let world = scene(
            vec![Light {
                position: lamp,
                color: [1.0, 1.0, 1.0],
            }],
            vec![wall.clone()],
            Vec::new(),
        );
        for camera in &cameras {
            let pixels = draw(&mut window, &mut renderer, &world, camera);
            let wall_samples = wall_line(&pixels, width, height, camera);
            let floor_samples = floor_line(&pixels, width, height, camera);
            assert!(
                bumps(&wall_samples) == 0,
                "the wall has a light band: lamp {lamp:?} wall {wall_samples:?}"
            );
            assert!(
                bumps(&floor_samples) == 0,
                "the floor has a light band: lamp {lamp:?} floor {floor_samples:?}"
            );
        }
        let north = Camera::new(0.0, 6.2, 0.0);
        let south = &cameras[0];
        let lit = draw(&mut window, &mut renderer, &world, south);
        let back = draw(&mut window, &mut renderer, &world, &north);
        let lit_face = brightness(sample(&lit, width, height, south, [0.2, 1.25, -0.28]));
        assert!(
            lit_face > 80.0,
            "the lit face is dark: {lit_face} lamp {lamp:?}"
        );
        for x in [-1.5_f32, 0.2, 1.6] {
            let face = brightness(sample(&back, width, height, &north, [x, 1.2, 0.3]));
            let base = brightness(sample(&back, width, height, &north, [x, 0.2, 0.3]));
            let floor = brightness(sample(&back, width, height, &north, [x, 0.0, 0.75]));
            println!(
                "unlit lamp {lamp:?} lit_face {lit_face} x={x} face {face} base {base} floor {floor}"
            );
            assert!(
                face < lit_face * 0.08 && base < lit_face * 0.08 && floor < lit_face * 0.08,
                "light crossed the wall at x={x}: face {face} base {base} floor {floor} lit {lit_face}"
            );
        }
    }

    let shadow_world = scene(
        vec![Light {
            position: Vec3::new(-2.6, 1.5, -3.2),
            color: [1.0, 1.0, 1.0],
        }],
        vec![wall],
        vec![Solid {
            shape: Shape::Square,
            position: Vec3::new(-0.4, 0.0, -1.5),
            size: 1.3,
            height: 1.2,
            color: [1.0, 0.0, 0.0],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }],
    );
    let camera = Camera::new(-0.6, -6.0, std::f32::consts::PI);
    let pixels = draw(&mut window, &mut renderer, &shadow_world, &camera);
    let lit = sample(&pixels, width, height, &camera, [-2.2, 0.0, -2.5]);
    let shadow = sample(&pixels, width, height, &camera, [1.1, 0.0, -0.7]);
    let lit_face = sample(&pixels, width, height, &camera, [-0.4, 0.6, -2.25]);
    let east = Camera::new(4.0, -1.5, -std::f32::consts::FRAC_PI_2);
    let side = draw(&mut window, &mut renderer, &shadow_world, &east);
    let far_side = sample(&side, width, height, &east, [0.35, 0.6, -1.5]);
    assert!(
        brightness(lit) > brightness(shadow) + 40.0,
        "the solid did not shadow the floor: lit {lit:?} shadow {shadow:?}"
    );
    assert!(
        brightness(shadow) > 18.0,
        "the solid shadow is black: {shadow:?}"
    );
    assert!(
        brightness(far_side) + 40.0 < brightness(lit_face),
        "the occluded side is lit like the lamp side: far {far_side:?} lit {lit_face:?}"
    );
}

#[test]
fn the_corridor_carries_bounce_around_the_bend() {
    let (_gpu, mut window, mut renderer) = open();
    let width = renderer.width();
    let height = renderer.height();
    let mut world = World::from_scene(
        genos_scene::load_path(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/camera/scene.rhai"
        )))
        .expect("shipped scene"),
    );
    world.scene.lights = vec![Light {
        position: Vec3::new(0.0, 1.1, 6.2),
        color: [1.0, 1.0, 1.0],
    }];
    let mouth_cam = Camera::new(0.0, 5.5, std::f32::consts::PI);
    let far_cam = Camera::new(0.2, 9.0, std::f32::consts::FRAC_PI_2);
    let out_cam = Camera::new(-6.2, 7.0, std::f32::consts::FRAC_PI_2);
    let mouth_px = draw(&mut window, &mut renderer, &world, &mouth_cam);
    let far_px = draw(&mut window, &mut renderer, &world, &far_cam);
    let out_px = draw(&mut window, &mut renderer, &world, &out_cam);
    let mouth = brightness(sample(
        &mouth_px,
        width,
        height,
        &mouth_cam,
        [0.0, 0.0, 8.8],
    ));
    let far = brightness(sample(&far_px, width, height, &far_cam, [4.2, 0.0, 9.0]));
    let outside = brightness(sample(&out_px, width, height, &out_cam, [-2.4, 0.0, 7.0]));
    world.scene.lights.clear();
    let dark_px = draw(&mut window, &mut renderer, &world, &far_cam);
    let dark = brightness(sample(&dark_px, width, height, &far_cam, [4.2, 0.0, 9.0]));
    println!("corridor mouth {mouth} far {far} outside {outside} dark {dark}");
    assert!(
        mouth > far + 30.0,
        "the bend is not dimmer than the mouth: mouth {mouth} far {far}"
    );
    assert!(
        far > dark + 12.0,
        "the far leg has no bounced light: far {far} dark {dark}"
    );
    assert!(
        outside + 8.0 < far,
        "light crossed a corridor wall: outside {outside} far {far}"
    );
}

#[test]
fn the_outside_lamp_and_the_fire_stop_at_the_corridor_wall() {
    let (_gpu, mut window, mut renderer) = open();
    let width = renderer.width();
    let height = renderer.height();
    let mut world = World::from_scene(
        genos_scene::load_path(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/camera/scene.rhai"
        )))
        .expect("shipped scene"),
    );
    world.scene.lights = vec![Light {
        position: Vec3::new(8.0, 1.0, 5.0),
        color: [8.0, 8.0, 8.0],
    }];
    let shadow_cam = Camera::new(1.0, 6.0, -std::f32::consts::FRAC_PI_2);
    let gap_cam = Camera::new(1.0, 9.0, -std::f32::consts::FRAC_PI_2);
    let shadow_px = draw(&mut window, &mut renderer, &world, &shadow_cam);
    let gap_px = draw(&mut window, &mut renderer, &world, &gap_cam);
    let shadow_face = sample(&shadow_px, width, height, &shadow_cam, [-1.28, 1.1, 6.0]);
    let gap_face = sample(&gap_px, width, height, &gap_cam, [-1.28, 1.1, 9.0]);
    assert!(
        brightness(gap_face) > brightness(shadow_face) + 80.0,
        "the gap face is not brighter than the shadowed face: gap {gap_face:?} shadow {shadow_face:?}"
    );

    world.scene.lights.clear();
    let mut sim = genos_render::Simulation::from_scene(&world.scene);
    for _ in 0..genos_render::PROOF_STEPS {
        sim.advance(&mut world, genos_render::STEP_DT);
    }
    let mut beside_cam = Camera::new(0.8, 4.6, 0.0);
    beside_cam.pitch = -0.45;
    let mut hall_cam = Camera::new(0.0, 8.6, 0.0);
    hall_cam.pitch = -0.4;
    let beside_px = draw(&mut window, &mut renderer, &world, &beside_cam);
    let hall_px = draw(&mut window, &mut renderer, &world, &hall_cam);
    let beside = sample(&beside_px, width, height, &beside_cam, [0.8, 0.0, 1.8]);
    let hall = sample(&hall_px, width, height, &hall_cam, [0.0, 0.0, 6.4]);
    assert!(
        brightness(beside) > 80.0 && beside[0] > beside[2] + 40.0,
        "the fire did not warm the nearby floor: {beside:?}"
    );
    assert!(
        brightness(hall) < 12.0,
        "the fire lit the corridor through the wall: beside {beside:?} hall {hall:?}"
    );
}

fn opaque_wall() -> Wall {
    Wall {
        position: Vec3::new(0.0, 0.0, 0.0),
        half_x: 0.2,
        half_z: 3.0,
        height: 3.0,
        color: [0.8, 0.8, 0.8],
        absorption: 0.0,
        reflectance: -1.0,
        color_mix: -1.0,
    }
}

#[test]
fn a_bright_lamp_does_not_cross_an_opaque_wall() {
    let (_gpu, mut window, mut renderer) = open();
    let width = renderer.width();
    let height = renderer.height();
    let east = Camera::new(2.4, 0.0, -std::f32::consts::FRAC_PI_2);
    let west = Camera::new(-2.4, 0.0, std::f32::consts::FRAC_PI_2);
    let mut floor_cam = Camera::new(1.2, 2.2, 0.0);
    floor_cam.pitch = -0.85;
    let world = scene(
        vec![Light {
            position: Vec3::new(-2.0, 1.2, 0.0),
            color: [8.0, 8.0, 8.0],
        }],
        vec![opaque_wall()],
        Vec::new(),
    );
    let east_px = draw(&mut window, &mut renderer, &world, &east);
    let west_px = draw(&mut window, &mut renderer, &world, &west);
    let floor_px = draw(&mut window, &mut renderer, &world, &floor_cam);
    let shadow_face = sample(&east_px, width, height, &east, [0.24, 1.2, 0.0]);
    let lit_face = sample(&west_px, width, height, &west, [-0.24, 1.2, 0.0]);
    let shadow_floor = sample(&floor_px, width, height, &floor_cam, [1.2, 0.0, 0.7]);
    assert!(
        brightness(shadow_face) < 24.0 && brightness(shadow_floor) < 24.0,
        "a bright lamp crossed the wall: face {shadow_face:?} floor {shadow_floor:?}"
    );
    assert!(
        brightness(lit_face) > 400.0,
        "the lit face went dark: {lit_face:?}"
    );
    let above = scene(
        vec![Light {
            position: Vec3::new(0.0, 6.0, 0.0),
            color: [8.0, 8.0, 8.0],
        }],
        vec![opaque_wall()],
        Vec::new(),
    );
    let east_px = draw(&mut window, &mut renderer, &above, &east);
    let west_px = draw(&mut window, &mut renderer, &above, &west);
    let floor_px = draw(&mut window, &mut renderer, &above, &floor_cam);
    let above_east = sample(&east_px, width, height, &east, [0.24, 1.2, 0.0]);
    let above_west = sample(&west_px, width, height, &west, [-0.24, 1.2, 0.0]);
    let above_floor = sample(&floor_px, width, height, &floor_cam, [1.2, 0.0, 0.7]);
    assert!(
        brightness(above_east) < 180.0 && brightness(above_west) < 180.0,
        "a lamp above the wall lit the faces through it: east {above_east:?} west {above_west:?}"
    );
    assert!(
        brightness(above_floor) > brightness(above_east) + 40.0,
        "light over the wall did not reach the floor: floor {above_floor:?} face {above_east:?}"
    );
}
