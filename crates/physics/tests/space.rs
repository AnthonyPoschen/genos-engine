//! S1 thruster force and torque on one chassis body.
//!
//! This fixture does not store an orbit. Gravity stays `World.gravity`. The
//! fixture sets it to zero. The wide orbit is `tests/orbit.rs`.

use genos_math::{Quat, Vec3};
use genos_physics::{step, Body, MotionQuality, Presence, SpaceCraft, Thruster, Vehicle, World};

const DT: f32 = 1.0 / 60.0;
const STEPS: u32 = 60;

#[test]
fn a_thruster_in_zero_gravity_matches_force_and_torque() {
    let center = craft(&[thruster(Vec3::ZERO, Vec3::X, 1_000.0, 1.0)]);
    let mut world = space_world(Vec3::ZERO, &center);
    let mut twin = space_world(Vec3::ZERO, &center);
    for _ in 0..STEPS {
        step(&mut world, DT);
        step(&mut twin, DT);
        assert!(same_motion(&world, &twin), "two centre runs diverged");
        assert_eq!(world.step_stats.wheel_rays, 0);
        assert_eq!(world.step_stats.vehicle_count, 1);
        assert!(finite_body(&world.bodies[0]));
    }
    let body = world.bodies[0];
    assert!(
        (body.velocity.x - 1.0).abs() < 1.0e-3,
        "centre speed {}",
        body.velocity.x
    );
    assert!(body.velocity.y.abs() < 1.0e-4 && body.velocity.z.abs() < 1.0e-4);
    assert!(
        body.angular_velocity.length() < 1.0e-4,
        "centre torque {:?}",
        body.angular_velocity
    );
    assert!(!body.motor);
    assert!(body.spring.is_none());

    let offset = craft(&[thruster(Vec3::new(0.0, 2.0, 0.0), Vec3::X, 1_000.0, 1.0)]);
    let mut yawed = space_world(Vec3::ZERO, &offset);
    let mut yaw_twin = space_world(Vec3::ZERO, &offset);
    for _ in 0..STEPS {
        step(&mut yawed, DT);
        step(&mut yaw_twin, DT);
        assert!(same_motion(&yawed, &yaw_twin), "two offset runs diverged");
        assert_eq!(yawed.step_stats.wheel_rays, 0);
        assert!(finite_body(&yawed.bodies[0]));
    }
    let body = yawed.bodies[0];
    let spin = body.angular_velocity.z;
    assert!(
        (spin + 3.0).abs() / 3.0 <= 0.01,
        "offset spin {spin}, want -3"
    );
    assert!(body.angular_velocity.x.abs() < 1.0e-3);
    assert!(body.angular_velocity.y.abs() < 1.0e-3);
    assert!(
        (body.velocity.x - 1.0).abs() > 1.0e-3,
        "offset thrust stayed on world +X at {}",
        body.velocity.x
    );
    assert!(body.velocity.length().is_finite());
}

#[test]
fn symmetric_thrusters_translate_and_cancel_torque() {
    let pair = craft(&[
        thruster(Vec3::new(0.0, 1.0, 0.0), Vec3::X, 500.0, 1.0),
        thruster(Vec3::new(0.0, -1.0, 0.0), Vec3::X, 500.0, 1.0),
    ]);
    let center = craft(&[thruster(Vec3::ZERO, Vec3::X, 1_000.0, 1.0)]);
    let mut paired = space_world(Vec3::ZERO, &pair);
    let mut single = space_world(Vec3::ZERO, &center);
    for _ in 0..STEPS {
        step(&mut paired, DT);
        step(&mut single, DT);
    }
    let body = paired.bodies[0];
    let centre = single.bodies[0];
    assert!(
        (body.velocity.x - 1.0).abs() < 1.0e-3,
        "symmetric speed {}",
        body.velocity.x
    );
    assert!((body.velocity.x - centre.velocity.x).abs() < 1.0e-5);
    assert!(body.velocity.y.abs() < 1.0e-4 && body.velocity.z.abs() < 1.0e-4);
    assert_eq!(body.angular_velocity.x.to_bits(), 0.0_f32.to_bits());
    assert_eq!(body.angular_velocity.y.to_bits(), 0.0_f32.to_bits());
    assert_eq!(body.angular_velocity.z.to_bits(), 0.0_f32.to_bits());
}

