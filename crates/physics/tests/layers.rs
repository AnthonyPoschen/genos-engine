//! Object layers drop pairs before the narrow phase.

use genos_math::Vec3;
use genos_physics::{step, Body, World};

#[test]
fn a_filtered_pair_never_reaches_the_contact_count() {
    let mut debris = World::new(Vec3::ZERO);
    debris.insert(Body::sphere(Vec3::new(0.0, 1.0, 0.0), 1.0, 0.5).with_layer(2));
    debris.insert(Body::sphere(Vec3::new(0.2, 1.0, 0.0), 1.0, 0.5).with_layer(2));
    let start = (debris.bodies[0].position, debris.bodies[1].position);
    step(&mut debris, 1.0 / 60.0);
    assert_eq!(debris.step_stats.pair_tests, 0);
    assert_eq!(debris.bodies[0].position, start.0);
    assert_eq!(debris.bodies[1].position, start.1);
    assert!(!debris.in_static_tree(0));
    assert!(debris.in_dynamic_tree(0));
    assert!(debris.in_dynamic_tree(1));

    let mut dynamic = World::new(Vec3::ZERO);
    dynamic.insert(Body::sphere(Vec3::new(0.0, 1.0, 0.0), 1.0, 0.5).with_layer(1));
    dynamic.insert(Body::sphere(Vec3::new(0.2, 1.0, 0.0), 1.0, 0.5).with_layer(1));
    step(&mut dynamic, 1.0 / 60.0);
    assert!(dynamic.step_stats.pair_tests >= 1);
    let gap = (dynamic.bodies[0].position - dynamic.bodies[1].position).length();
    assert!(gap > 0.2, "dynamic spheres stayed overlapped at {gap}");
}
