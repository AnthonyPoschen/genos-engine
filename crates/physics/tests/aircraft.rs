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
    let mut driven = still_aircraft(1.0);
    let mut quiet = still_aircraft(0.0);
    for _ in 0..45 {
        step(&mut driven, DT);
        step(&mut quiet, DT);
    }
    assert!(
        !chassis(&driven).sleeping,
        "throttle held the chassis asleep"
    );
    assert!(chassis(&quiet).sleeping, "a closed throttle did not sleep");
    assert!(!chassis(&driven).motor);
    assert!(chassis(&driven).spring.is_none());
    assert_eq!(chassis(&driven).velocity, Vec3::ZERO);
    assert_eq!(chassis(&quiet).velocity, Vec3::ZERO);
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

fn still_aircraft(throttle: f32) -> World {
    let mut world = flying(
        Vec3::ZERO,
        Vec3::ZERO,
        Quat::IDENTITY,
        Some(Aircraft {
            wing_area: 10.0,
            cl: 0.0,
            cd0: 0.0,
            induced: 0.0,
            thrust_max: 0.0,
            throttle,
            air_density: 1.225,
        }),
    );
    world.sleep_time = 0.5;
    world
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
