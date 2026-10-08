use genos_physics::{step, Body, World, GRAVITY};
use genos_scene::Vec3;

#[test]
fn the_scene_crate_reads_a_physics_fall() {
    let start = Vec3::new(0.0, 5.0, 1.0);
    let mut world = World::new(GRAVITY);
    world.insert(Body::sphere(start, 3.0, 0.2));
    let dt = 0.002;
    let steps = 200;
    for _ in 0..steps {
        step(&mut world, dt);
    }
    let t = steps as f32 * dt;
    let displacement = world.bodies[0].position.y - start.y;
    let expected = 0.5 * GRAVITY.y * t * t;
    let error = (displacement - expected).abs() / expected.abs();
    assert!(
        error < 0.01,
        "displacement {displacement} expected {expected}"
    );
}
