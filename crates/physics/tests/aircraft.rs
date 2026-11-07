//! A1 steady lift, drag, and thrust. The wing is not a body.
//!
//! Angle of attack is not read. Torque is zero because every force acts at
//! the centre of mass. Space thrust, gear, and sea are not in this stage.

use genos_math::{Quat, Vec3};
use genos_physics::{step, Aircraft, Body, MotionQuality, Vehicle, World, GRAVITY};

const DT: f32 = 1.0 / 60.0;
const MASS: f32 = 1_000.0;
const TRIM_STEPS: u32 = 120;

#[test]
fn a_trimmed_aircraft_holds_altitude_and_speed() {
    let mut world = trim_world();
    let mut twin = trim_world();
    let start_y = chassis(&world).position.y;
    let mut along = chassis(&world).position.x;
    for _ in 0..TRIM_STEPS {
        step(&mut world, DT);
        step(&mut twin, DT);
        let body = chassis(&world);
        along += body.velocity.x * DT;
        assert_eq!(
            body.position.y.to_bits(),
            start_y.to_bits(),
            "altitude drifted to {}",
            body.position.y
        );
        assert_eq!(body.velocity.x.to_bits(), 50.0_f32.to_bits());
        assert_eq!(body.velocity.y.to_bits(), 0.0_f32.to_bits());
        assert_eq!(body.velocity.z.to_bits(), 0.0_f32.to_bits());
        assert_eq!(body.angular_velocity, Vec3::ZERO);
        assert_eq!(world.step_stats.wheel_rays, 0);
        assert_eq!(world.step_stats.vehicle_count, 1);
        assert!(same_motion(&world, &twin), "two runs diverged");
    }
    let body = chassis(&world);
    assert_eq!(body.position.x.to_bits(), along.to_bits());
    assert!(body.spring.is_none());
    assert!(!body.motor);
    assert_eq!(body.motion, MotionQuality::Discrete);
}

#[test]
fn zero_airspeed_applies_thrust_and_no_drag_or_lift() {
    let aircraft = Aircraft {
        wing_area: 8.0,
        cl: 1.5,
        cd0: 0.3,
        induced: 0.2,
        thrust_max: 250.0,
        throttle: 1.0,
        air_density: 1.1,
    };
    let mut world = flying(GRAVITY, Vec3::ZERO, Quat::IDENTITY, Some(aircraft));
    let inverse_mass = chassis(&world).inverse_mass;
    step(&mut world, DT);
    let body = chassis(&world);
    let thrust = aircraft.throttle * aircraft.thrust_max;
    let expected = Vec3::new(thrust * inverse_mass * DT, GRAVITY.y * DT, 0.0);
    assert_eq!(body.velocity.x.to_bits(), expected.x.to_bits());
    assert_eq!(body.velocity.y.to_bits(), expected.y.to_bits());
    assert_eq!(body.velocity.z.to_bits(), expected.z.to_bits());
    assert_eq!(body.angular_velocity, Vec3::ZERO);
    assert_eq!(world.step_stats.wheel_rays, 0);
}

#[test]
fn lift_and_drag_scale_with_speed_squared() {
    let aircraft = Aircraft {
        wing_area: 10.0,
        cl: 0.5,
        cd0: 0.02,
        induced: 0.05,
        thrust_max: 100.0,
        throttle: 0.0,
        air_density: 1.25,
    };
    let slow = stepped(Vec3::new(10.0, 0.0, 0.0), Quat::IDENTITY, aircraft);
    let fast = stepped(Vec3::new(20.0, 0.0, 0.0), Quat::IDENTITY, aircraft);
    let expect_slow = predicted(Vec3::new(10.0, 0.0, 0.0), Quat::IDENTITY, aircraft);
    let expect_fast = predicted(Vec3::new(20.0, 0.0, 0.0), Quat::IDENTITY, aircraft);
    assert_eq!(slow.x.to_bits(), expect_slow.x.to_bits());
    assert_eq!(slow.y.to_bits(), expect_slow.y.to_bits());
    assert_eq!(fast.x.to_bits(), expect_fast.x.to_bits());
    assert_eq!(fast.y.to_bits(), expect_fast.y.to_bits());
    assert!(
        fast.y > slow.y * 3.5,
        "lift did not grow with speed squared"
    );
    assert!(
        (20.0 - fast.x) > (10.0 - slow.x) * 3.5,
        "drag did not grow with speed squared"
    );
}