#[test]
fn a_thruster_follows_local_direction_in_world() {
    let mut local_z = space_world(
        Vec3::ZERO,
        &craft(&[thruster(Vec3::ZERO, Vec3::Z, 1_000.0, 1.0)]),
    );
    for _ in 0..STEPS {
        step(&mut local_z, DT);
    }
    let body = local_z.bodies[0];
    assert!(
        (body.velocity.z - 1.0).abs() < 1.0e-3,
        "local +Z landed on {}",
        body.velocity.z
    );
    assert!(body.velocity.x.abs() < 1.0e-4 && body.velocity.y.abs() < 1.0e-4);
    assert!(body.angular_velocity.length() < 1.0e-4);

    let mut yawed = space_world(
        Vec3::ZERO,
        &craft(&[thruster(Vec3::ZERO, Vec3::X, 1_000.0, 1.0)]),
    );
    let turn = Quat::from_axis_angle(Vec3::Y, std::f32::consts::FRAC_PI_2);
    yawed.bodies[0].orientation = turn;
    for _ in 0..STEPS {
        step(&mut yawed, DT);
    }
    let body = yawed.bodies[0];
    let axis = turn.rotate(Vec3::X).normalize();
    let along = body.velocity.dot(axis);
    assert!(
        (along - 1.0).abs() < 1.0e-3,
        "yawed thrust {along} along {axis:?}, velocity {:?}",
        body.velocity
    );
    let sideways = (body.velocity - axis * along).length();
    assert!(sideways < 1.0e-4, "thrust left the body axis by {sideways}");
    assert!(
        body.velocity.x.abs() < 1.0e-3,
        "yawed thrust stayed on world +X"
    );
    assert!(body.angular_velocity.length() < 1.0e-4);
}

#[test]
fn a_zero_or_unlisted_thruster_applies_no_force() {
    let mut plain = plain_cube(Vec3::ZERO);
    let parked = [
        space_world(Vec3::ZERO, &craft(&[])),
        space_world(
            Vec3::ZERO,
            &listed(
                0,
                &[thruster(Vec3::new(0.0, 2.0, 0.0), Vec3::X, 1_000.0, 1.0)],
            ),
        ),
        space_world(
            Vec3::ZERO,
            &craft(&[thruster(Vec3::ZERO, Vec3::X, 1_000.0, 0.0)]),
        ),
        space_world(
            Vec3::ZERO,
            &craft(&[thruster(Vec3::ZERO, Vec3::X, 0.0, 1.0)]),
        ),
        space_world(
            Vec3::ZERO,
            &craft(&[thruster(Vec3::ZERO, Vec3::ZERO, 1_000.0, 1.0)]),
        ),
    ];
    for _ in 0..STEPS {
        step(&mut plain, DT);
    }
    for mut world in parked {
        for _ in 0..STEPS {
            step(&mut world, DT);
        }
        assert!(
            same_motion(&world, &plain),
            "an idle thruster moved {:?}",
            world.bodies[0].velocity
        );
        assert_eq!(world.step_stats.wheel_rays, 0);
    }

    let mut live = space_world(
        Vec3::ZERO,
        &craft(&[thruster(Vec3::ZERO, Vec3::X, 1_000.0, 1.0)]),
    );
    step(&mut live, DT);
    assert!(
        live.bodies[0].velocity.x.abs() > 1.0e-4,
        "a live thruster applied nothing"
    );
}

