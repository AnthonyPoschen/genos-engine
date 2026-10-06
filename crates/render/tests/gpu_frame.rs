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

/// The picture a game frame presents. This does not wait for a new light gather.
fn draw_game(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
) -> Vec<u8> {
    let _ = window.pump();
    renderer.draw(world, camera, false).expect("draw");
    renderer.read_picture().expect("picture")
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

/// Pixel channels for the direct lamp only, using the same falloff and occlusion as the shader.
/// Bounce is not included. A white wall and a white lamp make the three channels equal.
fn direct_wall_pixel(scene: &Scene, surface: [f32; 3], normal: [f32; 3]) -> [f32; 3] {
    let origin = [
        surface[0] + normal[0] * 0.02,
        surface[1] + normal[1] * 0.02,
        surface[2] + normal[2] * 0.02,
    ];
    let direct = genos_render::illuminate_facing(scene, origin[0], origin[1], origin[2], normal);
    let linear = 0.318309886 * direct;
    let toned = if linear <= 0.64 {
        linear.max(0.0)
    } else {
        let extra = linear - 0.64;
        0.64 + 0.14 * (extra / (extra + 1.1))
    };
    [toned * 255.0; 3]
}

fn shipped_scene() -> Scene {
    genos_scene::load_path(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/camera/scene.rhai"
    )))
    .expect("shipped scene")
}

fn shipped_hall() -> World {
    World::from_scene(shipped_scene())
}