#[test]
fn drag_opposes_velocity_and_lift_stays_world_up() {
    let aircraft = Aircraft {
        wing_area: 10.0,
        cl: 0.4,
        cd0: 0.05,
        induced: 0.0,
        thrust_max: 80.0,
        throttle: 0.0,
        air_density: 1.2,
    };
    let forward = Vec3::new(0.0, 0.0, 8.0);
    let got = stepped(forward, Quat::IDENTITY, aircraft);
    let expect = predicted(forward, Quat::IDENTITY, aircraft);
    assert_eq!(got.x.to_bits(), expect.x.to_bits());
    assert_eq!(got.y.to_bits(), expect.y.to_bits());
    assert_eq!(got.z.to_bits(), expect.z.to_bits());
    assert_eq!(got.x.to_bits(), 0.0_f32.to_bits());
    assert!(got.y > forward.y, "lift left world up");
    assert!(got.z < forward.z, "drag did not oppose +Z");

    let backward = Vec3::new(-6.0, 0.0, 0.0);
    let got = stepped(backward, Quat::IDENTITY, aircraft);
    let expect = predicted(backward, Quat::IDENTITY, aircraft);
    assert_eq!(got.x.to_bits(), expect.x.to_bits());
    assert!(got.x > backward.x, "drag did not oppose -X");
    assert!(
        got.y > 0.0,
        "lift followed the velocity instead of world up"
    );
    assert_eq!(got.z.to_bits(), 0.0_f32.to_bits());
}

#[test]
fn thrust_follows_local_plus_x() {
    let yaw = Quat::from_axis_angle(Vec3::Y, 0.7);
    let aircraft = Aircraft {
        wing_area: 4.0,
        cl: 0.0,
        cd0: 0.0,
        induced: 0.0,
        thrust_max: 250.0,
        throttle: 0.4,
        air_density: 1.225,
    };
    let mut world = flying(Vec3::ZERO, Vec3::ZERO, yaw, Some(aircraft));
    let inverse_mass = chassis(&world).inverse_mass;
    step(&mut world, DT);
    let thrust = aircraft.throttle * aircraft.thrust_max;
    let expected = (yaw.rotate(Vec3::X) * thrust * inverse_mass) * DT;
    let body = chassis(&world);
    assert_eq!(body.velocity.x.to_bits(), expected.x.to_bits());
    assert_eq!(body.velocity.y.to_bits(), expected.y.to_bits());
    assert_eq!(body.velocity.z.to_bits(), expected.z.to_bits());
    assert_eq!(body.angular_velocity, Vec3::ZERO);
    assert_ne!(expected.x.to_bits(), 0.0_f32.to_bits());
    assert_ne!(expected.z.to_bits(), 0.0_f32.to_bits());
}

#[test]
fn aircraft_force_at_the_centre_adds_no_torque() {
    let aircraft = Aircraft {
        wing_area: 12.0,
        cl: 0.8,
        cd0: 0.04,
        induced: 0.1,
        thrust_max: 400.0,
        throttle: 1.0,
        air_density: 1.225,
    };
    let spin = Vec3::new(0.3, -0.5, 0.8);
    let mut aero = oriented_spin(spin, Some(aircraft));
    let mut free = oriented_spin(spin, None);
    for _ in 0..30 {
        step(&mut aero, DT);
        step(&mut free, DT);
    }
    let left = chassis(&aero);
    let right = chassis(&free);
    assert_eq!(
        left.angular_velocity.x.to_bits(),
        right.angular_velocity.x.to_bits()
    );
    assert_eq!(
        left.angular_velocity.y.to_bits(),
        right.angular_velocity.y.to_bits()
    );
    assert_eq!(
        left.angular_velocity.z.to_bits(),
        right.angular_velocity.z.to_bits()
    );
    assert_ne!(left.velocity.x.to_bits(), right.velocity.x.to_bits());
}