#[test]
fn invalid_thruster_inputs_stay_finite_and_apply_nothing() {
    let mut plain = plain_cube(Vec3::ZERO);
    let broken = [
        thruster(Vec3::ZERO, Vec3::X, 1_000.0, f32::NAN),
        thruster(Vec3::ZERO, Vec3::X, f32::INFINITY, 1.0),
        thruster(Vec3::ZERO, Vec3::new(f32::NAN, 0.0, 0.0), 1_000.0, 1.0),
        thruster(Vec3::new(f32::NAN, 0.0, 0.0), Vec3::X, 1_000.0, 1.0),
        thruster(Vec3::ZERO, Vec3::X, f32::NEG_INFINITY, 1.0),
    ];
    for _ in 0..30 {
        step(&mut plain, DT);
    }
    for thruster in broken {
        let mut world = space_world(Vec3::ZERO, &craft(&[thruster]));
        for _ in 0..30 {
            step(&mut world, DT);
        }
        let body = world.bodies[0];
        assert!(finite_body(&body), "invalid thruster produced {body:?}");
        assert!(
            same_motion(&world, &plain),
            "invalid thruster moved {:?}",
            body.velocity
        );
    }
}

#[test]
fn space_thrust_keeps_world_gravity() {
    let craft = craft(&[thruster(Vec3::ZERO, Vec3::X, 1_000.0, 1.0)]);
    let mut thrust = space_world(genos_physics::GRAVITY, &craft);
    let mut free = plain_cube(genos_physics::GRAVITY);
    let mut weightless = space_world(Vec3::ZERO, &craft);
    for _ in 0..STEPS {
        step(&mut thrust, DT);
        step(&mut free, DT);
        step(&mut weightless, DT);
    }
    let pushed = thrust.bodies[0];
    let falling = free.bodies[0];
    assert_eq!(pushed.velocity.y.to_bits(), falling.velocity.y.to_bits());
    assert_eq!(
        pushed.velocity.z.to_bits(),
        falling.velocity.z.to_bits(),
        "gravity gained a sideways component"
    );
    assert!(
        (pushed.velocity.x - weightless.bodies[0].velocity.x).abs() < 1.0e-5,
        "gravity changed the thrust axis"
    );
    assert!(
        (pushed.velocity.x - 1.0).abs() < 1.0e-3,
        "thrust along +X was {}, gravity must stay on Y",
        pushed.velocity.x
    );
    assert!(
        pushed.velocity.y < -1.0,
        "space payload cleared gravity, vy {}",
        pushed.velocity.y
    );
    assert!(pushed.angular_velocity.length() < 1.0e-4);
}

#[test]
fn cancelling_thrusters_stay_awake() {
    let cancel = craft(&[
        thruster(Vec3::ZERO, Vec3::X, 1_000.0, 1.0),
        thruster(Vec3::ZERO, Vec3::new(-1.0, 0.0, 0.0), 1_000.0, 1.0),
    ]);
    let mut awake = space_world(Vec3::ZERO, &cancel);
    awake.sleep_time = 0.5;
    let mut idle = space_world(
        Vec3::ZERO,
        &craft(&[thruster(Vec3::ZERO, Vec3::X, 1_000.0, 0.0)]),
    );
    idle.sleep_time = 0.5;
    for _ in 0..180 {
        step(&mut awake, DT);
        step(&mut idle, DT);
    }
    let body = awake.bodies[0];
    assert!(!body.sleeping, "open throttles fell asleep");
    assert_eq!(body.velocity.x.to_bits(), 0.0_f32.to_bits());
    assert_eq!(body.velocity.y.to_bits(), 0.0_f32.to_bits());
    assert_eq!(body.velocity.z.to_bits(), 0.0_f32.to_bits());
    assert_eq!(body.angular_velocity.length().to_bits(), 0.0_f32.to_bits());
    assert!(idle.bodies[0].sleeping, "a closed throttle stayed awake");
    assert!(!body.motor);
}

#[test]
fn a_zero_throttle_space_payload_matches_the_ground_chassis() {
    let mut ground = genos_physics::chassis_rest_world(10.0);
    let mut stored = genos_physics::chassis_rest_world(10.0);
    stored.vehicles[0].space = Some(craft(&[thruster(
        Vec3::new(0.0, -0.2, 0.0),
        Vec3::X,
        1_000.0,
        0.0,
    )]));
    for _ in 0..STEPS {
        step(&mut ground, DT);
        step(&mut stored, DT);
        assert!(
            same_chassis(&ground, &stored),
            "stored space payload moved the chassis"
        );
        assert_eq!(ground.step_stats.wheel_rays, stored.step_stats.wheel_rays);
    }
}

