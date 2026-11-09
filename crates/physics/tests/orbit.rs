//! S2 wide orbit on one chassis body.
//!
//! The integration state is `f64`. The chassis pose is that state minus the
//! world origin. A coast with no gravitational parameter leaves the radius.
//! A body with no orbit is not pulled.

use genos_math::Vec3;
use genos_physics::{circular_orbit_world, step, Orbit, WidePos, World, ORBIT_GM, ORBIT_RADIUS_M};

const DT: f32 = 1.0;
const STEPS: u32 = 200;

#[test]
fn a_circular_orbit_at_1e7_metres_keeps_its_radius() {
    let mut world = circular_orbit_world();
    let mut twin = circular_orbit_world();
    assert_eq!(copied_orbit(&world).gm.to_bits(), ORBIT_GM.to_bits());
    let parked = world.bodies[1].position;
    for _ in 0..STEPS {
        step(&mut world, DT);
        step(&mut twin, DT);
        assert!(same_orbit(&world, &twin), "two orbit runs diverged");
        assert_eq!(world.step_stats.wheel_rays, 0);
        assert_eq!(world.step_stats.vehicle_count, 1);
        assert!(!world.step_stats.ccd_applied);
        assert_eq!(world.bodies[1].position, parked);
        assert_eq!(world.bodies[1].velocity, Vec3::ZERO);
    }
    let orbit = copied_orbit(&world);
    let error = (radius(orbit.position) - ORBIT_RADIUS_M).abs();
    assert!(
        error <= 0.05,
        "wide radius error {error} m, position {:?}",
        orbit.position
    );
    let pose_error = (f32_radius(world.bodies[0].position) - ORBIT_RADIUS_M).abs();
    assert!(
        pose_error > 0.05,
        "f32 pose held the radius without an origin shift, error {pose_error}"
    );
    assert!(world.bodies[0].velocity.length().is_finite());
    assert!(!world.bodies[0].motor);
    assert!(world.bodies[0].spring.is_none());
    assert_eq!(world.gravity, Vec3::ZERO);

    world.shift_origin(orbit.position);
    let shifted = f32_radius(world.bodies[0].position);
    assert!(
        shifted < 1.0,
        "origin shift left the chassis {shifted} m from the origin"
    );
}

#[test]
fn a_zero_gm_coast_leaves_the_circular_radius() {
    let mut world = circular_orbit_world();
    let Some(orbit) = world
        .vehicles
        .first_mut()
        .and_then(|vehicle| vehicle.space.as_mut())
        .and_then(|space| space.orbit.as_mut())
    else {
        assert!(world
            .vehicles
            .first()
            .is_some_and(|vehicle| { vehicle.space.is_some_and(|space| space.orbit.is_some()) }));
        return;
    };
    orbit.gm = 0.0;
    let speed = radius(orbit.velocity);
    for _ in 0..STEPS {
        step(&mut world, DT);
    }
    let orbit = copied_orbit(&world);
    let error = (radius(orbit.position) - ORBIT_RADIUS_M).abs();
    assert!(
        error > 1.0e4,
        "a coast stayed on the circular radius, error {error}"
    );
    let travelled = speed * f64::from(STEPS) * f64::from(DT);
    let coast = (ORBIT_RADIUS_M * ORBIT_RADIUS_M + travelled * travelled).sqrt();
    assert!(
        (radius(orbit.position) - coast).abs() < 1.0e-2,
        "coast radius {} wanted {coast}",
        radius(orbit.position)
    );
}

#[test]
fn an_orbit_leaves_uniform_gravity_on_a_body_with_no_orbit() {
    let mut world = circular_orbit_world();
    world.gravity = genos_physics::GRAVITY;
    let start = copied_orbit(&world).position;
    step(&mut world, 1.0 / 60.0);
    let dt = 1.0_f32 / 60.0;
    let expected = genos_physics::GRAVITY.y * dt;
    assert_eq!(world.bodies[1].velocity.y.to_bits(), expected.to_bits());
    assert_eq!(world.bodies[1].velocity.x.to_bits(), 0.0_f32.to_bits());
    assert_eq!(world.gravity, genos_physics::GRAVITY);
    let error = (radius(copied_orbit(&world).position) - radius(start)).abs();
    assert!(
        error < 1.0,
        "one gravity step moved the wide radius by {error}"
    );
}