#[test]
fn a_non_finite_aircraft_field_matches_a_body_with_no_aircraft() {
    let mut free = flying(GRAVITY, Vec3::new(12.0, -1.0, 3.0), Quat::IDENTITY, None);
    step(&mut free, DT);
    let plain = chassis(&free).velocity;
    let poisons: [fn(&mut Aircraft); 6] = [
        |aircraft| aircraft.cl = f32::NAN,
        |aircraft| aircraft.wing_area = f32::INFINITY,
        |aircraft| aircraft.air_density = f32::NEG_INFINITY,
        |aircraft| aircraft.throttle = f32::NAN,
        |aircraft| aircraft.thrust_max = f32::NAN,
        |aircraft| aircraft.cd0 = f32::INFINITY,
    ];
    for poison in poisons {
        let mut aircraft = finite_aircraft();
        poison(&mut aircraft);
        let mut world = flying(
            GRAVITY,
            Vec3::new(12.0, -1.0, 3.0),
            Quat::IDENTITY,
            Some(aircraft),
        );
        step(&mut world, DT);
        let velocity = chassis(&world).velocity;
        assert!(velocity.x.is_finite() && velocity.y.is_finite() && velocity.z.is_finite());
        assert_eq!(velocity.x.to_bits(), plain.x.to_bits());
        assert_eq!(velocity.y.to_bits(), plain.y.to_bits());
        assert_eq!(velocity.z.to_bits(), plain.z.to_bits());
    }
}

#[test]
fn a_zero_coefficient_aircraft_matches_a_body_with_no_vehicle() {
    let aircraft = Aircraft {
        wing_area: 10.0,
        cl: 0.0,
        cd0: 0.0,
        induced: 0.0,
        thrust_max: 0.0,
        throttle: 0.0,
        air_density: 1.225,
    };
    let velocity = Vec3::new(3.0, 1.0, -2.0);
    let mut with = flying(GRAVITY, velocity, Quat::IDENTITY, Some(aircraft));
    let mut bare = flying(GRAVITY, velocity, Quat::IDENTITY, None);
    for _ in 0..30 {
        step(&mut with, DT);
        step(&mut bare, DT);
        assert!(same_motion(&with, &bare));
        assert_eq!(with.step_stats.wheel_rays, 0);
    }
}

#[test]
fn an_absent_aircraft_payload_adds_no_force() {
    let velocity = Vec3::new(4.0, -2.0, 1.0);
    let mut stored = flying(GRAVITY, velocity, Quat::IDENTITY, None);
    stored.vehicles.push(Vehicle {
        chassis: 0,
        ground: None,
        aircraft: None,
        space: None,
    });
    let mut bare = flying(GRAVITY, velocity, Quat::IDENTITY, None);
    for _ in 0..30 {
        step(&mut stored, DT);
        step(&mut bare, DT);
        assert!(same_motion(&stored, &bare));
        assert_eq!(stored.step_stats.wheel_rays, 0);
        assert_eq!(stored.step_stats.vehicle_count, 1);
        assert_eq!(bare.step_stats.vehicle_count, 0);
    }
}

#[test]
fn a_zero_aircraft_on_the_ground_fixture_matches_ground_alone() {
    let mut with = genos_physics::chassis_rest_world(10.0);
    with.vehicles[0].aircraft = Some(Aircraft {
        wing_area: 10.0,
        cl: 0.0,
        cd0: 0.0,
        induced: 0.0,
        thrust_max: 0.0,
        throttle: 0.0,
        air_density: 1.225,
    });
    let mut bare = genos_physics::chassis_rest_world(10.0);
    for _ in 0..180 {
        step(&mut with, DT);
        step(&mut bare, DT);
        assert!(same_indexed(&with, &bare, 1));
        assert_eq!(with.step_stats.wheel_rays, bare.step_stats.wheel_rays);
    }
}

