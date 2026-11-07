//! T1 landing trial. Aircraft and wheeled ground share one chassis.
//!
//! Space combined with either payload is outside ADR 0040. A stored space
//! payload is not forced. Sea, steering, and a second integrator are not here.

use genos_math::Vec3;
use genos_physics::{
    step, Aircraft, Body, GroundContact, MotionQuality, SpaceCraft, Thruster, Vehicle, Wheel,
    WheeledGround, World, GRAVITY,
};

const DT: f32 = 1.0 / 60.0;
const TRIM_STEPS: u32 = 120;
const SHARE_STEPS: u32 = 180;
/// `0.4 - (8240.4 / 4) / k` with suspended mass 250 kg and `k = 250 * (2π * 4)²`.
const SHARED_Y: f32 = 0.386_954;

#[test]
fn a_gear_that_misses_matches_the_aircraft() {
    let mut geared = genos_physics::gear_miss_world();
    let mut twin = genos_physics::aero_trim_world();
    let start_y = chassis(&geared).position.y;
    for _ in 0..TRIM_STEPS {
        step(&mut geared, DT);
        step(&mut twin, DT);
        assert!(
            same_motion(&geared, &twin),
            "a miss diverged from the aircraft at {:?}",
            chassis(&geared).position
        );
        assert_eq!(geared.step_stats.wheel_rays, 4);
        assert_eq!(twin.step_stats.wheel_rays, 0);
        assert_eq!(chassis(&geared).position.y.to_bits(), start_y.to_bits());
    }
    let Some(GroundContact::Wheeled(ground)) = geared.vehicles[0].ground else {
        assert!(geared.vehicles[0].ground.is_some());
        return;
    };
    for length in ground.lengths {
        assert_eq!(length.to_bits(), 0.5_f32.to_bits());
    }
    assert!(!chassis(&geared).motor);
    assert!(chassis(&geared).spring.is_none());
    assert_eq!(chassis(&geared).motion, MotionQuality::Discrete);
}

#[test]
fn a_gear_on_the_plane_shares_the_weight() {
    let mut world = genos_physics::gear_plane_world();
    let mut twin = genos_physics::gear_plane_world();
    for _ in 0..SHARE_STEPS {
        step(&mut world, DT);
        step(&mut twin, DT);
        assert!(same_motion(&world, &twin), "two landing runs diverged");
        assert_eq!(world.step_stats.wheel_rays, 4);
    }
    let body = chassis(&world);
    let height_error = (body.position.y - SHARED_Y).abs();
    assert!(
        height_error < 0.02,
        "centre y {} is {height_error} m from {SHARED_Y}",
        body.position.y
    );
    assert!(
        body.velocity.y.abs() < 0.05,
        "vertical speed {}",
        body.velocity.y
    );
    assert!(
        (body.velocity.x - 20.0).abs() < 0.05,
        "forward speed {}",
        body.velocity.x
    );
    let up = body.orientation.rotate(Vec3::Y);
    let tilt = up.dot(Vec3::Y).clamp(-1.0, 1.0).acos();
    assert!(tilt < 0.01, "body up tilted {tilt} rad");
    assert_eq!(body.normal_impulse.to_bits(), 0.0_f32.to_bits());
    assert!(!body.motor);
    assert!(body.spring.is_none());
    assert!(!world.step_stats.ccd_applied);
    assert_eq!(world.step_stats.toi_failed, 0);
    assert_eq!(world.step_stats.pair_slices, 0);
}

