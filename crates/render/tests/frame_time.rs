//! Equal elapsed time keeps the same motion. A bad interval does not advance.

use genos_render::{Kind, LiveParticle, Simulation, World};
use genos_scene::{load_path, update, Actions, Camera, Vec3};
use std::path::PathBuf;

fn scene() -> genos_scene::Scene {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/camera/scene.rhai");
    load_path(&path).expect("camera scene")
}

fn walk(mut camera: Camera, scene: &genos_scene::Scene, steps: u32, dt: f32) -> Camera {
    let mut scene = scene.clone();
    let actions = Actions {
        forward: 1.0,
        look_x: 0.35,
        look_y: 0.2,
        ..Actions::default()
    };
    for _ in 0..steps {
        update(&mut camera, &mut scene, &actions, dt);
    }
    camera
}

fn horizontal(a: Vec3, b: Vec3) -> f32 {
    let dx = a.x - b.x;
    let dz = a.z - b.z;
    dx.hypot(dz)
}

fn particle_key(particle: &LiveParticle) -> (u8, u32, u32, u32) {
    let kind = match particle.kind {
        Kind::Flame => 0,
        Kind::Fire => 1,
        Kind::Smoke => 2,
    };
    (
        kind,
        particle.position[0].to_bits(),
        particle.position[2].to_bits(),
        particle.position[1].to_bits(),
    )
}

fn by_place(particles: &mut [LiveParticle]) {
    particles.sort_by_key(particle_key);
}

#[test]
fn equal_elapsed_time_matches_across_frame_schedules() {
    let scene = scene();
    let start = Camera::new(-6.0, -8.0, 0.0);
    let mut coarse = start.clone();
    coarse.attach_scene(&scene);
    let mut fine = start.clone();
    fine.attach_scene(&scene);
    let coarse = walk(coarse, &scene, 60, 1.0 / 60.0);
    let fine = walk(fine, &scene, 120, 1.0 / 120.0);
    let apart = horizontal(coarse.position, fine.position);
    assert!(apart < 0.05, "horizontal travel diverged by {apart} m");
    assert!(
        (coarse.yaw - fine.yaw).abs() < 1.0e-3,
        "stick yaw {} vs {}",
        coarse.yaw,
        fine.yaw
    );
    assert!(
        (coarse.pitch - fine.pitch).abs() < 1.0e-3,
        "stick pitch {} vs {}",
        coarse.pitch,
        fine.pitch
    );

    let mut mouse_slow = Camera::new(0.0, 0.0, 0.0);
    let mut mouse_fast = Camera::new(0.0, 0.0, 0.0);
    let mut empty = genos_scene::Scene {
        floor: scene.floor.clone(),
        walls: Vec::new(),
        solids: Vec::new(),
        lights: Vec::new(),
        ceiling: None,
        sky: None,
    };
    let mouse = Actions {
        capture_click: true,
        mouse_dx: 40.0,
        mouse_dy: -12.0,
        ..Actions::default()
    };
    update(&mut mouse_slow, &mut empty, &mouse, 1.0 / 60.0);
    update(&mut mouse_fast, &mut empty, &mouse, 1.0 / 120.0);
    assert_eq!(mouse_slow.yaw, mouse_fast.yaw);
    assert_eq!(mouse_slow.pitch, mouse_fast.pitch);

    let mut world_coarse = World::from_scene(scene.clone());
    let mut world_fine = World::from_scene(scene.clone());
    let mut sim_coarse = Simulation::from_scene(&world_coarse.scene);
    let mut sim_fine = Simulation::from_scene(&world_fine.scene);
    for _ in 0..60 {
        sim_coarse.advance(&mut world_coarse, 1.0 / 60.0);
    }
    for _ in 0..120 {
        sim_fine.advance(&mut world_fine, 1.0 / 120.0);
    }
    assert_eq!(sim_coarse.born(), sim_fine.born(), "spawn counts diverged");
    let mut left = sim_coarse.live();
    let mut right = sim_fine.live();
    assert_eq!(left.len(), right.len(), "live particle counts diverged");
    by_place(&mut left);
    by_place(&mut right);
    for (coarse_particle, fine_particle) in left.iter().zip(&right) {
        assert_eq!(coarse_particle.kind, fine_particle.kind);
        let gap = horizontal(
            Vec3::new(
                coarse_particle.position[0],
                coarse_particle.position[1],
                coarse_particle.position[2],
            ),
            Vec3::new(
                fine_particle.position[0],
                fine_particle.position[1],
                fine_particle.position[2],
            ),
        );
        let dy = (coarse_particle.position[1] - fine_particle.position[1]).abs();
        assert!(
            gap < 0.05 && dy < 0.05,
            "particle moved apart: {:?} vs {:?}",
            coarse_particle.position,
            fine_particle.position
        );
    }
}

#[test]
fn a_bad_interval_does_not_advance_the_camera_or_particles() {
    let scene = scene();
    let mut camera = Camera::new(-6.0, -8.0, 0.0);
    camera.attach_scene(&scene);
    let mut sim = Simulation::from_scene(&scene);
    let mut world = World::from_scene(scene.clone());
    sim.advance(&mut world, 1.0 / 60.0);
    let born = sim.born();
    let live = sim.live();
    let position = camera.position;
    let yaw = camera.yaw;
    let pitch = camera.pitch;
    let actions = Actions {
        forward: 1.0,
        look_x: 1.0,
        look_y: 1.0,
        ..Actions::default()
    };
    for dt in [0.0, -0.25, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut step_scene = scene.clone();
        update(&mut camera, &mut step_scene, &actions, dt);
        sim.advance(&mut world, dt);
        assert_eq!(camera.position, position, "dt {dt} moved the camera");
        assert_eq!(camera.yaw, yaw, "dt {dt} changed stick yaw");
        assert_eq!(camera.pitch, pitch, "dt {dt} changed stick pitch");
        assert_eq!(sim.born(), born, "dt {dt} spawned a particle");
        assert_eq!(sim.live(), live, "dt {dt} aged or moved a particle");
    }
}