#[test]
fn an_asleep_chassis_with_an_open_thruster_wakes_and_moves() {
    let mut world = space_world(
        Vec3::ZERO,
        &craft(&[thruster(Vec3::ZERO, Vec3::X, 1_000.0, 1.0)]),
    );
    world.sleep_time = 0.5;
    world.bodies[0].sleeping = true;
    world.bodies[0].sleep_time = 0.5;
    for _ in 0..5 {
        step(&mut world, DT);
    }
    let body = world.bodies[0];
    let (position, speed) = coast(1_000.0, 0.001, 5);
    assert!(
        !body.sleeping,
        "created asleep with throttle 1 stayed asleep"
    );
    assert_eq!(
        body.velocity.x.to_bits(),
        speed.to_bits(),
        "speed {}",
        body.velocity.x
    );
    assert_eq!(body.position.x.to_bits(), position.to_bits());
    assert!(
        body.velocity.length() < 0.25,
        "five steps already passed the speed wake"
    );
    assert!(!body.motor);
    assert!(body.spring.is_none());
    assert_eq!(body.sleep_time.to_bits(), 0.0_f32.to_bits());
}

#[test]
fn a_parked_spacecraft_wakes_when_the_throttle_opens() {
    let mut world = space_world(
        Vec3::ZERO,
        &craft(&[thruster(Vec3::ZERO, Vec3::X, 1_000.0, 0.0)]),
    );
    world.sleep_time = 0.5;
    for _ in 0..180 {
        step(&mut world, DT);
    }
    assert!(world.bodies[0].sleeping, "closed throttle did not park");
    assert_eq!(world.bodies[0].velocity.x.to_bits(), 0.0_f32.to_bits());
    if let Some(space) = world.vehicles[0].space.as_mut() {
        space.thrusters[0].throttle = 1.0;
    }
    for _ in 0..5 {
        step(&mut world, DT);
    }
    let body = world.bodies[0];
    let (position, speed) = coast(1_000.0, 0.001, 5);
    assert!(
        !body.sleeping,
        "opening the throttle left the chassis asleep"
    );
    assert_eq!(
        body.velocity.x.to_bits(),
        speed.to_bits(),
        "speed {}",
        body.velocity.x
    );
    assert_eq!(body.position.x.to_bits(), position.to_bits());
    assert!(body.velocity.length() < 0.25);
    assert!(!body.motor);
}

#[test]
fn an_ignition_in_gravity_keeps_the_fall() {
    let mut world = space_world(
        genos_physics::GRAVITY,
        &craft(&[thruster(Vec3::ZERO, Vec3::X, 1_000.0, 1.0)]),
    );
    world.bodies[0].sleeping = true;
    step(&mut world, DT);
    let body = world.bodies[0];
    let along = 1_000.0 * (0.001 * DT);
    assert!(!body.sleeping, "gravity ignition stayed asleep");
    assert_eq!(
        body.velocity.x.to_bits(),
        along.to_bits(),
        "thrust {}",
        body.velocity.x
    );
    assert_eq!(
        body.velocity.y.to_bits(),
        (genos_physics::GRAVITY.y * DT).to_bits(),
        "fall {}",
        body.velocity.y
    );
    assert_eq!(body.velocity.z.to_bits(), 0.0_f32.to_bits());
    assert!(!body.motor);
}

#[test]
fn a_thrust_under_the_speed_wake_still_wakes() {
    // 1 N on 1 000 kg stays under 0.25 m/s and under the 0.12 m/s sleep gate.
    let mut world = space_world(
        Vec3::ZERO,
        &craft(&[thruster(Vec3::ZERO, Vec3::X, 1.0, 1.0)]),
    );
    world.sleep_time = 0.5;
    world.bodies[0].sleeping = true;
    for _ in 0..180 {
        step(&mut world, DT);
    }
    let body = world.bodies[0];
    let (position, speed) = coast(1.0, 0.001, 180);
    assert!(
        speed < world.sleep_linear,
        "speed {speed} crossed the sleep gate {}",
        world.sleep_linear
    );
    assert!(
        world.sleep_linear < 0.25,
        "sleep gate {} is not under the 0.25 m/s speed wake",
        world.sleep_linear
    );
    assert!(!body.sleeping, "sub-threshold thrust stayed asleep");
    assert_eq!(
        body.velocity.x.to_bits(),
        speed.to_bits(),
        "speed {}",
        body.velocity.x
    );
    assert_eq!(body.position.x.to_bits(), position.to_bits());
    assert_eq!(body.angular_velocity.length().to_bits(), 0.0_f32.to_bits());
    assert!(!body.motor);
}

