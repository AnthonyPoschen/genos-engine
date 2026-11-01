#![cfg_attr(
    test,
    expect(
        clippy::as_conversions,
        reason = "grid coordinates stay small integers"
    )
)]
#![cfg_attr(
    test,
    expect(
        clippy::cast_possible_truncation,
        reason = "grid coordinates stay small integers"
    )
)]
#![cfg_attr(
    test,
    expect(
        clippy::cast_precision_loss,
        reason = "grid coordinates stay small integers"
    )
)]
//! Deterministic scale scenes. Prints bodies and microseconds per step.

use std::time::Instant;

use genos_math::Vec3;
use genos_physics::{step, Body, World, GRAVITY};

fn bench(name: &str, mut world: World, warm: u32, measure: u32) -> u128 {
    let dt = 1.0 / 60.0;
    for _ in 0..warm {
        step(&mut world, dt);
    }
    let start = Instant::now();
    for _ in 0..measure {
        step(&mut world, dt);
    }
    let us = start.elapsed().as_micros() / u128::from(measure.max(1));
    println!(
        "{name} bodies={} us_per_step={us} pair_tests={} solver_bodies={} awake={} sleeping={} nodes={} static_rebuilds={}",
        world.bodies.len(),
        world.step_stats.pair_tests,
        world.step_stats.solver_bodies,
        world.step_stats.awake_bodies,
        world.step_stats.sleeping_bodies,
        world.step_stats.tree_nodes,
        world.static_rebuilds()
    );
    us
}

fn resting(count: i32) -> World {
    let mut world = World::new(GRAVITY);
    world.insert(Body::plane(Vec3::Y, 0.0));
    let side = (count as f32).sqrt().ceil() as i32;
    let mut placed = 0;
    for x in 0..side {
        for z in 0..side {
            if placed == count {
                return world;
            }
            world.insert(Body::sphere(
                Vec3::new(x as f32 * 1.2, 0.5, z as f32 * 1.2),
                1.0,
                0.5,
            ));
            placed += 1;
        }
    }
    world
}

fn static_grid() -> World {
    let mut world = World::new(GRAVITY);
    let mut bodies = Vec::with_capacity(10_000);
    for x in 0..100 {
        for z in 0..100 {
            bodies.push(Body::cuboid(
                Vec3::new(x as f32 * 2.0, 0.0, z as f32 * 2.0),
                0.0,
                Vec3::new(0.9, 0.5, 0.9),
            ));
        }
    }
    world.insert_many(bodies);
    world.insert(Body::sphere(Vec3::new(4.0, 3.0, 4.0), 1.0, 0.45));
    world
}

#[test]
fn scale_scenes_print_step_time() {
    let grid = static_grid();
    let us = bench("static_10k", grid, 5, 20);
    let _ = us;
    let mut check = static_grid();
    step(&mut check, 1.0 / 60.0);
    assert!(
        check.step_stats.pair_tests < 32,
        "pair_tests {}",
        check.step_stats.pair_tests
    );
    assert!(check.in_static_tree(0));
    assert!(!check.in_dynamic_tree(0));

    bench("rest_1025", resting(1024), 30, 20);

    let mut asleep = resting(64);
    for _ in 0..240 {
        step(&mut asleep, 1.0 / 60.0);
    }
    let us_asleep = bench("asleep_64", asleep, 0, 20);
    println!("asleep_64_done us={us_asleep}");
}
