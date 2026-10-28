#![cfg_attr(test, expect(clippy::as_conversions, reason = "pixel index from a projected point"))]
#![cfg_attr(test, expect(clippy::cast_possible_truncation, reason = "pixel index from a projected point"))]
#![cfg_attr(test, expect(clippy::cast_sign_loss, reason = "pixel index from a projected point"))]
#![cfg_attr(test, expect(clippy::expect_used, reason = "a test aborts when a fixture fails"))]
#![cfg_attr(test, expect(clippy::needless_range_loop, reason = "three channels"))]

//! Path-traced reference for the fire test in genos-render `gpu_frame.rs`
//! (`the_outside_lamp_and_the_fire_stop_at_the_corridor_wall`): how bright the
//! fire leaves the floor beside it, with the fire traced as the point lamp the
//! renderer treats it as (`scene_rays.glsl` `lamp_list_get`). Slow on the CPU, so
//! ignored; run with
//! `cargo test --release -p genos-debug --test fire_reference -- --ignored --nocapture`.
//! 28 Oct 2026, 256 spp: [44.9, 15.2, 2.3], brightness 62.4, noise 0.071.

use genos_scene::{viewport_uv, Camera, Light, Vec3};

#[test]
#[ignore = "CPU path trace, about 20 s in release"]
fn the_fire_floor_reference() {
    let mut world = genos_render::World::from_scene(
        genos_scene::load_path(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/camera/scene.rhai"
        )))
        .expect("shipped scene"),
    );
    world.scene.lights.clear();
    let mut sim = genos_render::Simulation::from_scene(&world.scene);
    for _ in 0..genos_render::PROOF_STEPS {
        sim.advance(&mut world, genos_render::STEP_DT);
    }
    let mut camera = Camera::new(0.8, 4.6, 0.0);
    camera.pitch = -0.45;
    let eye = [camera.position.x, camera.position.y, camera.position.z];
    let (fire, _) = genos_render::medium(&world, eye);
    let mut scene = world.scene.clone();
    scene.lights = vec![Light {
        position: Vec3::new(fire.position[0], fire.position[1], fire.position[2]),
        color: fire.color.map(|c| c * fire.strength),
        direction: Vec3::ZERO,
    }];
    let (width, height) = (320_u32, 180_u32);
    let setup = genos_debug::reference::RefSetup::from_camera(&scene, &camera, width, height, 256);
    let reference = genos_debug::reference::render(&setup);
    let uv = viewport_uv(&camera, 320.0 / 180.0, [0.8, 0.0, 1.8]).expect("point is on screen");
    let x = (uv[0] * 320.0).round() as u32;
    let y = (uv[1] * 180.0).round() as u32;
    let mut sum = [0.0_f32; 3];
    for py in y - 1..=y + 1 {
        for px in x - 1..=x + 1 {
            let i = ((py * width + px) * 3) as usize;
            for c in 0..3 {
                sum[c] += f32::from(reference.image.rgb[i + c]) / 9.0;
            }
        }
    }
    println!(
        "fire {fire:?} beside {sum:?} brightness {} noise {}",
        sum[0] + sum[1] + sum[2],
        reference.noise
    );
    assert!(reference.noise < 0.1, "reference too noisy: {}", reference.noise);
}
