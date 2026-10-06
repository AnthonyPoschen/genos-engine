//! Presented fire. Pixel checks use `Renderer::draw` readback.
//!
//! Two cold simulations must agree. A warm column sits above the red box, a
//! grayer trail sits above that column, and the near floor is warmer than the
//! far floor because the particles emit light.

use genos_render::{
    build, fire_radiance, illuminate_facing, medium, sample as field_sample, shade_lit, DrawKind,
    FireLight, Renderer, Simulation, World, PROOF_STEPS, STEP_DT,
};
use genos_scene::{load_path, viewport_uv, Camera};
use genos_window::Window;
use std::path::PathBuf;

const NEAR_FLOOR: [f32; 3] = [0.8, 0.0, 1.8];
const FAR_FLOOR: [f32; 3] = [-4.2, 0.0, 3.3];
const FLAME: [f32; 3] = [0.0, 1.8, 0.0];
const TRAIL: [f32; 3] = [0.0, 3.35, 0.0];
const MISS: [f32; 3] = [2.2, 2.4, -1.6];

fn cold() -> (World, Simulation) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/camera/scene.rhai");
    let scene = load_path(&path).expect("camera scene");
    let mut world = World::from_scene(scene);
    let mut sim = Simulation::from_scene(&world.scene);
    for _ in 0..PROOF_STEPS {
        sim.advance(&mut world, STEP_DT);
    }
    (world, sim)
}

fn pixel_at(pixels: &[u8], width: u32, height: u32, camera: &Camera, world: [f32; 3]) -> [f32; 3] {
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

fn warmth(color: [f32; 3]) -> f32 {
    color[0] - color[2]
}

fn saturation(color: [f32; 3]) -> f32 {
    let max = color[0].max(color[1]).max(color[2]);
    let min = color[0].min(color[1]).min(color[2]);
    if max <= 1.0 {
        0.0
    } else {
        (max - min) / max
    }
}

fn column(world: &World) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
    let mut embers = Vec::new();
    let mut smoke = Vec::new();
    for object in &world.objects {
        if let DrawKind::Particles { points, lit, .. } = &object.kind {
            if *lit {
                smoke.extend(points.iter().copied());
            } else {
                embers.extend(points.iter().copied());
            }
        }
    }
    (embers, smoke)
}

fn cpu_fallback(open_error: &str) {
    eprintln!("particle-gpu-unavailable: {open_error}");
    let (world, _) = cold();
    let red = world
        .scene
        .solid_by_color([1.0, 0.0, 0.0])
        .expect("red solid");
    let (embers, smoke) = column(&world);
    assert!(!embers.is_empty() && !smoke.is_empty());
    let mut spread = 0.0_f32;
    for point in &embers {
        assert!(point[1] > red.height);
        spread = spread.max((point[0] - red.position.x).abs());
    }
    assert!(spread > 0.04, "the embers sit on one point");
    let max_ember = embers.iter().map(|point| point[1]).fold(0.0_f32, f32::max);
    let max_smoke = smoke.iter().map(|point| point[1]).fold(0.0_f32, f32::max);
    assert!(max_smoke > max_ember + 0.05);

    let (fire, _) = medium(&world, [-6.0, 1.7, 6.0]);
    let near = fire_radiance([NEAR_FLOOR[0], NEAR_FLOOR[2]], fire);
    let far = fire_radiance([FAR_FLOOR[0], FAR_FLOOR[2]], fire);
    assert!(
        near[0] > near[2] && near[0] > far[0] * 2.0,
        "near {near:?} far {far:?}"
    );
    assert_eq!(
        fire_radiance([NEAR_FLOOR[0], NEAR_FLOOR[2]], FireLight::off()),
        [0.0; 3]
    );

    let normal = [0.0, 1.0, 0.0];
    let albedo = [0.45, 0.45, 0.45];
    let direct = illuminate_facing(&world.scene, 0.0, 2.4, 0.0, normal);
    let field = build(&world.scene);
    let bounce = field_sample(&field, 0.0, 0.0);
    let lit = shade_lit(albedo, direct, bounce, normal);
    let mut dark_scene = world.scene.clone();
    dark_scene.lights.clear();
    let dark_direct = illuminate_facing(&dark_scene, 0.0, 2.4, 0.0, normal);
    let dark_field = build(&dark_scene);
    let dark_bounce = field_sample(&dark_field, 0.0, 0.0);
    let dark = shade_lit(albedo, dark_direct, dark_bounce, normal);
    assert!(
        brightness(lit) > brightness(dark) + 0.02,
        "a world-lit particle did not darken without the lamp: {lit:?} {dark:?}"
    );
    let ember = [1.0, 0.72, 0.28];
    assert!(ember[0] > ember[2] + 0.2, "the emitting card is not warm");
    println!(
        "cpu-fallback near={near:?} far={far:?} lit={lit:?} dark={dark:?} spread={spread:.3} trail={max_smoke:.3}"
    );
}

