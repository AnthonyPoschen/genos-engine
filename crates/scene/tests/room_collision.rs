use genos_physics::Shape as Collider;
use genos_scene::{update, Actions, Camera, Scene, Shape, CAMERA_HEIGHT};
use std::f32::consts::PI;

fn shipped() -> Scene {
    genos_scene::load_path(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/camera/scene.rhai"
    )))
    .unwrap()
}

fn placed(scene: &Scene, x: f32, z: f32, yaw: f32) -> Camera {
    let mut camera = Camera::new(x, z, yaw);
    camera.attach_scene(scene);
    camera
}

fn hold(camera: &mut Camera, _scene: &mut Scene, actions: Actions, seconds: f32) {
    let frames = (seconds * 60.0).round() as i32;
    for _ in 0..frames {
        update(camera, &actions, 1.0 / 60.0);
    }
}

fn near(got: f32, expected: f32) -> bool {
    (got - expected).abs() < 1.0e-3
}

#[test]
fn the_loaded_room_has_one_capsule_and_a_collider_on_each_object() {
    let scene = shipped();
    let camera = placed(&scene, -6.0, 6.0, 0.0);
    let bodies = &camera.physics.bodies[..camera.physics.count];
    let dynamic: Vec<_> = bodies
        .iter()
        .filter(|body| body.inverse_mass > 0.0)
        .collect();
    assert_eq!(dynamic.len(), 1, "a second dynamic body is in the room");
    assert_eq!(camera.view, bodies.len() - 1);
    let view = bodies[camera.view];
    assert!(view.motor);
    let Collider::Capsule {
        radius,
        half_height,
    } = view.shape
    else {
        panic!("the view is not a capsule");
    };
    let eye = view.position.y + radius + half_height;
    assert!((eye - camera.position.y).abs() < 1.0e-3, "eye {eye}");
    assert!((camera.position.y - CAMERA_HEIGHT).abs() < 1.0e-3);

    let floor = &scene.floor;
    assert!(
        bodies.iter().any(|body| {
            let Collider::Box { half_extents } = body.shape else {
                return false;
            };
            body.inverse_mass == 0.0
                && near(half_extents.x, floor.half_x)
                && near(half_extents.z, floor.half_z)
                && near(body.position.y + half_extents.y, 0.0)
        }),
        "the floor has no support"
    );
    for wall in &scene.walls {
        assert!(
            bodies.iter().any(|body| {
                let Collider::Box { half_extents } = body.shape else {
                    return false;
                };
                near(body.position.x, wall.position.x)
                    && near(body.position.z, wall.position.z)
                    && near(body.position.y, wall.height * 0.5)
                    && near(half_extents.x, wall.half_x)
                    && near(half_extents.z, wall.half_z)
                    && near(half_extents.y, wall.height * 0.5)
            }),
            "wall at {} {} has no box",
            wall.position.x,
            wall.position.z
        );
    }
    for solid in &scene.solids {
        let found = bodies.iter().any(|body| {
            near(body.position.x, solid.position.x)
                && near(body.position.z, solid.position.z)
                && match solid.shape {
                    Shape::Square => {
                        let Collider::Box { half_extents } = body.shape else {
                            return false;
                        };
                        let half = solid.size * 0.5;
                        near(half_extents.x, half)
                            && near(half_extents.z, half)
                            && near(half_extents.y, solid.height * 0.5)
                    }
                    Shape::Circle => {
                        matches!(body.shape, Collider::Mesh(_))
                            && body.shape.contact_pieces() >= 1
                            && body.position.y == 0.0
                    }
                }
        });
        assert!(
            found,
            "solid at {} {} has no collider",
            solid.position.x, solid.position.z
        );
    }
    assert_eq!(bodies.len(), 1 + scene.walls.len() + scene.solids.len() + 1);
}

#[test]
fn the_view_rests_near_eye_height_and_walks_on_open_floor() {
    let mut scene = shipped();
    let mut camera = placed(&scene, -6.0, 6.0, 0.0);
    hold(&mut camera, &mut scene, Actions::default(), 2.0);
    let error = (camera.position.y - CAMERA_HEIGHT).abs();
    assert!(error < 0.05, "rest eye {} error {error}", camera.position.y);
    assert!((camera.position.x + 6.0).abs() < 0.05);
    assert!((camera.position.z - 6.0).abs() < 0.05);

    let start = camera.position;
    hold(
        &mut camera,
        &mut scene,
        Actions {
            forward: 1.0,
            ..Actions::default()
        },
        0.5,
    );
    assert!(
        camera.position.z < start.z - 1.0,
        "open floor did not follow the wish, z {}",
        camera.position.z
    );
    assert!((camera.position.x - start.x).abs() < 0.05);
    let walk_error = (camera.position.y - CAMERA_HEIGHT).abs();
    assert!(walk_error < 0.05, "walk eye {}", camera.position.y);
}

#[test]
fn a_wish_into_a_wall_or_solid_does_not_cross_it() {
    let mut scene = shipped();
    let long = scene
        .walls
        .iter()
        .find(|wall| wall.half_x > wall.half_z)
        .unwrap();
    let long_x = long.position.x;
    let long_z = long.position.z;
    let long_half_z = long.half_z;
    let mut into_long = placed(&scene, long_x, long_z + long_half_z + 0.4, 0.0);
    hold(
        &mut into_long,
        &mut scene,
        Actions {
            forward: 1.0,
            ..Actions::default()
        },
        1.0,
    );
    assert!(
        into_long.position.z > long_z + long_half_z + 0.2,
        "crossed the long wall at z {}",
        into_long.position.z
    );

    let short = scene
        .walls
        .iter()
        .find(|wall| wall.half_z > wall.half_x)
        .unwrap();
    let short_x = short.position.x;
    let short_z = short.position.z;
    let short_half_x = short.half_x;
    let mut into_short = placed(&scene, short_x - short_half_x - 0.4, short_z, PI / 2.0);
    hold(
        &mut into_short,
        &mut scene,
        Actions {
            forward: 1.0,
            ..Actions::default()
        },
        1.0,
    );
    assert!(
        into_short.position.x < short_x - short_half_x - 0.2,
        "crossed the short wall at x {}",
        into_short.position.x
    );

    let solids: Vec<_> = scene
        .solids
        .iter()
        .map(|solid| (solid.position.x, solid.position.z, solid.size))
        .collect();
    for (x, z, size) in solids {
        let reach = size * 0.5 + 0.45;
        let mut camera = placed(&scene, x, z + reach, 0.0);
        let start_z = camera.position.z;
        hold(
            &mut camera,
            &mut scene,
            Actions {
                forward: 1.0,
                ..Actions::default()
            },
            1.0,
        );
        assert!(
            camera.position.z > z + size * 0.25,
            "crossed solid at {x} {z}, z {}",
            camera.position.z
        );
        assert!(
            camera.position.z < start_z,
            "wish did not move toward solid at {z}"
        );
    }
}
