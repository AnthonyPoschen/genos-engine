//! Parent links. A root with zero Y keeps the picture's old vertical center.

#![expect(clippy::expect_used, reason = "a test aborts when a fixture fails")]

use genos_scene::{
    set_parent, Camera, Codimation, Easing, Floor, HierarchyError, Scene, Shape, Solid, Vec3,
};

fn solid_at(position: Vec3, yaw: f32) -> Solid {
    Solid {
        shape: Shape::Square,
        position,
        size: 1.0,
        height: 2.0,
        yaw,
        color: [1.0, 1.0, 1.0],
        absorption: 0.0,
        reflectance: -1.0,
        color_mix: -1.0,
    }
}

fn room(solids: Vec<Solid>) -> Scene {
    Scene::from_lists(
        Floor {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 4.0,
            half_z: 4.0,
            color: [1.0, 1.0, 1.0],
        },
        Vec::new(),
        solids,
        Vec::new(),
    )
}

#[test]
fn child_follows_parent() {
    let mut scene = room(vec![
        solid_at(Vec3::new(1.0, 2.0, 3.0), std::f32::consts::FRAC_PI_2),
        solid_at(Vec3::new(4.0, 0.0, 0.0), 0.0),
    ]);
    let parent = scene.solid_handle(0).expect("parent");
    let child = scene.solid_handle(1).expect("child");
    set_parent(&mut scene, child, parent).expect("link");

    let place = scene.world_point(child, scene.solid(1).position);
    // Yaw +90 about +Y sends local +X to -Z. Parent y is added.
    assert!((place.x - 1.0).abs() < 1.0e-5, "x {}", place.x);
    assert!((place.y - 2.0).abs() < 1.0e-5, "y {}", place.y);
    assert!((place.z - (3.0 - 4.0)).abs() < 1.0e-5, "z {}", place.z);
    let yaw = scene.world_yaw(child);
    assert!(
        (yaw - std::f32::consts::FRAC_PI_2).abs() < 1.0e-5,
        "yaw {yaw}"
    );

    // A root at y 0 still draws its center at half the height.
    let flat = room(vec![solid_at(Vec3::new(2.0, 0.0, -1.0), 0.4)]);
    let handle = flat.solid_handle(0).expect("flat");
    let center = flat.world_point(handle, Vec3::new(2.0, 1.0, -1.0));
    assert_eq!(center, Vec3::new(2.0, 1.0, -1.0));
}

#[test]
fn a_cycle_is_rejected() {
    let mut scene = room(vec![
        solid_at(Vec3::ZERO, 0.0),
        solid_at(Vec3::new(1.0, 0.0, 0.0), 0.0),
    ]);
    let parent = scene.solid_handle(0).expect("parent");
    let child = scene.solid_handle(1).expect("child");
    set_parent(&mut scene, child, parent).expect("first link");
    let error = set_parent(&mut scene, parent, child).expect_err("cycle");
    assert_eq!(error, HierarchyError::Cycle);
    assert_eq!(scene.cycle_rejects, 1);
    let still = scene.object_world().get::<genos_scene::Parent>(child);
    assert_eq!(still.map(|link| link.of), Some(parent));
    assert!(scene
        .object_world()
        .get::<genos_scene::Parent>(parent)
        .is_none());
}

#[test]
fn codimate_handle_names_the_solid() {
    let scene = room(vec![solid_at(Vec3::ZERO, 0.0)]);
    let handle = scene.solid_handle(0).expect("solid");
    let motion = Codimation::new(
        vec![Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0)],
        Easing::Linear,
        1.0,
    )
    .expect("path");
    let mut camera = Camera::new(0.0, 0.0, 0.0);
    assert!(camera.codimate_handle(&scene, handle, motion.clone()));
    let stale = genos_ecs::SceneHandle::new(9, 1).expect("handle");
    assert!(!camera.codimate_handle(&scene, stale, motion));
}