#[test]
fn a_ground_push_does_not_change_the_airspeed_sample() {
    let mut clear = compressed_aircraft(false);
    let mut sprung = compressed_aircraft(true);
    let start = chassis(&clear);
    let Some(aircraft) = clear.vehicles[0].aircraft else {
        assert!(clear.vehicles[0].aircraft.is_some());
        return;
    };
    step(&mut clear, DT);
    step(&mut sprung, DT);
    let wing = chassis(&clear);
    let gear = chassis(&sprung);
    let wing_expect = aircraft_then_spring(&start, aircraft, false);
    let gear_expect = aircraft_then_spring(&start, aircraft, true);
    assert_eq!(wing.velocity.x.to_bits(), wing_expect.x.to_bits());
    assert_eq!(wing.velocity.y.to_bits(), wing_expect.y.to_bits());
    assert_eq!(wing.velocity.z.to_bits(), wing_expect.z.to_bits());
    assert_eq!(
        gear.velocity.x.to_bits(),
        gear_expect.x.to_bits(),
        "forward speed {} sampled a spring the aircraft step should not see",
        gear.velocity.x
    );
    assert_eq!(
        gear.velocity.y.to_bits(),
        gear_expect.y.to_bits(),
        "vertical speed {}",
        gear.velocity.y
    );
    assert_eq!(gear.velocity.z.to_bits(), gear_expect.z.to_bits());
    assert!(
        gear.velocity.y > wing.velocity.y,
        "the compressed gear did not push"
    );
    assert_eq!(gear.angular_velocity, Vec3::ZERO);
    assert_eq!(wing.angular_velocity, Vec3::ZERO);
    assert!(!gear.motor);
    assert!(gear.spring.is_none());
}

#[test]
fn a_zero_aircraft_on_the_ground_matches_the_chassis_without_one() {
    let mut with = genos_physics::chassis_rest_world(10.0);
    let mut bare = genos_physics::chassis_rest_world(10.0);
    with.vehicles[0].aircraft = Some(zero_aircraft());
    for _ in 0..30 {
        step(&mut with, DT);
        step(&mut bare, DT);
        assert!(
            same_indexed(&with, &bare, with.vehicles[0].chassis),
            "a zero aircraft payload moved the chassis"
        );
        assert_eq!(with.step_stats.wheel_rays, bare.step_stats.wheel_rays);
    }
}

#[test]
fn a_stored_space_payload_matches_the_landing_body_without_one() {
    let mut stored = genos_physics::gear_miss_world();
    let mut bare = genos_physics::gear_miss_world();
    stored.vehicles[0].space = Some(armed_space());
    for _ in 0..5 {
        step(&mut stored, DT);
        step(&mut bare, DT);
        assert!(
            same_motion(&stored, &bare),
            "a stored thruster changed the landing chassis"
        );
        assert_eq!(stored.step_stats.wheel_rays, 4);
    }
    assert!(!chassis(&stored).motor);
}

#[test]
fn a_geared_sleeper_ignites_at_one_newton() {
    let mut world = geared_ignition(1.0, 1.0);
    world.bodies[0].sleeping = true;
    world.bodies[0].sleep_time = 0.5;
    world.bodies[0].velocity = Vec3::ZERO;
    let inverse_mass = world.bodies[0].inverse_mass;
    for _ in 0..180 {
        step(&mut world, DT);
    }
    let body = chassis(&world);
    let mut expected = 0.0_f32;
    for _ in 0..180 {
        expected += (1.0 * inverse_mass) * DT;
    }
    assert!(
        body.velocity.length() < world.sleep_linear && body.velocity.length() < 0.25,
        "1 N reached the speed wake at {}",
        body.velocity.length()
    );
    assert!(!body.sleeping, "1 N on geared gear stayed asleep");
    assert_eq!(body.velocity.x.to_bits(), expected.to_bits());
    assert_eq!(world.step_stats.wheel_rays, 4);
    assert!(!body.motor);
    assert!(body.spring.is_none());
}

#[test]
fn a_geared_ignition_wakes_its_island_and_leaves_another_asleep() {
    let mut world = geared_ignition(1_000.0, 1.0);
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
    assert!(
        !world.bodies[0].sleeping,
        "the geared chassis stayed asleep"
    );
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
    assert_eq!(world.step_stats.wheel_rays, 4);
    assert!(!world.bodies[0].motor);
}