/// Hall walls and the white lamp. The flame and the colored solids stay out.
fn hall_lamp() -> World {
    let mut scene = shipped_scene();
    scene.solids.clear();
    World::from_scene(scene)
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

            direction: Vec3::ZERO,
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
        brightness(lit_floor) > brightness(shadow_floor) + 12.0,
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

            direction: Vec3::ZERO,
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
        beside[0] > beside[1] + 1.0 && beside[0] > beside[2] + 1.0,
        "an overhead lamp did not color the floor: {beside:?}"
    );

    let side = scene(
        vec![Light {
            position: Vec3::new(-4.0, 3.0, 0.0),
            color: [1.0, 1.0, 1.0],

            direction: Vec3::ZERO,
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

            direction: Vec3::ZERO,
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
        brightness(near_shadow) > 4.0,
        "the floor in front of the wall has no bounce: near {near_shadow:?} far {far_shadow:?}"
    );
    assert!(
        brightness(behind) < brightness(near_shadow) + 4.0,
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

            direction: Vec3::ZERO,
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

        direction: Vec3::ZERO,
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
        after[0] > before[0] + 0.2,
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
        edge[1] + 1.0 >= open[1],
        "an off-floor wall darkened the floor: open {open:?} edge {edge:?}"
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

            direction: Vec3::ZERO,
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

            direction: Vec3::ZERO,
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

            direction: Vec3::ZERO,
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

            direction: Vec3::ZERO,
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
        brightness(shadow) > 4.0,
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

#[test]
fn a_camera_spin_does_not_recolor_the_wall() {
    let (_gpu, mut window, mut renderer) = open();
    let width = renderer.width();
    let height = renderer.height();
    let world = hall_lamp();
    // South face of the long hall wall. The point sits just outside the face.
    let points = [
        [0.2, 1.3, 4.72],
        [-1.2, 1.6, 4.72],
        [1.6, 0.8, 4.72],
        [2.4, 1.5, 4.72],
        [-0.4, 1.0, 4.72],
    ];
    let mut base: Option<Vec<[f32; 3]>> = None;
    let mut worst = 0.0_f32;
    let mut note = String::new();
    let mut compared = 0u32;
    for step_i in 0..8 {
        let mut camera = Camera::new(0.4, 2.4, std::f32::consts::PI - 0.3 + step_i as f32 * 0.08);
        camera.pitch = -0.12;
        let pixels = draw_game(&mut window, &mut renderer, &world, &camera);
        let aspect = width as f32 / height as f32;
        let colors: Vec<[f32; 3]> = points
            .iter()
            .map(|point| {
                if viewport_uv(&camera, aspect, *point).is_none() {
                    [-1.0, -1.0, -1.0]
                } else {
                    sample(&pixels, width, height, &camera, *point)
                }
            })
            .collect();
        if let Some(before) = &base {
            for (index, (old, new)) in before.iter().zip(colors.iter()).enumerate() {
                if old[0] < 0.0 || new[0] < 0.0 {
                    continue;
                }
                compared += 1;
                for channel in 0..3 {
                    let gap = (old[channel] - new[channel]).abs();
                    if gap > worst {
                        worst = gap;
                        note = format!("step {step_i} point {index} {old:?} -> {new:?}");
                    }
                }
            }
        } else {
            base = Some(colors);
        }
    }
    assert!(compared >= 4, "the wall left the picture");
    assert!(worst < 4.0, "spin gap {worst:.1}: {note}");
}

#[test]
fn walking_does_not_flip_the_light_between_states() {
    let (_gpu, mut window, mut renderer) = open();
    let width = renderer.width();
    let height = renderer.height();
    let world = shipped_hall();
    let wall = [0.2_f32, 1.3, 4.72];
    let mut camera = Camera::new(0.0, 1.2, std::f32::consts::PI);
    camera.pitch = -0.18;
    let mut wall_colors: Vec<[f32; 3]> = Vec::new();
    let mut note = String::new();
    let mut worst = 0.0_f32;
    let mut flips = 0u32;
    for step_i in 0..48 {
        // Four meters a second at 60 frames a second, plus a small strafe and look.
        camera.position.z = 1.2 + step_i as f32 * (4.0 / 60.0);
        camera.position.x = (step_i as f32 * 0.03).sin() * 0.35;
        camera.yaw = std::f32::consts::PI + (step_i as f32 * 0.04).sin() * 0.2;
        let pixels = draw_game(&mut window, &mut renderer, &world, &camera);
        let aspect = width as f32 / height as f32;
        let wall_px = if viewport_uv(&camera, aspect, wall).is_none() {
            [-1.0, -1.0, -1.0]
        } else {
            sample(&pixels, width, height, &camera, wall)
        };
        if let Some(prev) = wall_colors.last().copied() {
            if prev[0] >= 0.0 && wall_px[0] >= 0.0 {
                for channel in 0..3 {
                    let gap = (prev[channel] - wall_px[channel]).abs();
                    if gap > worst {
                        worst = gap;
                        note = format!("step {step_i} {prev:?} -> {wall_px:?}");
                    }
                    if gap > 8.0 {
                        flips += 1;
                    }
                }
            }
        }
        wall_colors.push(wall_px);
    }
    let mut lo = [1.0e9_f32; 3];
    let mut hi = [0.0_f32; 3];
    let mut seen = 0u32;
    for color in &wall_colors {
        if color[0] < 0.0 {
            continue;
        }
        seen += 1;
        for channel in 0..3 {
            lo[channel] = lo[channel].min(color[channel]);
            hi[channel] = hi[channel].max(color[channel]);
        }
    }
    let span = (0..3)
        .map(|channel| hi[channel] - lo[channel])
        .fold(0.0_f32, f32::max);
    assert!(seen >= 40, "the wall left the picture");
    assert!(
        // The screen grid moves with the camera. The wall stays one color. It does not flip.
        flips == 0 && span < 6.0,
        "flips {flips} span {span:.1} worst {worst:.1}: {note}\n{wall_colors:?}"
    );
}

#[test]
fn a_turn_toward_the_lamp_does_not_pop_the_bounce() {
    let (_gpu, mut window, mut renderer) = open();
    let width = renderer.width();
    let height = renderer.height();
    let world = shipped_hall();
    // Floor beside the red solid. The lamp is at (0, 7, 0).
    let mut away = Camera::new(0.2, 3.2, std::f32::consts::PI);
    away.pitch = -0.45;
    let mut toward = away.clone();
    toward.yaw = 0.0;
    let _away_px = draw(&mut window, &mut renderer, &world, &away);
    let held = draw_game(&mut window, &mut renderer, &world, &toward);
    let settled = draw(&mut window, &mut renderer, &world, &toward);
    let aspect = width as f32 / height as f32;
    let mut worst = 0.0_f32;
    let mut note = String::new();
    let mut compared = 0u32;
    for iz in 0..12 {
        for ix in 0..10 {
            let point = [-2.0 + ix as f32 * 0.5, 0.0, iz as f32 * 0.4];
            let on = viewport_uv(&toward, aspect, point)
                .is_some_and(|uv| uv[0] >= 0.02 && uv[0] <= 0.98 && uv[1] >= 0.02 && uv[1] <= 0.98);
            if !on {
                continue;
            }
            let before = sample(&held, width, height, &toward, point);
            let after = sample(&settled, width, height, &toward, point);
            compared += 1;
            for channel in 0..3 {
                let gap = (before[channel] - after[channel]).abs();
                if gap > worst {
                    worst = gap;
                    note = format!("{point:?} {before:?} -> {after:?}");
                }
            }
        }
    }
    assert!(
        // A half-turn rebuilds the screen grid. The floor stays lit. One frame can shift it.
        compared >= 8 && worst < 16.0,
        "the bounce popped when the view met the lamp, worst {worst:.1} {note} compared {compared}"
    );
}

#[test]
fn a_small_step_does_not_flash_a_wall() {
    let (_gpu, mut window, mut renderer) = open();
    let width = renderer.width();
    let height = renderer.height();
    let world = scene(
        vec![Light {
            position: Vec3::new(-1.2, 1.3, -3.0),
            color: [1.0, 1.0, 1.0],

            direction: Vec3::ZERO,
        }],
        vec![Wall {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 4.0,
            half_z: 0.2,
            height: 2.6,
            color: [0.85, 0.85, 0.85],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }],
        Vec::new(),
    );
    let mut here = Camera::new(0.0, -6.0, std::f32::consts::PI);
    here.pitch = -0.15;
    let mut stepped = here.clone();
    stepped.position.x += 0.08;
    stepped.position.z += 0.06;
    stepped.yaw += 0.012;
    stepped.pitch -= 0.008;
    let here_px = draw(&mut window, &mut renderer, &world, &here);
    let step_px = draw(&mut window, &mut renderer, &world, &stepped);
    let mut worst = 0.0_f32;
    for step_i in 0..12 {
        let point = [-1.2 + step_i as f32 * 0.2, 1.4, -0.28];
        let before = brightness(sample(&here_px, width, height, &here, point));
        let after = brightness(sample(&step_px, width, height, &stepped, point));
        worst = worst.max((before - after).abs());
    }
    assert!(worst < 12.0, "the wall flashed by {worst}");
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

                direction: Vec3::ZERO,
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

            direction: Vec3::ZERO,
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
    // Floor in front of the lit south face. The red leaves that face onto this ground.
    let shadow = sample(&pixels, width, height, &camera, [-0.4, 0.0, -2.4]);
    let lit_face = sample(&pixels, width, height, &camera, [-0.4, 0.6, -2.25]);
    let east = Camera::new(4.0, -1.5, -std::f32::consts::FRAC_PI_2);
    let side = draw(&mut window, &mut renderer, &shadow_world, &east);
    let far_side = sample(&side, width, height, &east, [0.35, 0.6, -1.5]);
    assert!(
        brightness(lit) > brightness(shadow) + 40.0,
        "the solid did not shadow the floor: lit {lit:?} shadow {shadow:?}"
    );
    assert!(
        shadow[0] > shadow[1] && brightness(shadow) > 0.5,
        "the solid shadow lost the red bounce: {shadow:?}"
    );
    assert!(
        brightness(far_side) + 40.0 < brightness(lit_face),
        "the occluded side is lit like the lamp side: far {far_side:?} lit {lit_face:?}"
    );
}

#[test]
fn the_hallway_wall_matches_its_direct_light() {
    let (_gpu, mut window, mut renderer) = open();
    let width = renderer.width();
    let height = renderer.height();
    let world = shipped_hall();
    // The sun travels down at 45 degrees toward +Z. A south face meets that ray.
    // The long wall's hall face points +Z, so the sun misses it. Bounce stays dim.
    let mut lit = 0.0_f32;
    lit = lit.max(check_hall_wall(
        &mut window,
        &mut renderer,
        &world,
        width,
        height,
        "far wall sees the lamp",
        -0.6,
        8.6,
        std::f32::consts::PI,
        [-1.0, 1.3, 10.2],
        [0.0, 0.0, -1.0],
        [-1.0, 1.3, 10.12],
    ));
    lit = lit.max(check_hall_wall(
        &mut window,
        &mut renderer,
        &world,
        width,
        height,
        "cross wall sees the lamp",
        3.8,
        6.6,
        std::f32::consts::PI,
        [3.8, 1.3, 8.1],
        [0.0, 0.0, -1.0],
        [3.8, 1.3, 8.02],
    ));
    check_hall_wall(
        &mut window,
        &mut renderer,
        &world,
        width,
        height,
        "long wall faces away from the lamp",
        2.0,
        7.0,
        0.0,
        [2.0, 1.3, 5.2],
        [0.0, 0.0, 1.0],
        [2.0, 1.3, 5.28],
    );
    assert!(lit > 40.0, "no hallway wall carried the direct lamp");
}

fn check_hall_wall(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    width: u32,
    height: u32,
    name: &str,
    eye_x: f32,
    eye_z: f32,
    yaw: f32,
    surface: [f32; 3],
    normal: [f32; 3],
    sample_at: [f32; 3],
) -> f32 {
    let mut camera = Camera::new(eye_x, eye_z, yaw);
    camera.pitch = -0.22;
    let expected = direct_wall_pixel(&world.scene, surface, normal);
    let pixels = draw(window, renderer, world, &camera);
    let measured = sample(&pixels, width, height, &camera, sample_at);
    if expected[0] > 15.0 {
        for channel in 0..3 {
            assert!(
                measured[channel] + 12.0 >= expected[channel],
                "{name} lost the direct lamp: expected {expected:?} measured {measured:?}"
            );
            assert!(
                measured[channel] <= expected[channel] + 20.0,
                "{name} is brighter than the direct lamp plus bounce: expected {expected:?} measured {measured:?}"
            );
        }
        return brightness(measured);
    }
    assert!(
        brightness(measured) < 90.0,
        "{name} is lit without a lamp ray: expected {expected:?} measured {measured:?}"
    );
    0.0
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

        direction: Vec3::ZERO,
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
        far > dark + 2.0,
        "the far leg has no bounced light: far {far} dark {dark}"
    );
    assert!(
        outside * 2.5 < mouth,
        "light crossed a corridor wall: outside {outside} mouth {mouth} far {far}"
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

        direction: Vec3::ZERO,
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
        brightness(beside) > 48.0 && beside[0] > beside[2] + 20.0,
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

            direction: Vec3::ZERO,
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

            direction: Vec3::ZERO,
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
        brightness(above_east) < brightness(above_floor)
            && brightness(above_west) < brightness(above_floor),
        "a lamp above the wall lit the faces through it: east {above_east:?} west {above_west:?} floor {above_floor:?}"
    );
    assert!(
        brightness(above_floor) > brightness(above_east) + 40.0,
        "light over the wall did not reach the floor: floor {above_floor:?} face {above_east:?}"
    );
}

fn fit3(rows: &[[f32; 3]], ys: &[f32]) -> [f32; 3] {
    let mut ata = [[0.0_f64; 3]; 3];
    let mut atb = [0.0_f64; 3];
    for (row, y) in rows.iter().zip(ys.iter()) {
        for i in 0..3 {
            atb[i] += row[i] as f64 * *y as f64;
            for j in 0..3 {
                ata[i][j] += row[i] as f64 * row[j] as f64;
            }
        }
    }
    // Gaussian elimination. The radial basis is well conditioned on a short span.
    let mut a = ata;
    let mut b = atb;
    for col in 0..3 {
        let mut pivot = col;
        for row in col + 1..3 {
            if a[row][col].abs() > a[pivot][col].abs() {
                pivot = row;
            }
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        let div = a[col][col];
        if div.abs() < 1.0e-12 {
            return [0.0; 3];
        }
        for j in col..3 {
            a[col][j] /= div;
        }
        b[col] /= div;
        for row in 0..3 {
            if row == col {
                continue;
            }
            let factor = a[row][col];
            for j in col..3 {
                a[row][j] -= factor * a[col][j];
            }
            b[row] -= factor * b[col];
        }
    }
    [b[0] as f32, b[1] as f32, b[2] as f32]
}

fn on_screen(camera: &Camera, width: u32, height: u32, world: [f32; 3]) -> bool {
    let uv = viewport_uv(camera, width as f32 / height as f32, world);
    uv.is_some_and(|uv| (0.02..0.98).contains(&uv[0]) && (0.02..0.98).contains(&uv[1]))
}

/// Presented light on an open floor, and colored bounce inside a geometric shadow.
#[test]
fn an_open_floor_falls_off_smoothly_and_a_shadow_keeps_colored_bounce() {
    let (_gpu, mut window, mut renderer) = open();
    let width = renderer.width();
    let height = renderer.height();
    let floor = scene(
        vec![Light {
            position: Vec3::new(0.0, 7.0, 0.0),
            color: [1.0, 1.0, 1.0],

            direction: Vec3::ZERO,
        }],
        Vec::new(),
        Vec::new(),
    );
    let mut cam = Camera::new(0.0, -4.0, std::f32::consts::PI);
    cam.pitch = -0.55;
    let pixels = draw(&mut window, &mut renderer, &floor, &cam);
    let step = 0.08_f32;
    let n = 8i32;
    let mut grid = Vec::new();
    for iz in 0..n {
        for ix in 0..n {
            let x = 1.2 + ix as f32 * step;
            let z = -0.28 + iz as f32 * step;
            assert!(
                on_screen(&cam, width, height, [x, 0.0, z]),
                "open-floor sample is off screen: {x} {z}"
            );
            let color = sample(&pixels, width, height, &cam, [x, 0.0, z]);
            grid.push((x, z, brightness(color)));
        }
    }
    let near = grid
        .iter()
        .min_by(|a, b| {
            let da = a.0 * a.0 + a.1 * a.1;
            let db = b.0 * b.0 + b.1 * b.1;
            da.partial_cmp(&db).unwrap()
        })
        .copied()
        .unwrap();
    let far = grid
        .iter()
        .max_by(|a, b| {
            let da = a.0 * a.0 + a.1 * a.1;
            let db = b.0 * b.0 + b.1 * b.1;
            da.partial_cmp(&db).unwrap()
        })
        .copied()
        .unwrap();
    assert!(
        near.2 > far.2,
        "brightness does not fall with distance: near {near:?} far {far:?}"
    );
    let mut rows = Vec::new();
    let mut ys = Vec::new();
    for (x, z, b) in &grid {
        let r = (x * x + z * z).sqrt();
        rows.push([1.0, r, r * r]);
        ys.push(*b);
    }
    let coeff = fit3(&rows, &ys);
    let mut lo = f32::MAX;
    let mut hi = f32::MIN;
    for (row, b) in rows.iter().zip(ys.iter()) {
        let trend = coeff[0] + coeff[1] * row[1] + coeff[2] * row[2];
        let residual = b - trend;
        lo = lo.min(residual);
        hi = hi.max(residual);
    }
    let bound = near.2 * 0.15;
    assert!(
        hi - lo < bound,
        "open floor residual is {:.1}..{:.1}, bound {bound:.1}",
        lo,
        hi
    );
    let mut peaks = 0u32;
    for iz in 1..n - 1 {
        for ix in 1..n - 1 {
            let here = grid[(iz * n + ix) as usize].2;
            let nbs = [
                grid[(iz * n + ix - 1) as usize].2,
                grid[(iz * n + ix + 1) as usize].2,
                grid[((iz - 1) * n + ix) as usize].2,
                grid[((iz + 1) * n + ix) as usize].2,
            ];
            if nbs.iter().all(|v| here > v + bound) || nbs.iter().all(|v| here + bound < *v) {
                peaks += 1;
            }
        }
    }
    assert!(peaks == 0, "open floor has {peaks} probe-scale peaks");

    let bounce = scene(
        vec![Light {
            position: Vec3::new(-5.0, 4.0, 0.0),
            color: [1.0, 1.0, 1.0],

            direction: Vec3::ZERO,
        }],
        vec![Wall {
            position: Vec3::new(3.0, 0.0, 0.0),
            half_x: 0.2,
            half_z: 2.5,
            height: 2.6,
            color: [1.0, 0.12, 0.08],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }],
        vec![Solid {
            shape: Shape::Square,
            position: Vec3::new(0.0, 0.0, 0.0),
            size: 1.2,
            height: 1.4,
            color: [0.55, 0.55, 0.55],
            absorption: 0.0,
            reflectance: -1.0,
            color_mix: -1.0,
        }],
    );
    let mut shadow_cam = Camera::new(1.4, -5.5, std::f32::consts::PI);
    shadow_cam.pitch = -0.35;
    let shadow_px = draw(&mut window, &mut renderer, &bounce, &shadow_cam);
    let mut line = Vec::new();
    for step_i in 0..8 {
        let x = 1.05 + step_i as f32 * 0.08;
        let world = [x, 0.0, 0.0];
        assert!(
            on_screen(&shadow_cam, width, height, world),
            "bounce sample is off screen: {x}"
        );
        let color = sample(&shadow_px, width, height, &shadow_cam, world);
        line.push((x, color, brightness(color)));
    }
    let lit = sample(&shadow_px, width, height, &shadow_cam, [-1.6, 0.0, 0.0]);
    let (far_x, _far_color, far_b) = line[0];
    let (near_x, near_color, near_b) = line[line.len() - 1];
    assert!(
        brightness(lit) > near_b + 20.0,
        "the shadow floor is not behind the solid: lit {lit:?} shadow {near_b}"
    );
    assert!(
        near_b + 4.0 >= far_b,
        "bounce is darker beside the wall: near x {near_x} {near_b} far x {far_x} {far_b}"
    );
    assert!(
        near_color[0] > near_color[1] && near_color[0] > near_color[2],
        "the shadow is not the wall color: {near_color:?}"
    );
    let drop = near_b - far_b;
    let mut worst = 0.0_f32;
    for pair in line.windows(2) {
        let reversal = pair[0].2 - pair[1].2;
        if reversal > worst {
            worst = reversal;
        }
    }
    if drop > 1.0 {
        assert!(
            worst < drop * 0.15,
            "bounce reverses by {worst:.1}, drop {drop:.1}, line {line:?}"
        );
    }
}

fn draw_mode(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
    wireframe: bool,
) -> Vec<u8> {
    let _ = window.pump();
    renderer
        .draw_with_overlay(world, camera, &[], true, wireframe)
        .expect("draw")
        .expect("readback")
}

#[test]
fn wireframe_draws_wall_vertices_and_keeps_a_second_face() {
    let (_gpu, mut window, mut renderer) = open();
    let width = renderer.width();
    let height = renderer.height();
    let camera = Camera::new(3.0, 0.0, -std::f32::consts::FRAC_PI_2);
    let lamp = Light {
        position: Vec3::new(2.0, 2.0, 0.0),
        color: [1.0, 1.0, 1.0],

        direction: Vec3::ZERO,
    };
    let wall = Wall {
        position: Vec3::new(0.0, 0.0, 0.0),
        half_x: 0.2,
        half_z: 1.0,
        height: 2.4,
        color: [0.9, 0.9, 0.9],
        absorption: 0.0,
        reflectance: -1.0,
        color_mix: -1.0,
    };
    let one = scene(vec![lamp.clone()], vec![wall.clone()], Vec::new());
    let face = [0.2, 1.2, 0.45];
    let inside = [0.2, 2.28, 0.0];
    let edge = [0.2, 2.4, 0.0];
    let vertex = [0.2, 2.4, 1.0];

    let filled = draw_mode(&mut window, &mut renderer, &one, &camera, false);
    let filled_face = brightness(sample(&filled, width, height, &camera, face));
    assert!(
        filled_face > 25.0,
        "the wall face is not a filled shaded surface: {filled_face}"
    );

    let lines = draw_mode(&mut window, &mut renderer, &one, &camera, true);
    let open_face = brightness(sample(&lines, width, height, &camera, face));
    let open_inside = brightness(sample(&lines, width, height, &camera, inside));
    let line = brightness(sample(&lines, width, height, &camera, edge));
    let point = brightness(sample(&lines, width, height, &camera, vertex));
    assert!(
        open_face < 8.0,
        "wireframe filled the wall face: {open_face}"
    );
    assert!(
        open_inside < 8.0,
        "wireframe filled the area beside the edge: {open_inside}"
    );
    assert!(
        line > open_face + 30.0,
        "the face edge has no line: edge {line} face {open_face}"
    );
    assert!(
        point > open_face + 20.0,
        "the corner has no point: corner {point} face {open_face}"
    );

    let two = scene(vec![lamp], vec![wall.clone(), wall], Vec::new());
    let stacked = draw_mode(&mut window, &mut renderer, &two, &camera, true);
    let stacked_line = brightness(sample(&stacked, width, height, &camera, edge));
    eprintln!(
        "wireframe filled {filled_face:.1} face {open_face:.1} inside {open_inside:.1} edge {line:.1} corner {point:.1} stacked {stacked_line:.1}"
    );
    assert!(
        stacked_line > line + 20.0,
        "a second face on the same edge is missing: one {line} two {stacked_line}"
    );
}

#[test]
fn the_shape_top_and_the_near_corridor_read_local_probes() {
    let (_gpu, mut window, mut renderer) = open();
    let width = renderer.width();
    let height = renderer.height();
    let path = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/camera/scene.rhai"
    ));
    let mut world = World::from_scene(genos_scene::load_path(path).expect("shipped scene"));
    let mut top_cam = Camera::new(0.0, 2.6, 0.0);
    top_cam.position.y = 4.2;
    top_cam.pitch = -0.95;
    let lit_px = draw(&mut window, &mut renderer, &world, &top_cam);
    let top = brightness(sample(&lit_px, width, height, &top_cam, [0.0, 1.2, 0.0]));
    let away = brightness(sample(&lit_px, width, height, &top_cam, [0.0, 0.55, 0.78]));
    world.scene.lights.clear();
    let dark_px = draw(&mut window, &mut renderer, &world, &top_cam);
    let dark_top = brightness(sample(&dark_px, width, height, &top_cam, [0.0, 1.2, 0.0]));
    eprintln!("shape top {top:.1} away {away:.1} dark {dark_top:.1}");
    assert!(
        top > dark_top + 25.0,
        "the overhead lamp left the top dark: lit {top} dark {dark_top}"
    );
    assert!(
        top > away + 15.0,
        "the top is not brighter than the unlit face: top {top} face {away}"
    );

    let mut corridor = World::from_scene(genos_scene::load_path(path).expect("shipped scene"));
    corridor.scene.lights = vec![Light {
        position: Vec3::new(0.0, 3.0, 6.5),
        color: [1.0, 1.0, 1.0],

        direction: Vec3::ZERO,
    }];
    let mut leg_cam = Camera::new(0.0, 5.1, std::f32::consts::PI);
    leg_cam.position.y = 10.0;
    leg_cam.pitch = -1.43;
    let leg_px = draw(&mut window, &mut renderer, &corridor, &leg_cam);
    let near = brightness(sample(&leg_px, width, height, &leg_cam, [0.0, 0.0, 6.35]));
    let next = brightness(sample(&leg_px, width, height, &leg_cam, [0.0, 0.0, 6.55]));
    let across = brightness(sample(&leg_px, width, height, &leg_cam, [-2.8, 0.0, 6.45]));
    eprintln!("corridor near {near:.1} next {next:.1} across {across:.1}");
    assert!(near > 20.0, "the corridor leg is black: {near}");
    assert!(next > 20.0, "the next corridor sample is black: {next}");
    let brighter = near.max(next);
    let dimmer = near.min(next);
    assert!(
        dimmer * 3.0 > brighter,
        "nearby corridor samples split: near {near} next {next}"
    );
    assert!(
        across * 2.0 < dimmer,
        "light crossed the corridor wall: across {across} leg {dimmer}"
    );
}
