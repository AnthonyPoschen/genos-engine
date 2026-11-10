//! G1 wheeled ground. The chassis is one body. The wheels are rays.
//!
//! Drive, brake, and steer are not in this stage. ADR 0040 leaves them to
//! later plan boxes, and the ground payload has no stick.

use genos_math::{Quat, Vec3};
use genos_physics::{
    step, Body, GroundContact, MotionQuality, Vehicle, Wheel, WheeledGround, World, GRAVITY,
};

const DT: f32 = 1.0 / 60.0;
const STEPS: u32 = 180;
/// `rest_length - (mass * 9.81 / 4) / k` with `k = 300 * (2π * 4)²`.
const SETTLED_Y: f32 = 0.384_469;

#[test]
fn a_chassis_rests_on_four_springs() {
    let mut world = genos_physics::chassis_rest_world(10.0);
    let mut twin = genos_physics::chassis_rest_world(10.0);
    for _ in 0..STEPS {
        step(&mut world, DT);
        step(&mut twin, DT);
        let body = chassis(&world);
        assert!(
            bounded(&body),
            "chassis left the bound at {:?}",
            body.position
        );
        assert_eq!(world.step_stats.wheel_rays, 4);
        assert_eq!(world.step_stats.vehicle_count, 1);
        assert!(same_pose(&world, &twin), "two runs diverged");
    }
    let body = chassis(&world);
    let height_error = (body.position.y - SETTLED_Y).abs();
    assert!(
        height_error < 0.02,
        "centre y {} is {height_error} m from {SETTLED_Y}",
        body.position.y
    );
    assert!(
        body.velocity.y.abs() < 0.05,
        "vertical speed {}",
        body.velocity.y
    );
    assert!(
        body.velocity.x.abs() < 0.05 && body.velocity.z.abs() < 0.05,
        "suspension walked the chassis {:?}",
        body.velocity
    );
    let up = body.orientation.rotate(Vec3::Y);
    let tilt = up.dot(Vec3::Y).clamp(-1.0, 1.0).acos();
    assert!(tilt < 0.01, "body up tilted {tilt} rad");
    assert_eq!(body.normal_impulse.to_bits(), 0.0_f32.to_bits());
    assert!(!world.step_stats.ccd_applied);
    assert_eq!(world.step_stats.toi_failed, 0);
    assert_eq!(world.step_stats.pair_slices, 0);
    assert!(body.spring.is_none());
    assert!(!body.motor);
    assert!(same_pose(&world, &twin));
}

#[test]
fn a_parked_chassis_sleeps_and_stops_casting() {
    let mut world = genos_physics::chassis_rest_world(0.5);
    for _ in 0..STEPS {
        step(&mut world, DT);
        let body = chassis(&world);
        assert!(bounded(&body), "parked chassis left the bound");
    }
    let body = chassis(&world);
    assert!(body.sleeping, "chassis stayed awake");
    let height_error = (body.position.y - SETTLED_Y).abs();
    assert!(
        height_error < 0.02,
        "sleeping centre y {} is {height_error} m from {SETTLED_Y}",
        body.position.y
    );
    assert_eq!(world.step_stats.wheel_rays, 0);
    assert_eq!(world.step_stats.vehicle_count, 1);
}

#[test]
fn a_ray_that_misses_matches_free_fall() {
    let mut sprung = high_chassis(true);
    let mut free = high_chassis(false);
    // Twenty steps fall about 0.5 m from y = 2, so the ray stays past
    // max length and the chassis shape stays clear of the plane.
    for _ in 0..20 {
        step(&mut sprung, DT);
        step(&mut free, DT);
        assert_eq!(sprung.step_stats.wheel_rays, 4);
        assert!(same_chassis_motion(&sprung, &free));
    }
    let Some(GroundContact::Wheeled(ground)) = sprung.vehicles[0].ground else {
        assert!(sprung.vehicles[0].ground.is_some());
        return;
    };
    for length in ground.lengths {
        assert_eq!(length.to_bits(), 0.6_f32.to_bits());
    }
}