#[test]
fn a_closed_or_invalid_gear_throttle_stays_asleep_and_casts_nothing() {
    for aircraft in [zero_aircraft(), poisoned_aircraft()] {
        let mut world = geared_ignition(1.0, 1.0);
        world.vehicles[0].aircraft = Some(aircraft);
        world.bodies[0].sleeping = true;
        world.bodies[0].sleep_time = 0.4;
        step(&mut world, DT);
        assert!(
            chassis(&world).sleeping,
            "invalid gear thrust woke the chassis"
        );
        assert_eq!(chassis(&world).velocity, Vec3::ZERO);
        assert_eq!(world.step_stats.wheel_rays, 0);
        assert_eq!(chassis(&world).sleep_time.to_bits(), 0.4_f32.to_bits());
        assert!(!chassis(&world).motor);
    }
}

#[test]
fn a_parked_gear_with_a_closed_throttle_stops_casting() {
    let mut world = genos_physics::chassis_rest_world(0.5);
    world.vehicles[0].aircraft = Some(zero_aircraft());
    for _ in 0..SHARE_STEPS {
        step(&mut world, DT);
    }
    let index = world.vehicles[0].chassis;
    assert!(
        world.bodies[index].sleeping,
        "closed-throttle gear stayed awake"
    );
    assert_eq!(world.step_stats.wheel_rays, 0);
    let height_error = (world.bodies[index].position.y - 0.384_469).abs();
    assert!(
        height_error < 0.02,
        "parked gear y {} is {height_error} m from 0.384469",
        world.bodies[index].position.y
    );
}

fn trim_aircraft(mass: f32, gravity: Vec3) -> Aircraft {
    let speed = 50.0_f32;
    let wing_area = 10.0_f32;
    let air_density = 1.225_f32;
    let cd0 = 0.02_f32;
    let induced = 0.05_f32;
    let weight = mass * -gravity.y;
    let q = 0.5 * air_density * (speed * speed);
    let cl = weight / (q * wing_area);
    let cd = cd0 + induced * (cl * cl);
    Aircraft {
        wing_area,
        cl,
        cd0,
        induced,
        thrust_max: (q * wing_area) * cd,
        throttle: 1.0,
        air_density,
    }
}

fn g1_wheels(max_length: f32) -> [Wheel; 4] {
    let wheel = |local_position: Vec3| Wheel {
        local_position,
        direction: Vec3::new(0.0, -1.0, 0.0),
        rest_length: 0.4,
        min_length: 0.1,
        max_length,
        hz: 4.0,
        damping_ratio: 0.7,
    };
    [
        wheel(Vec3::new(1.0, 0.0, 0.5)),
        wheel(Vec3::new(1.0, 0.0, -0.5)),
        wheel(Vec3::new(-1.0, 0.0, 0.5)),
        wheel(Vec3::new(-1.0, 0.0, -0.5)),
    ]
}

fn compressed_aircraft(plane: bool) -> World {
    let mut world = World::new(GRAVITY);
    world.sleep_time = 10.0;
    if plane {
        world.insert(Body::plane(Vec3::Y, 0.0));
    }
    let mut body = Body::cuboid(
        Vec3::new(0.0, 0.35, 0.0),
        1_000.0,
        Vec3::new(1.2, 0.15, 0.7),
    );
    body.friction = 0.6;
    body.restitution = 0.0;
    body.motion = MotionQuality::Discrete;
    body.velocity = Vec3::new(20.0, 0.0, 0.0);
    let index = world.insert(body);
    world.vehicles.push(Vehicle {
        chassis: index,
        ground: Some(GroundContact::Wheeled(WheeledGround {
            wheels: g1_wheels(0.6),
            lengths: [0.4; 4],
        })),
        aircraft: Some(trim_aircraft(1_000.0, GRAVITY)),
        space: None,
    });
    world
}