#[test]
fn throttle_above_zero_holds_the_chassis_awake() {
    let mut driven = still_aircraft(1.0, 1.0);
    let mut quiet = still_aircraft(0.0, 1.0);
    let mut dead = still_aircraft(1.0, 0.0);
    for _ in 0..45 {
        step(&mut driven, DT);
        step(&mut quiet, DT);
        step(&mut dead, DT);
    }
    assert!(
        !chassis(&driven).sleeping,
        "a 1 N thrust under the sleep gate fell asleep"
    );
    assert!(
        chassis(&driven).velocity.length() < driven.sleep_linear,
        "1 N on 1 000 kg left the sleep gate at {}",
        chassis(&driven).velocity.length()
    );
    assert!(chassis(&driven).velocity.length() < 0.25);
    assert!(chassis(&quiet).sleeping, "a closed throttle did not sleep");
    assert!(
        chassis(&dead).sleeping,
        "throttle 1 with zero thrust held the chassis awake"
    );
    assert_eq!(chassis(&dead).velocity, Vec3::ZERO);
    assert!(!chassis(&driven).motor);
    assert!(chassis(&driven).spring.is_none());
}

#[test]
fn an_asleep_aircraft_ignites_at_one_newton_and_at_one_meganewton() {
    let low = ignite_from_sleep(1.0, 180);
    let high = ignite_from_sleep(1.0e6, 5);
    assert!(
        low.speed < low.sleep_linear && low.speed < 0.25,
        "1 N reached the speed wake at {}",
        low.speed
    );
    assert!(!low.sleeping, "1 N created asleep stayed asleep");
    assert_eq!(low.velocity.to_bits(), low.expected_speed.to_bits());
    assert_eq!(low.position.to_bits(), low.expected_position.to_bits());
    assert!(!low.motor);
    assert!(low.spring_none);
    assert!(!high.sleeping, "1e6 N created asleep stayed asleep");
    assert_eq!(high.velocity.to_bits(), high.expected_speed.to_bits());
    assert_eq!(high.position.to_bits(), high.expected_position.to_bits());
    assert!(!high.motor);
}

#[test]
fn a_parked_aircraft_ignites_when_the_throttle_opens() {
    let mut world = parked_aircraft(1.0e6);
    for _ in 0..45 {
        step(&mut world, DT);
    }
    assert!(chassis(&world).sleeping, "a closed throttle did not park");
    assert_eq!(chassis(&world).velocity, Vec3::ZERO);
    assert!(
        world.vehicles[0].aircraft.is_some(),
        "parked aircraft missing"
    );
    if let Some(aircraft) = world.vehicles[0].aircraft.as_mut() {
        aircraft.throttle = 1.0;
    }
    for _ in 0..5 {
        step(&mut world, DT);
    }
    let body = chassis(&world);
    let (position, speed) = coast(1.0e6, body.inverse_mass, 5);
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
    assert_eq!(body.velocity.y.to_bits(), 0.0_f32.to_bits());
    assert!(!body.motor);
    assert!(body.spring.is_none());
}

#[test]
fn an_ignition_in_gravity_includes_the_first_fall() {
    let mut world = ignition_world(GRAVITY, 250.0, 1.0);
    world.bodies[0].sleeping = true;
    world.bodies[0].velocity = Vec3::ZERO;
    let inverse_mass = chassis(&world).inverse_mass;
    step(&mut world, DT);
    let body = chassis(&world);
    let thrust = 250.0_f32;
    assert!(!body.sleeping, "the first gravity step stayed asleep");
    assert_eq!(
        body.velocity.x.to_bits(),
        ((thrust * inverse_mass) * DT).to_bits(),
        "thrust {}",
        body.velocity.x
    );
    assert_eq!(
        body.velocity.y.to_bits(),
        (GRAVITY.y * DT).to_bits(),
        "fall {}",
        body.velocity.y
    );
    assert_eq!(body.velocity.z.to_bits(), 0.0_f32.to_bits());
    assert!(!body.motor);
}