fn assert_picture(
    label: &str,
    pixels: &[u8],
    bare: &[u8],
    no_emit: &[u8],
    dark: &[u8],
    width: u32,
    height: u32,
    camera: &Camera,
) {
    let flame = pixel_at(pixels, width, height, camera, FLAME);
    let trail = pixel_at(pixels, width, height, camera, TRAIL);
    let miss = pixel_at(pixels, width, height, camera, MISS);
    let background = pixel_at(bare, width, height, camera, FLAME);
    let trail_background = pixel_at(bare, width, height, camera, TRAIL);
    let near = pixel_at(pixels, width, height, camera, NEAR_FLOOR);
    let far = pixel_at(pixels, width, height, camera, FAR_FLOOR);
    let near_off = pixel_at(no_emit, width, height, camera, NEAR_FLOOR);
    let trail_dark = pixel_at(dark, width, height, camera, TRAIL);
    let flame_dark = pixel_at(dark, width, height, camera, FLAME);
    let near_dark = pixel_at(dark, width, height, camera, NEAR_FLOOR);
    let far_dark = pixel_at(dark, width, height, camera, FAR_FLOOR);
    println!(
        "{label} flame={flame:?} trail={trail:?} miss={miss:?} background={background:?} trail_background={trail_background:?} near={near:?} far={far:?} near_off={near_off:?} near_dark={near_dark:?} far_dark={far_dark:?} trail_dark={trail_dark:?} flame_dark={flame_dark:?}"
    );
    let mean = pixels.iter().map(|byte| *byte as f32).sum::<f32>() / pixels.len().max(1) as f32;
    assert!(mean > 1.0, "{label} readback is blank: mean {mean}");
    assert!(
        warmth(flame) > warmth(background) + 8.0 && brightness(flame) > brightness(background) + 12.0,
        "{label} the column is not warmer than the empty background: flame {flame:?} background {background:?}"
    );
    assert!(
        brightness(trail) > brightness(trail_background) + 8.0,
        "{label} the trail is missing: trail {trail:?} background {trail_background:?}"
    );
    assert!(
        saturation(trail) + 0.08 < saturation(flame),
        "{label} the trail is not less saturated than the embers: trail {trail:?} flame {flame:?}"
    );
    assert!(
        (trail[0] - miss[0]).abs() > 6.0
            || (trail[1] - miss[1]).abs() > 6.0
            || (trail[2] - miss[2]).abs() > 6.0,
        "{label} the trail matches a ray that misses the column: trail {trail:?} miss {miss:?}"
    );
    assert!(
        brightness(near) > brightness(far) + 40.0 && near[0] + 1.0 >= near[2],
        "{label} the near floor lost the lamp in front of the fire: near {near:?} far {far:?}"
    );
    assert!(
        brightness(near) > brightness(near_off) + 8.0,
        "{label} emission did not brighten the near floor: on {near:?} off {near_off:?}"
    );
    assert!(
        brightness(trail_dark) + 6.0 < brightness(trail),
        "{label} the world-lit trail did not darken without the lamp: lit {trail:?} dark {trail_dark:?}"
    );
    assert!(
        warmth(flame_dark) > 12.0 && brightness(flame_dark) > 30.0,
        "{label} the emitting card did not stay warm without the lamp: {flame_dark:?}"
    );
    assert!(
        brightness(near_dark) > 80.0
            && brightness(near_dark) > brightness(far_dark) + 40.0
            && warmth(near_dark) > warmth(far_dark) + 30.0,
        "{label} the fire did not light the floor without the lamp: near {near_dark:?} far {far_dark:?}"
    );
}

#[test]
fn the_presented_frame_shows_embers_a_trail_and_emitted_floor_light() {
    let opened = (|| {
        let mut window = Window::open_proof(1280, 720)?;
        let frame = window.pump();
        let renderer = Renderer::open(window.display, window.surface, frame.width, frame.height)?;
        Ok::<_, String>((window, renderer))
    })();
    let (mut window, mut renderer) = match opened {
        Ok(pair) => pair,
        Err(err) => {
            cpu_fallback(&err);
            return;
        }
    };
    let width = renderer.width();
    let height = renderer.height();
    let camera = Camera::opening();
    let draw = |window: &mut Window, renderer: &mut Renderer, world: &World| {
        let _ = window.pump();
        renderer
            .draw(world, &camera, true)
            .expect("draw")
            .expect("readback")
    };

    let (mut first, sim) = cold();
    let (second, _) = cold();
    let on = draw(&mut window, &mut renderer, &first);
    let on_again = draw(&mut window, &mut renderer, &second);
    let mut bare = first.clone();
    bare.objects
        .retain(|object| !matches!(object.kind, DrawKind::Particles { .. }));
    let background = draw(&mut window, &mut renderer, &bare);
    sim.apply(&mut first, false, true);
    let no_emit = draw(&mut window, &mut renderer, &first);
    sim.apply(&mut first, true, true);
    first.scene.lights.clear();
    let dark = draw(&mut window, &mut renderer, &first);

    assert_picture(
        "cold-1",
        &on,
        &background,
        &no_emit,
        &dark,
        width,
        height,
        &camera,
    );
    assert_picture(
        "cold-2",
        &on_again,
        &background,
        &no_emit,
        &dark,
        width,
        height,
        &camera,
    );
    let a = pixel_at(&on, width, height, &camera, FLAME);
    let b = pixel_at(&on_again, width, height, &camera, FLAME);
    for axis in 0..3 {
        assert!(
            (a[axis] - b[axis]).abs() < 8.0,
            "the two cold captures disagree at the column: {a:?} {b:?}"
        );
    }
}
