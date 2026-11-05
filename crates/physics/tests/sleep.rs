#![cfg_attr(test, expect(clippy::as_conversions, reason = "stack level is 0 or 1"))]
#![cfg_attr(
    test,
    expect(clippy::cast_precision_loss, reason = "stack level is 0 or 1")
)]
//! Touching bodies sleep and wake as one island.

use genos_math::Vec3;
use genos_physics::{step, Body, World};

const DT: f32 = 1.0 / 60.0;

fn stack(world: &mut World, x: f32) {
    for level in 0..2 {
        let mut box_body = Body::cuboid(
            Vec3::new(x, 0.5 + level as f32, 0.0),
            1.0,
            Vec3::new(0.5, 0.5, 0.5),
        );
        box_body.friction = 0.8;
        world.insert(box_body);
    }
}

#[test]
fn two_piles_sleep_on_their_own_and_a_touch_wakes_one() {
    let mut world = World::new(Vec3::new(0.0, -9.81, 0.0));
    world.insert(Body::plane(Vec3::Y, 0.0));
    stack(&mut world, 0.0);
    stack(&mut world, 6.0);
    for _ in 0..180 {
        step(&mut world, DT);
    }
    assert!(world.bodies[1].sleeping, "stack A bottom stayed awake");
    assert!(world.bodies[2].sleeping, "stack A top stayed awake");
    assert!(world.bodies[3].sleeping, "stack B bottom stayed awake");
    assert!(world.bodies[4].sleeping, "stack B top stayed awake");
    assert_eq!(world.bodies[1].island, world.bodies[2].island);
    assert_ne!(world.bodies[1].island, world.bodies[3].island);
    step(&mut world, DT);
    assert_eq!(world.step_stats.solver_bodies, 0);

    let sphere = world.insert(Body::sphere(Vec3::new(0.0, 3.0, 0.0), 0.0, 0.4));
    world.set_position(sphere, Vec3::new(0.0, 1.6, 0.0));
    step(&mut world, DT);
    assert!(!world.bodies[1].sleeping, "hit stack stayed asleep");
    assert!(!world.bodies[2].sleeping, "the neighbour stayed asleep");
    assert!(world.bodies[3].sleeping, "the far stack woke");
    assert!(world.bodies[4].sleeping, "the far stack woke");
    assert!(world.step_stats.solver_bodies >= 2);

    let far = world.bodies.len();
    world.insert(Body::sphere(Vec3::new(6.0, 3.0, 0.0), 1.0, 0.3));
    world.remove(far);
    step(&mut world, DT);
    assert!(
        world.bodies[1].sleeping || world.bodies[1].velocity.length() < 2.0,
        "remove woke the other island into a launch"
    );
    assert!(world.bodies[3].sleeping, "remove woke stack B");
}