#[test]
fn a_parked_ground_chassis_casts_again_when_aircraft_thrust_opens() {
    let mut world = genos_physics::chassis_rest_world(0.5);
    for _ in 0..180 {
        step(&mut world, DT);
    }
    let chassis_index = world.vehicles[0].chassis;
    assert!(
        world.bodies[chassis_index].sleeping,
        "the ground chassis did not park"
    );
    assert_eq!(world.step_stats.wheel_rays, 0);
    step(&mut world, DT);
    assert!(
        world.bodies[chassis_index].sleeping,
        "a parked ground chassis woke with no aircraft thrust"
    );
    assert_eq!(world.step_stats.wheel_rays, 0);
    world.vehicles[0].aircraft = Some(ignition_aircraft(1.0, 1.0));
    step(&mut world, DT);
    assert!(
        !world.bodies[chassis_index].sleeping,
        "opening aircraft thrust left the ground chassis asleep"
    );
    assert_eq!(world.step_stats.wheel_rays, 4);
    assert!(!world.bodies[chassis_index].motor);
    assert!(world.bodies[chassis_index].spring.is_none());
}

#[test]
fn an_ignition_wakes_its_island_and_leaves_another_asleep() {
    let mut world = ignition_world(Vec3::ZERO, 1_000.0, 1.0);
    world.sleep_time = 0.5;
    let neighbor = world.insert(cube_at(Vec3::new(10.0, 100.0, 0.0)));
    let far = world.insert(cube_at(Vec3::new(40.0, 100.0, 0.0)));
    world.bodies[0].sleeping = true;
    world.bodies[0].island = 7;
    world.bodies[neighbor].sleeping = true;
    world.bodies[neighbor].island = 7;
    world.bodies[neighbor].sleep_time = 0.49;
    world.bodies[far].sleeping = true;
    world.bodies[far].island = 9;
    world.bodies[far].sleep_time = 0.49;
    for _ in 0..5 {
        step(&mut world, DT);
    }
    assert!(!world.bodies[0].sleeping, "the chassis stayed asleep");
    assert!(
        !world.bodies[neighbor].sleeping,
        "the island neighbour stayed asleep"
    );
    assert_eq!(
        world.bodies[neighbor].sleep_time.to_bits(),
        (5.0 * DT).to_bits()
    );
    assert!(world.bodies[far].sleeping, "another island woke");
    assert_eq!(world.bodies[far].sleep_time.to_bits(), 0.49_f32.to_bits());
    assert_eq!(world.bodies[far].velocity, Vec3::ZERO);
    assert!(!world.bodies[0].motor);
}

#[test]
fn a_non_finite_or_zero_actuation_does_not_wake_or_hold() {
    for aircraft in [
        ignition_aircraft(1.0, 0.0),
        ignition_aircraft(1.0, -1.0),
        poisoned(|aircraft| aircraft.induced = f32::NAN),
        poisoned(|aircraft| aircraft.throttle = f32::INFINITY),
        poisoned(|aircraft| aircraft.thrust_max = f32::NAN),
        poisoned(|aircraft| aircraft.cd0 = f32::INFINITY),
    ] {
        let mut asleep = ignition_world(Vec3::ZERO, 1.0, 1.0);
        asleep.vehicles[0].aircraft = Some(aircraft);
        asleep.bodies[0].sleeping = true;
        asleep.bodies[0].sleep_time = 0.4;
        asleep.sleep_time = 0.5;
        step(&mut asleep, DT);
        assert!(
            chassis(&asleep).sleeping,
            "invalid actuation woke a sleeper: {aircraft:?}"
        );
        assert_eq!(chassis(&asleep).velocity, Vec3::ZERO);
        assert_eq!(chassis(&asleep).sleep_time.to_bits(), 0.4_f32.to_bits());

        let mut awake = ignition_world(Vec3::ZERO, 1.0, 1.0);
        awake.vehicles[0].aircraft = Some(aircraft);
        awake.sleep_time = 0.5;
        for _ in 0..45 {
            step(&mut awake, DT);
        }
        assert!(
            chassis(&awake).sleeping,
            "invalid actuation held the chassis awake: {aircraft:?}"
        );
        assert!(!chassis(&awake).motor);
    }

    let mut bare = flying(Vec3::ZERO, Vec3::ZERO, Quat::IDENTITY, None);
    bare.bodies[0].sleeping = true;
    bare.bodies[0].sleep_time = 0.4;
    let mut stored = flying(Vec3::ZERO, Vec3::ZERO, Quat::IDENTITY, None);
    stored.vehicles.push(Vehicle {
        chassis: 0,
        ground: None,
        aircraft: None,
        space: None,
    });
    stored.bodies[0].sleeping = true;
    stored.bodies[0].sleep_time = 0.4;
    for _ in 0..5 {
        step(&mut bare, DT);
        step(&mut stored, DT);
    }
    assert!(chassis(&bare).sleeping, "no vehicle woke a sleeper");
    assert!(
        chassis(&stored).sleeping,
        "an empty aircraft payload woke a sleeper"
    );
    assert_eq!(chassis(&bare).velocity, Vec3::ZERO);
    assert_eq!(chassis(&stored).velocity, Vec3::ZERO);
    assert_eq!(chassis(&bare).sleep_time.to_bits(), 0.4_f32.to_bits());
    assert_eq!(stored.step_stats.wheel_rays, 0);
}

