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
//! Static and dynamic trees, and batch insert.

use genos_math::Vec3;
use genos_physics::{step, Body, World, GRAVITY};

fn grid(world: &mut World, count: i32, many: bool) {
    let mut bodies = Vec::new();
    for x in 0..count {
        for z in 0..count {
            bodies.push(Body::cuboid(
                Vec3::new(x as f32 * 2.0, 0.0, z as f32 * 2.0),
                0.0,
                Vec3::new(0.4, 0.4, 0.4),
            ));
        }
    }
    if many {
        world.insert_many(bodies);
    } else {
        for body in bodies {
            world.insert(body);
        }
    }
}

#[test]
fn a_dynamic_sphere_among_ten_thousand_statics_tests_a_handful_of_pairs() {
    let mut first = World::new(GRAVITY);
    grid(&mut first, 100, true);
    let sphere = first.insert(Body::sphere(Vec3::new(2.0, 0.7, 2.0), 1.0, 0.45));
    assert!(first.in_static_tree(0));
    assert!(!first.in_dynamic_tree(0));
    assert!(first.in_dynamic_tree(sphere));
    assert!(!first.in_static_tree(sphere));
    step(&mut first, 1.0 / 60.0);
    assert!(
        (1..32).contains(&first.step_stats.pair_tests),
        "pair_tests {}",
        first.step_stats.pair_tests
    );
    let pairs = first.overlapping_pairs();

    let mut second = World::new(GRAVITY);
    grid(&mut second, 100, true);
    second.insert(Body::sphere(Vec3::new(2.0, 0.7, 2.0), 1.0, 0.45));
    step(&mut second, 1.0 / 60.0);
    assert_eq!(second.overlapping_pairs(), pairs);
}

#[test]
fn batch_insert_matches_one_by_one_and_a_move_keeps_the_pair() {
    let mut batch = World::new(Vec3::ZERO);
    grid(&mut batch, 100, true);
    let dynamic = batch.insert(Body::sphere(Vec3::new(0.0, 0.7, 0.0), 1.0, 0.45));
    let batch_nodes = batch.tree_node_count();
    batch.refit();
    let batch_pairs = batch.overlapping_pairs();

    let mut single = World::new(Vec3::ZERO);
    grid(&mut single, 100, false);
    single.insert(Body::sphere(Vec3::new(0.0, 0.7, 0.0), 1.0, 0.45));
    let single_nodes = single.tree_node_count();
    assert!(
        single_nodes >= batch_nodes,
        "incremental {single_nodes} vs batch {batch_nodes}"
    );
    single.refit();
    assert_eq!(single.overlapping_pairs(), batch_pairs);

    single.set_position(dynamic, Vec3::new(6.0, 0.7, 0.0));
    let expanded = single.overlapping_pairs();
    assert!(
        expanded
            .iter()
            .any(|&(a, b)| a == dynamic as u32 || b == dynamic as u32),
        "expanded leaf lost the dynamic body"
    );
    single.refit();
    let mut fresh = World::new(Vec3::ZERO);
    grid(&mut fresh, 100, true);
    fresh.insert(Body::sphere(Vec3::new(6.0, 0.7, 0.0), 1.0, 0.45));
    fresh.refit();
    assert_eq!(single.overlapping_pairs(), fresh.overlapping_pairs());
}

#[test]
fn ten_thousand_batch_matches_one_by_one_positions() {
    fn build(many: bool) -> World {
        let mut world = World::new(Vec3::ZERO);
        let mut bodies = Vec::new();
        for index in 0..10_000 {
            let x = (index % 100) as f32;
            let z = (index / 100) as f32;
            bodies.push(Body::cuboid(
                Vec3::new(x * 2.0, 0.0, z * 2.0),
                0.0,
                Vec3::new(0.4, 0.4, 0.4),
            ));
        }
        if many {
            world.insert_many(bodies);
        } else {
            for body in bodies {
                world.insert(body);
            }
        }
        world.insert(Body::sphere(Vec3::new(4.0, 3.0, 4.0), 1.0, 0.4));
        world.refit();
        step(&mut world, 1.0 / 60.0);
        world
    }
    let mut batch = build(true);
    let mut single = build(false);
    assert_eq!(batch.overlapping_pairs(), single.overlapping_pairs());
    assert_eq!(
        batch.bodies[10_000].position,
        single.bodies[10_000].position
    );
    assert_eq!(
        batch.bodies[10_000].velocity,
        single.bodies[10_000].velocity
    );
}