#[test]
fn cancelling_thrusters_wake_a_sleeping_chassis() {
    let cancel = craft(&[
        thruster(Vec3::ZERO, Vec3::X, 1_000.0, 1.0),
        thruster(Vec3::ZERO, Vec3::new(-1.0, 0.0, 0.0), 1_000.0, 1.0),
    ]);
    let mut world = space_world(Vec3::ZERO, &cancel);
    world.sleep_time = 0.5;
    world.bodies[0].sleeping = true;
    world.bodies[0].sleep_time = 0.5;
    for _ in 0..180 {
        step(&mut world, DT);
    }
    let body = world.bodies[0];
    assert!(!body.sleeping, "cancelling thrust left the chassis asleep");
    assert_eq!(body.velocity.x.to_bits(), 0.0_f32.to_bits());
    assert_eq!(body.velocity.y.to_bits(), 0.0_f32.to_bits());
    assert_eq!(body.velocity.z.to_bits(), 0.0_f32.to_bits());
    assert_eq!(body.position.x.to_bits(), 0.0_f32.to_bits());
    assert_eq!(body.angular_velocity.length().to_bits(), 0.0_f32.to_bits());
    assert!(!body.motor);
}

#[test]
fn an_ignition_wakes_the_island_and_leaves_another_asleep() {
    let mut world = space_world(
        Vec3::ZERO,
        &craft(&[thruster(Vec3::ZERO, Vec3::X, 1_000.0, 1.0)]),
    );
    world.sleep_time = 0.5;
    let neighbor = world.insert(cube());
    let far = world.insert(cube_at(Vec3::new(30.0, 0.0, 0.0)));
    world.bodies[0].sleeping = true;
    world.bodies[0].island = 7;
    world.bodies[neighbor].position = Vec3::new(10.0, 0.0, 0.0);
    world.bodies[neighbor].sleeping = true;
    world.bodies[neighbor].island = 7;
    world.bodies[neighbor].sleep_time = 0.49;
    world.bodies[far].sleeping = true;
    world.bodies[far].island = 9;
    world.bodies[far].sleep_time = 0.49;
    for _ in 0..5 {
        step(&mut world, DT);
    }
    assert!(!world.bodies[0].sleeping, "chassis stayed asleep");
    assert!(
        !world.bodies[neighbor].sleeping,
        "island neighbour stayed asleep"
    );
    assert_eq!(
        world.bodies[neighbor].sleep_time.to_bits(),
        (5.0 * DT).to_bits()
    );
    assert!(world.bodies[far].sleeping, "the other island woke");
    assert_eq!(world.bodies[far].sleep_time.to_bits(), 0.49_f32.to_bits());
}