fn zero_aircraft() -> Aircraft {
    Aircraft {
        wing_area: 10.0,
        cl: 0.0,
        cd0: 0.0,
        induced: 0.0,
        thrust_max: 0.0,
        throttle: 0.0,
        air_density: 1.225,
    }
}

fn poisoned_aircraft() -> Aircraft {
    let mut aircraft = zero_aircraft();
    aircraft.throttle = 1.0;
    aircraft.thrust_max = 1.0;
    aircraft.induced = f32::NAN;
    aircraft
}

fn geared_ignition(thrust_max: f32, throttle: f32) -> World {
    let mut world = World::new(Vec3::ZERO);
    world.sleep_time = 0.5;
    let mut body = Body::cuboid(
        Vec3::new(0.0, 100.0, 0.0),
        1_000.0,
        Vec3::new(1.0, 0.2, 3.0),
    );
    body.motion = MotionQuality::Discrete;
    let index = world.insert(body);
    world.vehicles.push(Vehicle {
        chassis: index,
        ground: Some(GroundContact::Wheeled(WheeledGround {
            wheels: g1_wheels(0.5),
            lengths: [0.4; 4],
        })),
        aircraft: Some(Aircraft {
            wing_area: 10.0,
            cl: 0.0,
            cd0: 0.0,
            induced: 0.0,
            thrust_max,
            throttle,
            air_density: 1.225,
        }),
        space: None,
    });
    world
}

fn armed_space() -> SpaceCraft {
    let mut thrusters = [Thruster {
        local_position: Vec3::ZERO,
        local_direction: Vec3::X,
        max_force: 0.0,
        throttle: 0.0,
    }; 8];
    thrusters[0] = Thruster {
        local_position: Vec3::new(0.0, 2.0, 0.0),
        local_direction: Vec3::X,
        max_force: 1_000.0,
        throttle: 1.0,
    };
    SpaceCraft {
        thrusters,
        count: 1,
    }
}

/// Aircraft force from the pre-gravity speed, then the ground push from that
/// result. A spring written first changes the sample and misses these bits.
fn aircraft_then_spring(body: &Body, aircraft: Aircraft, sprung: bool) -> Vec3 {
    let mut velocity = body.velocity + GRAVITY * DT;
    let air = velocity - GRAVITY * DT;
    let speed = air.length();
    let q = 0.5 * aircraft.air_density * (speed * speed);
    let lift = aircraft.cl * q * aircraft.wing_area;
    let cd = aircraft.cd0 + aircraft.induced * (aircraft.cl * aircraft.cl);
    let drag_mag = cd * q * aircraft.wing_area;
    let mut force = Vec3::Y * lift
        + body.orientation.rotate(Vec3::X) * (aircraft.throttle * aircraft.thrust_max);
    if speed > 0.0 {
        force += air * (-drag_mag / speed);
    }
    velocity += (force * body.inverse_mass) * DT;
    if !sprung {
        return velocity;
    }
    let direction = Vec3::new(0.0, -1.0, 0.0);
    let length = body.position.y;
    let compression = 0.4 - length;
    let suspended = body.mass() / 4.0;
    let omega = 2.0 * std::f32::consts::PI * 4.0;
    let stiffness = suspended * omega * omega;
    let damping = 2.0 * 0.7 * suspended * omega;
    let compression_rate = -direction.dot(velocity);
    let magnitude = stiffness * compression - damping * compression_rate;
    let push = -direction * magnitude;
    for _ in 0..4 {
        velocity += push * (body.inverse_mass * DT);
    }
    velocity
}

fn cube_at(position: Vec3) -> Body {
    let mut body = Body::cuboid(position, 1_000.0, Vec3::new(1.0, 0.2, 3.0));
    body.motion = MotionQuality::Discrete;
    body
}

fn chassis(world: &World) -> Body {
    let index = world.vehicles[0].chassis;
    world.bodies[index]
}

fn same_motion(a: &World, b: &World) -> bool {
    same_indexed(a, b, a.vehicles[0].chassis)
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