#[test]
fn a_mesh_under_the_wheels_is_a_miss() {
    let mut mesh = compressed_chassis();
    mesh.insert(flat_mesh());
    let mut empty = compressed_chassis();
    let mut plane = compressed_chassis();
    plane.insert(Body::plane(Vec3::Y, 0.0));
    // The plane is inserted after the chassis, so retarget the chassis index.
    // compressed_chassis inserts the chassis first. The plane is a second body.
    step(&mut mesh, DT);
    step(&mut empty, DT);
    step(&mut plane, DT);
    assert!(
        same_chassis_motion(&mesh, &empty),
        "a mesh ray pushed the chassis"
    );
    assert!(
        plane.bodies[0].velocity.y > empty.bodies[0].velocity.y,
        "the plane ray did not push"
    );
    assert_eq!(mesh.step_stats.wheel_rays, 4);
    assert_eq!(plane.step_stats.wheel_rays, 4);
}

#[test]
fn a_box_holds_the_chassis() {
    let mut world = box_ground_world(10.0);
    for _ in 0..STEPS {
        step(&mut world, DT);
        assert!(bounded(&chassis(&world)));
        assert_eq!(world.step_stats.wheel_rays, 4);
    }
    let body = chassis(&world);
    let height_error = (body.position.y - SETTLED_Y).abs();
    assert!(
        height_error < 0.02,
        "box support centre y {} is {height_error} m from {SETTLED_Y}",
        body.position.y
    );
    assert!(
        body.velocity.y.abs() < 0.05,
        "vertical speed {}",
        body.velocity.y
    );
    assert_eq!(body.normal_impulse.to_bits(), 0.0_f32.to_bits());
}

#[test]
fn a_rotated_box_is_tested_in_its_own_frame() {
    let mut aligned = one_wheel_over_box(Some(Quat::IDENTITY));
    let mut turned = one_wheel_over_box(Some(Quat::from_axis_angle(
        Vec3::Y,
        std::f32::consts::FRAC_PI_2,
    )));
    let mut empty = one_wheel_over_box(None);
    step(&mut aligned, DT);
    step(&mut turned, DT);
    step(&mut empty, DT);
    assert!(
        aligned.bodies[0].velocity.y > empty.bodies[0].velocity.y,
        "the aligned box did not support the wheel"
    );
    assert!(
        same_chassis_motion(&turned, &empty),
        "a yawed thin box still pushed at a point outside its frame"
    );
}

#[test]
fn a_world_without_a_vehicle_casts_no_rays() {
    let mut world = World::new(GRAVITY);
    world.insert(Body::sphere(Vec3::new(0.0, 2.0, 0.0), 1.0, 0.2));
    step(&mut world, DT);
    assert_eq!(world.step_stats.wheel_rays, 0);
    assert_eq!(world.step_stats.vehicle_count, 0);
    assert_eq!(world.vehicles.len(), 0);
}

fn chassis(world: &World) -> Body {
    world.bodies[1]
}

fn bounded(body: &Body) -> bool {
    body.position.x.is_finite()
        && body.position.y.is_finite()
        && body.position.z.is_finite()
        && body.velocity.x.is_finite()
        && body.velocity.y.is_finite()
        && body.velocity.z.is_finite()
        && body.angular_velocity.x.is_finite()
        && body.angular_velocity.y.is_finite()
        && body.angular_velocity.z.is_finite()
        && body.position.y > 0.2
        && body.position.y < 1.0
        && body.velocity.length() < 5.0
        && body.angular_velocity.length() < 5.0
}

fn same_pose(a: &World, b: &World) -> bool {
    same_chassis_motion(a, b) && same_lengths(a, b)
}

fn same_chassis_motion(a: &World, b: &World) -> bool {
    let left = chassis_of(a);
    let right = chassis_of(b);
    same_vec(left.position, right.position)
        && same_vec(left.velocity, right.velocity)
        && same_vec(left.angular_velocity, right.angular_velocity)
        && left.orientation.x.to_bits() == right.orientation.x.to_bits()
        && left.orientation.y.to_bits() == right.orientation.y.to_bits()
        && left.orientation.z.to_bits() == right.orientation.z.to_bits()
        && left.orientation.w.to_bits() == right.orientation.w.to_bits()
}

fn chassis_of(world: &World) -> Body {
    world
        .vehicles
        .first()
        .and_then(|vehicle| world.bodies.get(vehicle.chassis).copied())
        .unwrap_or(world.bodies[0])
}

fn same_lengths(a: &World, b: &World) -> bool {
    match (ground(a), ground(b)) {
        (Some(left), Some(right)) => left
            .lengths
            .iter()
            .zip(right.lengths.iter())
            .all(|(x, y)| x.to_bits() == y.to_bits()),
        _ => false,
    }
}