fn trim_world() -> World {
    genos_physics::aero_trim_world()
}

fn finite_aircraft() -> Aircraft {
    Aircraft {
        wing_area: 6.0,
        cl: 0.2,
        cd0: 0.03,
        induced: 0.04,
        thrust_max: 50.0,
        throttle: 0.5,
        air_density: 1.0,
    }
}

fn flying(gravity: Vec3, velocity: Vec3, orientation: Quat, aircraft: Option<Aircraft>) -> World {
    let mut world = World::new(gravity);
    world.sleep_time = 10.0;
    let mut body = Body::cuboid(Vec3::new(0.0, 100.0, 0.0), MASS, Vec3::new(1.0, 0.2, 3.0));
    body.velocity = velocity;
    body.orientation = orientation;
    body.motion = MotionQuality::Discrete;
    let index = world.insert(body);
    if let Some(aircraft) = aircraft {
        world.vehicles.push(Vehicle {
            chassis: index,
            ground: None,
            aircraft: Some(aircraft),
            space: None,
        });
    }
    world
}

fn oriented_spin(spin: Vec3, aircraft: Option<Aircraft>) -> World {
    let mut world = World::new(Vec3::ZERO);
    world.sleep_time = 10.0;
    let mut body = Body::cuboid(Vec3::new(0.0, 20.0, 0.0), MASS, Vec3::new(0.4, 1.2, 2.5));
    body.velocity = Vec3::new(15.0, 0.0, -4.0);
    body.angular_velocity = spin;
    body.motion = MotionQuality::Discrete;
    let index = world.insert(body);
    if let Some(aircraft) = aircraft {
        world.vehicles.push(Vehicle {
            chassis: index,
            ground: None,
            aircraft: Some(aircraft),
            space: None,
        });
    }
    world
}

fn still_aircraft(throttle: f32, thrust_max: f32) -> World {
    let mut world = ignition_world(Vec3::ZERO, thrust_max, throttle);
    world.sleep_time = 0.5;
    world
}

fn ignition_aircraft(thrust_max: f32, throttle: f32) -> Aircraft {
    Aircraft {
        wing_area: 10.0,
        cl: 0.0,
        cd0: 0.0,
        induced: 0.0,
        thrust_max,
        throttle,
        air_density: 1.225,
    }
}

fn poisoned(edit: impl FnOnce(&mut Aircraft)) -> Aircraft {
    let mut aircraft = ignition_aircraft(1.0, 1.0);
    edit(&mut aircraft);
    aircraft
}

fn ignition_world(gravity: Vec3, thrust_max: f32, throttle: f32) -> World {
    flying(
        gravity,
        Vec3::ZERO,
        Quat::IDENTITY,
        Some(ignition_aircraft(thrust_max, throttle)),
    )
}

fn parked_aircraft(thrust_max: f32) -> World {
    let mut world = ignition_world(Vec3::ZERO, thrust_max, 0.0);
    world.sleep_time = 0.5;
    world
}

