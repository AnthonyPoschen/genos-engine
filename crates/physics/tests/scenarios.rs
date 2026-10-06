use genos_math::{Quat, Vec3};
use genos_physics::{step, Body, Spring, World, GRAVITY};

fn within_one_percent(got: f32, expected: f32) {
    let scale = expected.abs().max(1.0e-4);
    let error = (got - expected).abs() / scale;
    assert!(
        error < 0.01,
        "got {got} expected {expected} relative error {error}"
    );
}

#[test]
fn free_fall_matches_half_gt_squared_and_gt() {
    let start = Vec3::new(2.0, 8.0, -3.0);
    let mut world = World::new(GRAVITY);
    assert!(world.insert(Body::sphere(start, 1.5, 0.25)));
    let dt = 0.001;
    let steps = 400;
    for _ in 0..steps {
        world = step(&world, dt);
    }
    let t = steps as f32 * dt;
    let body = world.bodies[0];
    within_one_percent(body.position.y - start.y, 0.5 * GRAVITY.y * t * t);
    within_one_percent(body.velocity.y, GRAVITY.y * t);
    within_one_percent(body.position.x, start.x);
    within_one_percent(body.position.z, start.z);
}

#[test]
fn a_static_surface_bounce_keeps_restitution() {
    let impact = 6.0;
    let restitution = 0.45;
    let mut body = Body::sphere(Vec3::new(0.0, 0.5, 0.0), 1.0, 0.5);
    body.velocity = Vec3::new(0.0, -impact, 0.0);
    body.restitution = restitution;
    let mut world = World::new(GRAVITY);
    assert!(world.insert(body));
    assert!(world.insert(Body::plane(Vec3::Y, 0.0)));
    world = step(&world, 0.001);
    within_one_percent(world.bodies[0].velocity.y, restitution * impact);
}

#[test]
fn zero_restitution_does_not_separate() {
    let mut body = Body::sphere(Vec3::new(0.0, 0.5, 0.0), 1.0, 0.5);
    body.velocity = Vec3::new(0.0, -6.0, 0.0);
    body.restitution = 0.0;
    let mut world = World::new(GRAVITY);
    assert!(world.insert(body));
    assert!(world.insert(Body::plane(Vec3::Y, 0.0)));
    world = step(&world, 0.001);
    assert!(
        world.bodies[0].velocity.y <= 1.0e-4,
        "restitution 0 separated at {}",
        world.bodies[0].velocity.y
    );
}

#[test]
fn unequal_masses_conserve_normal_momentum() {
    let mut light = Body::sphere(Vec3::new(0.0, 1.0, 0.0), 1.0, 0.5);
    light.velocity = Vec3::new(2.0, 0.0, 0.0);
    light.restitution = 0.2;
    let mut heavy = Body::sphere(Vec3::new(1.0, 1.0, 0.0), 4.0, 0.5);
    heavy.restitution = 0.2;
    let before = light.mass() * light.velocity.x + heavy.mass() * heavy.velocity.x;
    let light_speed = light.velocity.x;
    let heavy_speed = heavy.velocity.x;
    let mut world = World::new(GRAVITY);
    assert!(world.insert(light));
    assert!(world.insert(heavy));
    world = step(&world, 1.0 / 60.0);
    let a = world.bodies[0];
    let b = world.bodies[1];
    let after = a.mass() * a.velocity.x + b.mass() * b.velocity.x;
    within_one_percent(after, before);
    let light_change = (a.velocity.x - light_speed).abs();
    let heavy_change = (b.velocity.x - heavy_speed).abs();
    assert!(
        light_change > heavy_change,
        "lighter change {light_change} heavier change {heavy_change}"
    );
}

#[test]
fn higher_friction_removes_more_tangential_speed() {
    let none = tangential_after(0.0);
    let low = tangential_after(0.15);
    let high = tangential_after(0.9);
    within_one_percent(none, 4.0);
    assert!(
        high < low,
        "high friction left {high}, low friction left {low}"
    );
    assert!(
        low < none,
        "low friction left {low}, zero friction left {none}"
    );
}

fn tangential_after(friction: f32) -> f32 {
    let mut body = Body::sphere(Vec3::new(0.0, 0.5, 0.0), 1.0, 0.5);
    body.velocity = Vec3::new(4.0, -2.0, 0.0);
    body.friction = friction;
    body.restitution = 0.0;
    let mut world = World::new(GRAVITY);
    assert!(world.insert(body));
    assert!(world.insert(Body::plane(Vec3::Y, 0.0)));
    step(&world, 1.0 / 120.0).bodies[0].velocity.x
}

#[test]
fn a_spring_rests_at_mg_over_k_and_does_not_pass_through() {
    let mass = 1.0;
    let stiffness = 80.0;
    let rest = 1.0;
    let radius = 0.05;
    let target = mass * -GRAVITY.y / stiffness;
    let orientation = Quat::from_axis_angle(Vec3::Y, 0.4);
    let mut body = Body::sphere(Vec3::new(0.0, rest, 0.0), mass, radius);
    body.orientation = orientation;
    body.spring = Some(Spring {
        anchor: Vec3::ZERO,
        rest_length: rest,
        stiffness,
        damping: 25.0,
    });
    let mut world = World::new(GRAVITY);
    assert!(world.insert(body));
    assert!(world.insert(Body::plane(Vec3::Y, 0.0)));
    let mut lowest = rest;
    let dt = 0.001;
    for _ in 0..4_000 {
        world = step(&world, dt);
        lowest = lowest.min(world.bodies[0].position.y);
    }
    let body = world.bodies[0];
    let compression = rest - body.position.y;
    within_one_percent(compression, target);
    assert!(
        body.velocity.length() < 0.02,
        "still moving {}",
        body.velocity.length()
    );
    assert!(lowest >= radius - 0.01, "passed the support at y {lowest}");
    assert!((body.orientation.w - orientation.w).abs() < 1.0e-5);
    assert!((body.orientation.y - orientation.y).abs() < 1.0e-5);
}

#[test]
fn a_spring_released_away_from_balance_moves_back() {
    let mass = 1.0;
    let stiffness = 80.0;
    let rest = 1.0;
    let target = mass * -GRAVITY.y / stiffness;
    let start_y = rest - target * 2.0;
    let mut body = Body::sphere(Vec3::new(0.0, start_y, 0.0), mass, 0.05);
    body.orientation = Quat::from_axis_angle(Vec3::X, 0.2);
    body.spring = Some(Spring {
        anchor: Vec3::ZERO,
        rest_length: rest,
        stiffness,
        damping: 25.0,
    });
    let mut world = World::new(GRAVITY);
    assert!(world.insert(body));
    assert!(world.insert(Body::plane(Vec3::Y, 0.0)));
    let start_error = ((rest - start_y) - target).abs();
    for _ in 0..200 {
        world = step(&world, 0.001);
    }
    let compression = rest - world.bodies[0].position.y;
    let error = (compression - target).abs();
    assert!(
        error < start_error,
        "error {error} did not fall from {start_error}"
    );
}