fn ground(world: &World) -> Option<WheeledGround> {
    world
        .vehicles
        .first()
        .and_then(|vehicle| vehicle.ground)
        .map(|GroundContact::Wheeled(ground)| ground)
}

fn same_vec(a: Vec3, b: Vec3) -> bool {
    a.x.to_bits() == b.x.to_bits()
        && a.y.to_bits() == b.y.to_bits()
        && a.z.to_bits() == b.z.to_bits()
}

fn g1_wheel(local_position: Vec3) -> Wheel {
    Wheel {
        local_position,
        direction: Vec3::new(0.0, -1.0, 0.0),
        rest_length: 0.4,
        min_length: 0.1,
        max_length: 0.6,
        hz: 4.0,
        damping_ratio: 0.7,
    }
}

fn g1_vehicle(chassis: usize) -> Vehicle {
    Vehicle {
        chassis,
        ground: Some(GroundContact::Wheeled(WheeledGround {
            wheels: [
                g1_wheel(Vec3::new(1.0, 0.0, 0.5)),
                g1_wheel(Vec3::new(1.0, 0.0, -0.5)),
                g1_wheel(Vec3::new(-1.0, 0.0, 0.5)),
                g1_wheel(Vec3::new(-1.0, 0.0, -0.5)),
            ],
            lengths: [0.4, 0.4, 0.4, 0.4],
        })),
        aircraft: None,
        space: None,
    }
}

fn chassis_body(position: Vec3, mass: f32) -> Body {
    let mut body = Body::cuboid(position, mass, Vec3::new(1.2, 0.15, 0.7));
    body.friction = 0.6;
    body.restitution = 0.0;
    body.motion = MotionQuality::Discrete;
    body
}

fn high_chassis(with_plane: bool) -> World {
    let mut world = World::new(GRAVITY);
    world.sleep_time = 10.0;
    if with_plane {
        world.insert(Body::plane(Vec3::Y, 0.0));
    }
    let index = world.insert(chassis_body(Vec3::new(0.0, 2.0, 0.0), 1_200.0));
    world.vehicles.push(g1_vehicle(index));
    world
}

fn compressed_chassis() -> World {
    let mut world = World::new(GRAVITY);
    world.sleep_time = 10.0;
    let index = world.insert(chassis_body(Vec3::new(0.0, 0.35, 0.0), 1_200.0));
    world.vehicles.push(g1_vehicle(index));
    world
}

fn flat_mesh() -> Body {
    let tris = [
        [
            Vec3::new(-4.0, 0.0, -4.0),
            Vec3::new(4.0, 0.0, -4.0),
            Vec3::new(4.0, 0.0, 4.0),
        ],
        [
            Vec3::new(-4.0, 0.0, -4.0),
            Vec3::new(4.0, 0.0, 4.0),
            Vec3::new(-4.0, 0.0, 4.0),
        ],
    ];
    Body::mesh(Vec3::ZERO, 0.0, &tris)
}

fn box_ground_world(sleep_time: f32) -> World {
    let mut world = World::new(GRAVITY);
    world.sleep_time = sleep_time;
    let mut slab = Body::cuboid(Vec3::new(0.0, -0.25, 0.0), 0.0, Vec3::new(8.0, 0.25, 8.0));
    slab.friction = 0.6;
    world.insert(slab);
    let index = world.insert(chassis_body(Vec3::new(0.0, 0.45, 0.0), 1_200.0));
    world.vehicles.push(g1_vehicle(index));
    world
}

/// Chassis first, then an optional slab. The wheel sits at local x = 1.
/// The box is long in local X and thin in local Z. A quarter turn about Y
/// swaps those axes, so that wheel is outside the box.
fn one_wheel_over_box(slab: Option<Quat>) -> World {
    let mut world = World::new(GRAVITY);
    world.sleep_time = 10.0;
    let index = world.insert(chassis_body(Vec3::new(0.0, 0.35, 0.0), 1_200.0));
    let mut vehicle = g1_vehicle(index);
    if let Some(GroundContact::Wheeled(ref mut ground)) = vehicle.ground {
        let wheel = g1_wheel(Vec3::new(1.0, 0.0, 0.0));
        ground.wheels = [wheel, wheel, wheel, wheel];
    }
    world.vehicles.push(vehicle);
    if let Some(orientation) = slab {
        let mut deck = Body::cuboid(Vec3::new(0.0, -0.1, 0.0), 0.0, Vec3::new(2.0, 0.1, 0.05));
        deck.orientation = orientation;
        world.insert(deck);
    }
    world
}