struct Ignition {
    sleeping: bool,
    velocity: f32,
    position: f32,
    expected_speed: f32,
    expected_position: f32,
    speed: f32,
    sleep_linear: f32,
    motor: bool,
    spring_none: bool,
}

fn ignite_from_sleep(thrust_max: f32, steps: u32) -> Ignition {
    let mut world = ignition_world(Vec3::ZERO, thrust_max, 1.0);
    world.sleep_time = 0.5;
    world.bodies[0].sleeping = true;
    world.bodies[0].sleep_time = 0.5;
    world.bodies[0].velocity = Vec3::ZERO;
    let inverse_mass = world.bodies[0].inverse_mass;
    for _ in 0..steps {
        step(&mut world, DT);
    }
    let body = world.bodies[0];
    let (expected_position, expected_speed) = coast(thrust_max, inverse_mass, steps);
    Ignition {
        sleeping: body.sleeping,
        velocity: body.velocity.x,
        position: body.position.x,
        expected_speed,
        expected_position,
        speed: body.velocity.length(),
        sleep_linear: world.sleep_linear,
        motor: body.motor,
        spring_none: body.spring.is_none(),
    }
}

fn cube_at(position: Vec3) -> Body {
    let mut body = Body::cuboid(position, MASS, Vec3::new(1.0, 0.2, 3.0));
    body.motion = MotionQuality::Discrete;
    body
}

/// `v += (thrust * inverse_mass) * dt`, then `x += v * dt`. Gravity is zero.
fn coast(thrust: f32, inverse_mass: f32, steps: u32) -> (f32, f32) {
    let mut velocity = 0.0_f32;
    let mut position = 0.0_f32;
    for _ in 0..steps {
        velocity += (thrust * inverse_mass) * DT;
        position += velocity * DT;
    }
    (position, velocity)
}

fn stepped(velocity: Vec3, orientation: Quat, aircraft: Aircraft) -> Vec3 {
    let mut world = flying(Vec3::ZERO, velocity, orientation, Some(aircraft));
    step(&mut world, DT);
    chassis(&world).velocity
}

fn predicted(velocity: Vec3, orientation: Quat, aircraft: Aircraft) -> Vec3 {
    let inverse_mass = 1.0 / MASS;
    let speed = velocity.length();
    let q = 0.5 * aircraft.air_density * (speed * speed);
    let lift = aircraft.cl * q * aircraft.wing_area;
    let cd = aircraft.cd0 + aircraft.induced * (aircraft.cl * aircraft.cl);
    let drag_mag = cd * q * aircraft.wing_area;
    let mut force =
        Vec3::Y * lift + orientation.rotate(Vec3::X) * (aircraft.throttle * aircraft.thrust_max);
    if speed > 0.0 {
        force += velocity * (-drag_mag / speed);
    }
    velocity + (force * inverse_mass) * DT
}

fn chassis(world: &World) -> Body {
    world.bodies[0]
}

fn same_motion(a: &World, b: &World) -> bool {
    same_indexed(a, b, 0)
}

fn same_indexed(a: &World, b: &World, index: usize) -> bool {
    let left = a.bodies[index];
    let right = b.bodies[index];
    left.position.x.to_bits() == right.position.x.to_bits()
        && left.position.y.to_bits() == right.position.y.to_bits()
        && left.position.z.to_bits() == right.position.z.to_bits()
        && left.velocity.x.to_bits() == right.velocity.x.to_bits()
        && left.velocity.y.to_bits() == right.velocity.y.to_bits()
        && left.velocity.z.to_bits() == right.velocity.z.to_bits()
        && left.angular_velocity.x.to_bits() == right.angular_velocity.x.to_bits()
        && left.angular_velocity.y.to_bits() == right.angular_velocity.y.to_bits()
        && left.angular_velocity.z.to_bits() == right.angular_velocity.z.to_bits()
        && left.orientation.x.to_bits() == right.orientation.x.to_bits()
        && left.orientation.y.to_bits() == right.orientation.y.to_bits()
        && left.orientation.z.to_bits() == right.orientation.z.to_bits()
        && left.orientation.w.to_bits() == right.orientation.w.to_bits()
}