#[test]
fn a_sleeping_orbit_does_not_move() {
    let mut world = circular_orbit_world();
    let Some(orbit) = world
        .vehicles
        .first_mut()
        .and_then(|vehicle| vehicle.space.as_mut())
        .and_then(|space| space.orbit.as_mut())
    else {
        assert!(
            world
                .vehicles
                .first()
                .is_some_and(|vehicle| vehicle.space.is_some_and(|space| space.orbit.is_some())),
            "fixture stores an orbit"
        );
        return;
    };
    orbit.velocity = WidePos::ZERO;
    world.bodies[0].velocity = Vec3::ZERO;
    world.bodies[0].sleeping = true;
    let before = copied_orbit(&world).position;
    let pose = world.bodies[0].position;
    step(&mut world, DT);
    let after = copied_orbit(&world).position;
    assert_eq!(before.x.to_bits(), after.x.to_bits());
    assert_eq!(before.y.to_bits(), after.y.to_bits());
    assert_eq!(before.z.to_bits(), after.z.to_bits());
    assert!(world.bodies[0].sleeping);
    assert_eq!(world.bodies[0].position, pose);
}

#[test]
fn a_non_finite_gm_leaves_the_wide_position() {
    let mut world = circular_orbit_world();
    let Some(orbit) = world
        .vehicles
        .first_mut()
        .and_then(|vehicle| vehicle.space.as_mut())
        .and_then(|space| space.orbit.as_mut())
    else {
        assert!(
            world
                .vehicles
                .first()
                .is_some_and(|vehicle| vehicle.space.is_some_and(|space| space.orbit.is_some())),
            "fixture stores an orbit"
        );
        return;
    };
    orbit.gm = f64::NAN;
    orbit.velocity = WidePos::ZERO;
    world.bodies[0].velocity = Vec3::ZERO;
    let before = copied_orbit(&world).position;
    step(&mut world, DT);
    let after = copied_orbit(&world);
    assert_eq!(before.x.to_bits(), after.position.x.to_bits());
    assert_eq!(before.y.to_bits(), after.position.y.to_bits());
    assert_eq!(before.z.to_bits(), after.position.z.to_bits());
    assert!(after.gm.is_nan());
    assert!(world.bodies[0].position.x.is_finite());
}

fn copied_orbit(world: &World) -> Orbit {
    world
        .vehicles
        .first()
        .and_then(|vehicle| vehicle.space)
        .and_then(|space| space.orbit)
        .unwrap_or(Orbit {
            position: WidePos::ZERO,
            velocity: WidePos::ZERO,
            gm: 0.0,
        })
}

fn same_orbit(left: &World, right: &World) -> bool {
    let a = copied_orbit(left);
    let b = copied_orbit(right);
    a.position.x.to_bits() == b.position.x.to_bits()
        && a.position.y.to_bits() == b.position.y.to_bits()
        && a.position.z.to_bits() == b.position.z.to_bits()
        && a.velocity.x.to_bits() == b.velocity.x.to_bits()
        && a.velocity.y.to_bits() == b.velocity.y.to_bits()
        && a.velocity.z.to_bits() == b.velocity.z.to_bits()
        && a.gm.to_bits() == b.gm.to_bits()
        && left.bodies[0].position == right.bodies[0].position
        && left.bodies[0].velocity == right.bodies[0].velocity
}

fn radius(position: WidePos) -> f64 {
    (position.x * position.x + position.y * position.y + position.z * position.z).sqrt()
}

fn f32_radius(position: Vec3) -> f64 {
    let x = f64::from(position.x);
    let y = f64::from(position.y);
    let z = f64::from(position.z);
    (x * x + y * y + z * z).sqrt()
}