#[test]
fn a_thruster_that_cannot_push_leaves_the_sleeper_asleep() {
    let idle = [
        craft(&[]),
        listed(
            0,
            &[thruster(Vec3::new(0.0, 2.0, 0.0), Vec3::X, 1_000.0, 1.0)],
        ),
        craft(&[thruster(Vec3::ZERO, Vec3::X, 1_000.0, 0.0)]),
        craft(&[thruster(Vec3::ZERO, Vec3::X, 0.0, 1.0)]),
        craft(&[thruster(Vec3::ZERO, Vec3::ZERO, 1_000.0, 1.0)]),
        craft(&[thruster(Vec3::ZERO, Vec3::X, 1_000.0, f32::NAN)]),
        craft(&[thruster(Vec3::ZERO, Vec3::X, f32::INFINITY, 1.0)]),
        craft(&[thruster(Vec3::ZERO, Vec3::X, f32::NEG_INFINITY, 1.0)]),
        craft(&[thruster(
            Vec3::ZERO,
            Vec3::new(f32::NAN, 0.0, 0.0),
            1_000.0,
            1.0,
        )]),
        craft(&[thruster(
            Vec3::new(f32::NAN, 0.0, 0.0),
            Vec3::X,
            1_000.0,
            1.0,
        )]),
    ];
    for craft in idle {
        let mut world = space_world(Vec3::ZERO, &craft);
        world.sleep_time = 0.5;
        world.bodies[0].sleeping = true;
        world.bodies[0].sleep_time = 0.4;
        for _ in 0..5 {
            step(&mut world, DT);
        }
        let body = world.bodies[0];
        assert!(body.sleeping, "a dead thruster woke the chassis: {body:?}");
        assert_eq!(body.velocity.x.to_bits(), 0.0_f32.to_bits());
        assert_eq!(body.position.x.to_bits(), 0.0_f32.to_bits());
        assert!(finite_body(&body));
        assert_eq!(body.sleep_time.to_bits(), 0.4_f32.to_bits());
    }

    let mut parked = genos_physics::chassis_rest_world(0.5);
    for _ in 0..180 {
        step(&mut parked, DT);
    }
    assert!(parked.bodies[1].sleeping, "ground rest woke");
    assert_eq!(parked.step_stats.wheel_rays, 0);

    let mut missing = space_world(
        Vec3::ZERO,
        &craft(&[thruster(Vec3::ZERO, Vec3::X, 1_000.0, 1.0)]),
    );
    missing.vehicles[0].chassis = 9;
    missing.bodies[0].sleeping = true;
    missing.bodies[0].sleep_time = 0.4;
    let mut removed = space_world(
        Vec3::ZERO,
        &craft(&[thruster(Vec3::ZERO, Vec3::X, 1_000.0, 1.0)]),
    );
    removed.bodies[0].presence = Presence::Culled;
    removed.bodies[0].sleeping = true;
    let mut frozen = World::new(Vec3::ZERO);
    let mut block = cube();
    block.inverse_mass = 0.0;
    block.sleeping = true;
    let index = frozen.insert(block);
    frozen.vehicles.push(Vehicle {
        chassis: index,
        ground: None,
        aircraft: None,
        space: Some(craft(&[thruster(Vec3::ZERO, Vec3::X, 1_000.0, 1.0)])),
    });
    for _ in 0..5 {
        step(&mut missing, DT);
        step(&mut removed, DT);
        step(&mut frozen, DT);
    }
    assert!(
        missing.bodies[0].sleeping,
        "a missing chassis woke another body"
    );
    assert_eq!(missing.bodies[0].velocity.x.to_bits(), 0.0_f32.to_bits());
    assert_eq!(missing.bodies[0].sleep_time.to_bits(), 0.4_f32.to_bits());
    assert!(removed.bodies[0].sleeping, "a culled chassis woke");
    assert_eq!(removed.bodies[0].velocity.x.to_bits(), 0.0_f32.to_bits());
    assert!(frozen.bodies[0].sleeping, "a static chassis woke");
    assert_eq!(frozen.bodies[0].velocity.x.to_bits(), 0.0_f32.to_bits());
    assert_eq!(frozen.bodies[0].position.x.to_bits(), 0.0_f32.to_bits());
}

#[test]
fn two_space_runs_match_at_a_fixed_step() {
    let offset = craft(&[thruster(Vec3::new(0.0, 2.0, 0.0), Vec3::X, 1_000.0, 1.0)]);
    let mut first = space_world(Vec3::ZERO, &offset);
    let mut second = space_world(Vec3::ZERO, &offset);
    for _ in 0..STEPS {
        step(&mut first, DT);
        step(&mut second, DT);
        assert!(same_motion(&first, &second));
    }
}

fn thruster(
    local_position: Vec3,
    local_direction: Vec3,
    max_force: f32,
    throttle: f32,
) -> Thruster {
    Thruster {
        local_position,
        local_direction,
        max_force,
        throttle,
    }
}

fn craft(thrusters: &[Thruster]) -> SpaceCraft {
    listed(thrusters.len(), thrusters)
}

fn listed(count: usize, thrusters: &[Thruster]) -> SpaceCraft {
    let mut slots = [thruster(Vec3::ZERO, Vec3::X, 0.0, 0.0); 8];
    for (slot, thruster) in slots.iter_mut().zip(thrusters.iter().take(8)) {
        *slot = *thruster;
    }
    SpaceCraft {
        thrusters: slots,
        count: u8::try_from(count).unwrap_or(0),
        orbit: None,
    }
}

fn cube() -> Body {
    cube_at(Vec3::ZERO)
}

fn cube_at(position: Vec3) -> Body {
    let mut body = Body::cuboid(position, 1_000.0, Vec3::new(1.0, 1.0, 1.0));
    body.motion = MotionQuality::Discrete;
    body.friction = 0.0;
    body.restitution = 0.0;
    body
}

/// Same update as the wrench: `v += force * (inverse_mass * dt)`, then `x += v * dt`.
fn coast(force: f32, inverse_mass: f32, steps: u32) -> (f32, f32) {
    let mut velocity = 0.0_f32;
    let mut position = 0.0_f32;
    for _ in 0..steps {
        velocity += force * (inverse_mass * DT);
        position += velocity * DT;
    }
    (position, velocity)
}

fn space_world(gravity: Vec3, craft: &SpaceCraft) -> World {
    let mut world = World::new(gravity);
    world.sleep_time = 10.0;
    let index = world.insert(cube());
    world.vehicles.push(Vehicle {
        chassis: index,
        ground: None,
        aircraft: None,
        space: Some(*craft),
    });
    world
}

fn plain_cube(gravity: Vec3) -> World {
    let mut world = World::new(gravity);
    world.sleep_time = 10.0;
    world.insert(cube());
    world
}

fn finite_body(body: &Body) -> bool {
    finite_vec(body.position)
        && finite_vec(body.velocity)
        && finite_vec(body.angular_velocity)
        && body.orientation.x.is_finite()
        && body.orientation.y.is_finite()
        && body.orientation.z.is_finite()
        && body.orientation.w.is_finite()
}

fn finite_vec(value: Vec3) -> bool {
    value.x.is_finite() && value.y.is_finite() && value.z.is_finite()
}

fn same_motion(left: &World, right: &World) -> bool {
    let a = left.bodies[0];
    let b = right.bodies[0];
    same_vec(a.position, b.position)
        && same_vec(a.velocity, b.velocity)
        && same_vec(a.angular_velocity, b.angular_velocity)
        && a.orientation.x.to_bits() == b.orientation.x.to_bits()
        && a.orientation.y.to_bits() == b.orientation.y.to_bits()
        && a.orientation.z.to_bits() == b.orientation.z.to_bits()
        && a.orientation.w.to_bits() == b.orientation.w.to_bits()
}

fn same_chassis(left: &World, right: &World) -> bool {
    let a = left.bodies[1];
    let b = right.bodies[1];
    same_vec(a.position, b.position)
        && same_vec(a.velocity, b.velocity)
        && same_vec(a.angular_velocity, b.angular_velocity)
        && a.orientation.x.to_bits() == b.orientation.x.to_bits()
        && a.orientation.y.to_bits() == b.orientation.y.to_bits()
        && a.orientation.z.to_bits() == b.orientation.z.to_bits()
        && a.orientation.w.to_bits() == b.orientation.w.to_bits()
        && same_lengths(left, right)
}

fn same_lengths(left: &World, right: &World) -> bool {
    let Some(genos_physics::GroundContact::Wheeled(a)) = left.vehicles[0].ground else {
        return false;
    };
    let Some(genos_physics::GroundContact::Wheeled(b)) = right.vehicles[0].ground else {
        return false;
    };
    a.lengths
        .iter()
        .zip(b.lengths.iter())
        .all(|(x, y)| x.to_bits() == y.to_bits())
}

fn same_vec(a: Vec3, b: Vec3) -> bool {
    a.x.to_bits() == b.x.to_bits()
        && a.y.to_bits() == b.y.to_bits()
        && a.z.to_bits() == b.z.to_bits()
}
