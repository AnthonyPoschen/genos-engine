//! Domain forces on one chassis body.
//!
//! The record holds a ground payload, an aircraft payload, and a space payload.
//! This stage applies the wheeled ground spring-damper, steady aircraft lift,
//! drag, and thrust, including both on one landing chassis, and the space
//! thrusters. Sea is not a field.
//! A thruster writes force and torque through [`integrate_wrench`]. The existing
//! gyroscopic step then consumes that angular velocity. There is no second integrator.

use genos_math::{Quat, Vec3};

use crate::solver::world_inverse_inertia;
use crate::{Body, MotionQuality, Presence, Shape, World, GRAVITY};

/// Extra state on one chassis [`Body`](crate::Body). The chassis index is the
/// `usize` from [`World::insert`](crate::World::insert).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vehicle {
    pub chassis: usize,
    pub ground: Option<GroundContact>,
    pub aircraft: Option<Aircraft>,
    pub space: Option<SpaceCraft>,
}

/// Ground support. Wheels exist only here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GroundContact {
    Wheeled(WheeledGround),
}

/// One suspension ray. `direction` is in the chassis frame, pointing from the
/// chassis toward the surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wheel {
    pub local_position: Vec3,
    pub direction: Vec3,
    pub rest_length: f32,
    pub min_length: f32,
    pub max_length: f32,
    pub hz: f32,
    pub damping_ratio: f32,
}

/// Four wheel rays and the last length of each ray.
///
/// A hit stores the distance along the ray. A miss stores `max_length`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WheeledGround {
    pub wheels: [Wheel; 4],
    pub lengths: [f32; 4],
}

/// Steady lift, drag, and thrust through the centre of mass.
///
/// `cl` is a stored constant. Angle of attack is not read. Torque is zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aircraft {
    pub wing_area: f32,
    pub cl: f32,
    pub cd0: f32,
    pub induced: f32,
    pub thrust_max: f32,
    pub throttle: f32,
    pub air_density: f32,
}

/// One space thruster. Force is `throttle * max_force` along the chassis-frame direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thruster {
    pub local_position: Vec3,
    pub local_direction: Vec3,
    pub max_force: f32,
    pub throttle: f32,
}

/// A short thruster list. `count` is how many entries are live.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpaceCraft {
    pub thrusters: [Thruster; 8],
    pub count: u8,
}

/// The G1 fixture: a 1 200 kg chassis over a static plane.
///
/// `sleep_time` of 10 keeps the chassis awake for the 3 s rest check.
/// The default 0.5 lets it sleep.
#[must_use]
pub fn chassis_rest_world(sleep_time: f32) -> World {
    let mut world = World::new(GRAVITY);
    world.sleep_time = sleep_time;
    world.insert(Body::plane(Vec3::Y, 0.0));
    let mut chassis = Body::cuboid(
        Vec3::new(0.0, 0.45, 0.0),
        1_200.0,
        Vec3::new(1.2, 0.15, 0.7),
    );
    chassis.friction = 0.6;
    chassis.restitution = 0.0;
    chassis.motion = MotionQuality::Discrete;
    let index = world.insert(chassis);
    world.vehicles.push(g1_vehicle(index));
    world
}

/// The S1 fixture: one offset thruster on a 1 000 kg cube, gravity zero.
///
/// Local position `(0, 2, 0)`, direction local +X, `max_force` 1 000, throttle 1.
/// `sleep_time` of 10 keeps the cube awake for the 60-step check.
#[must_use]
pub fn thruster_world() -> World {
    let mut world = World::new(Vec3::ZERO);
    world.sleep_time = 10.0;
    let mut body = Body::cuboid(Vec3::ZERO, 1_000.0, Vec3::new(1.0, 1.0, 1.0));
    body.friction = 0.0;
    body.restitution = 0.0;
    body.motion = MotionQuality::Discrete;
    let index = world.insert(body);
    let mut thrusters = [Thruster {
        local_position: Vec3::ZERO,
        local_direction: Vec3::X,
        max_force: 0.0,
        throttle: 0.0,
    }; 8];
    if let Some(slot) = thrusters.first_mut() {
        *slot = Thruster {
            local_position: Vec3::new(0.0, 2.0, 0.0),
            local_direction: Vec3::X,
            max_force: 1_000.0,
            throttle: 1.0,
        };
    }
    world.vehicles.push(Vehicle {
        chassis: index,
        ground: None,
        aircraft: None,
        space: Some(SpaceCraft {
            thrusters,
            count: 1,
        }),
    });
    world
}

fn g1_vehicle(chassis: usize) -> Vehicle {
    Vehicle {
        chassis,
        ground: Some(GroundContact::Wheeled(WheeledGround {
            wheels: g1_wheels(0.6),
            lengths: [0.4, 0.4, 0.4, 0.4],
        })),
        aircraft: None,
        space: None,
    }
}

/// The A1 fixture: a trimmed 1 000 kg aircraft. No ground and no plane.
///
/// `cl` and `thrust_max` come from the f32 trim association in ADR 0040, so
/// lift equals weight and thrust equals drag at 50 m/s.
#[must_use]
pub fn aero_trim_world() -> World {
    let mut world = World::new(GRAVITY);
    let mut body = Body::cuboid(
        Vec3::new(0.0, 100.0, 0.0),
        1_000.0,
        Vec3::new(1.0, 0.2, 3.0),
    );
    body.velocity = Vec3::new(50.0, 0.0, 0.0);
    body.motion = MotionQuality::Discrete;
    let index = world.insert(body);
    world.vehicles.push(Vehicle {
        chassis: index,
        ground: None,
        aircraft: Some(trim_aircraft(1_000.0, world.gravity)),
        space: None,
    });
    world
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

/// The T1 miss fixture: the A1 aircraft plus four wheels of `max_length` 0.5.
///
/// There is no plane. Every ray misses. Pose and velocity match [`aero_trim_world`].
#[must_use]
pub fn gear_miss_world() -> World {
    let mut world = aero_trim_world();
    if let Some(vehicle) = world.vehicles.first_mut() {
        vehicle.ground = Some(GroundContact::Wheeled(WheeledGround {
            wheels: g1_wheels(0.5),
            lengths: [0.4, 0.4, 0.4, 0.4],
        }));
    }
    world
}

/// The T1 shared-weight fixture. Mass is 1 000 kg, so each spring carries less
/// than G1. `cl` stays the A1 trim constant. `thrust_max` is the drag at 20 m/s.
#[must_use]
pub fn gear_plane_world() -> World {
    let mut world = World::new(GRAVITY);
    world.sleep_time = 10.0;
    world.insert(Body::plane(Vec3::Y, 0.0));
    let mut body = Body::cuboid(
        Vec3::new(0.0, 0.45, 0.0),
        1_000.0,
        Vec3::new(1.2, 0.15, 0.7),
    );
    body.friction = 0.6;
    body.restitution = 0.0;
    body.motion = MotionQuality::Discrete;
    body.velocity = Vec3::new(20.0, 0.0, 0.0);
    let index = world.insert(body);
    let mut aircraft = trim_aircraft(1_000.0, world.gravity);
    let speed = 20.0_f32;
    let q = 0.5 * aircraft.air_density * (speed * speed);
    let cd = aircraft.cd0 + aircraft.induced * (aircraft.cl * aircraft.cl);
    aircraft.thrust_max = (q * aircraft.wing_area) * cd;
    world.vehicles.push(Vehicle {
        chassis: index,
        ground: Some(GroundContact::Wheeled(WheeledGround {
            wheels: g1_wheels(0.6),
            lengths: [0.4, 0.4, 0.4, 0.4],
        })),
        aircraft: Some(aircraft),
        space: None,
    });
    world
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
    let thrust_max = (q * wing_area) * cd;
    Aircraft {
        wing_area,
        cl,
        cd0,
        induced,
        thrust_max,
        throttle: 1.0,
        air_density,
    }
}

/// Wake a sleeping chassis, and the rest of its island, when aircraft thrust
/// would write a finite non-zero force.
///
/// Runs before gravity and before that force, so the ignition step integrates
/// both. A closed, zero, negative, or non-finite throttle does not wake. A
/// non-finite coefficient, including `induced`, does not wake. This does not
/// set `Body.motor`.
pub(crate) fn wake_sleeping_aircraft(world: &mut World) {
    let count = world.vehicles.len();
    for index in 0..count {
        let Some(vehicle) = world.vehicles.get(index).copied() else {
            continue;
        };
        let Some(aircraft) = vehicle.aircraft else {
            continue;
        };
        let armed = world.bodies.get(vehicle.chassis).is_some_and(|body| {
            body.presence == Presence::Live
                && body.inverse_mass != 0.0
                && body.sleeping
                && aircraft_can_thrust(&aircraft, body)
        });
        if armed {
            world.wake(vehicle.chassis);
        }
    }
}

/// Wake a sleeping chassis, and the rest of its island, when a live thruster
/// would write a finite non-zero force.
///
/// Runs before gravity and before the wrench, so the ignition step integrates
/// both. A closed, zero, or non-finite thruster does not wake. This does not
/// set `Body.motor`.
pub(crate) fn wake_sleeping_thrusters(world: &mut World) {
    let count = world.vehicles.len();
    for index in 0..count {
        let Some(vehicle) = world.vehicles.get(index).copied() else {
            continue;
        };
        let Some(craft) = vehicle.space else {
            continue;
        };
        let armed = world.bodies.get(vehicle.chassis).is_some_and(|body| {
            body.presence == Presence::Live
                && body.inverse_mass != 0.0
                && body.sleeping
                && space_has_live_thrust(body, &craft)
        });
        if armed {
            world.wake(vehicle.chassis);
        }
    }
}

/// Domain forces, after gravity and before the chassis is integrated.
///
/// Order is aircraft, then space, then ground, including aircraft and ground
/// together on a landing chassis. [`integrate_wrench`] is the velocity update
/// the thruster uses. Aircraft force acts at the centre of mass. The aircraft
/// sample runs before the thruster and before the ground push.
pub(crate) fn apply(world: &mut World, dt: f32) {
    world.step_stats.vehicle_count = u32::try_from(world.vehicles.len()).unwrap_or(u32::MAX);
    let mut rays = 0_u32;
    let count = world.vehicles.len();
    for index in 0..count {
        apply_aircraft(world, index, dt);
        apply_space(world, index, dt);
        rays = rays.saturating_add(apply_ground(world, index, dt));
    }
    world.step_stats.wheel_rays = rays;
}

/// True when this chassis must stay awake the way a motor does.
///
/// A1 holds for a finite positive throttle whose thrust is a finite non-zero
/// force. The same check is what [`wake_sleeping_aircraft`] uses. A thruster
/// hold is [`actuator_holds_awake`]. This does not set `Body.motor`.
pub(crate) fn holds_awake(vehicles: &[Vehicle], body: &Body, chassis: usize) -> bool {
    vehicles.iter().any(|vehicle| {
        vehicle.chassis == chassis
            && vehicle
                .aircraft
                .is_some_and(|aircraft| aircraft_can_thrust(&aircraft, body))
    })
}

/// Finite aircraft fields and a positive throttle with a finite non-zero thrust.
///
/// Negative throttle is reverse thrust. The force still applies when the
/// chassis is already awake. It does not wake and it does not hold.
fn aircraft_can_thrust(aircraft: &Aircraft, body: &Body) -> bool {
    if !finite_aircraft(aircraft) || !(aircraft.throttle > 0.0) {
        return false;
    }
    let thrust = aircraft.throttle * aircraft.thrust_max;
    if !thrust.is_finite() || thrust == 0.0 {
        return false;
    }
    let force = body.orientation.rotate(Vec3::X) * thrust;
    finite_vec(force) && force.length_squared() > 0.0
}

fn apply_aircraft(world: &mut World, index: usize, dt: f32) {
    let Some(vehicle) = world.vehicles.get(index).copied() else {
        return;
    };
    let Some(aircraft) = vehicle.aircraft else {
        return;
    };
    let Some(body) = world.bodies.get(vehicle.chassis).copied() else {
        return;
    };
    if body.presence != Presence::Live || body.inverse_mass == 0.0 || body.sleeping {
        return;
    }
    let Some(force) = aircraft_force(&aircraft, &body, world.gravity, dt) else {
        return;
    };
    if let Some(stored) = world.bodies.get_mut(vehicle.chassis) {
        stored.velocity += (force * stored.inverse_mass) * dt;
    }
}

/// Lift, drag, and thrust. `None` means the input cannot produce a finite force.
fn aircraft_force(aircraft: &Aircraft, body: &Body, gravity: Vec3, dt: f32) -> Option<Vec3> {
    // `apply_forces` has already added `gravity * dt`. Dynamic pressure uses the
    // velocity from before that increment, and from before the thruster and the
    // ground push: `apply` runs aircraft, then space, then ground. Drag is zero
    // when that speed is zero.
    let air = body.velocity - gravity * dt;
    if !finite_aircraft(aircraft) || !finite_vec(air) {
        return None;
    }
    let speed = air.length();
    let q = 0.5 * aircraft.air_density * (speed * speed);
    let lift = aircraft.cl * q * aircraft.wing_area;
    let cd = aircraft.cd0 + aircraft.induced * (aircraft.cl * aircraft.cl);
    let drag_mag = cd * q * aircraft.wing_area;
    let thrust = aircraft.throttle * aircraft.thrust_max;
    let mut force = Vec3::Y * lift + body.orientation.rotate(Vec3::X) * thrust;
    if speed > 0.0 {
        force += air * (-drag_mag / speed);
    }
    finite_vec(force).then_some(force)
}

fn finite_aircraft(aircraft: &Aircraft) -> bool {
    aircraft.wing_area.is_finite()
        && aircraft.cl.is_finite()
        && aircraft.cd0.is_finite()
        && aircraft.induced.is_finite()
        && aircraft.thrust_max.is_finite()
        && aircraft.throttle.is_finite()
        && aircraft.air_density.is_finite()
}

/// A thruster with `throttle > 0` holds the chassis awake the way `Body.motor` does.
///
/// This does not wake a sleeper. `wake_sleeping_thrusters` does that, and only
/// for a finite non-zero force. Aircraft wake and hold stay on [`holds_awake`].
/// The function does not set `Body.motor`.
pub(crate) fn actuator_holds_awake(vehicles: &[Vehicle], chassis: usize) -> bool {
    vehicles
        .iter()
        .any(|vehicle| vehicle.chassis == chassis && space_throttle_open(vehicle.space))
}

fn space_throttle_open(space: Option<SpaceCraft>) -> bool {
    let Some(craft) = space else {
        return false;
    };
    let live = usize::from(craft.count).min(craft.thrusters.len());
    craft
        .thrusters
        .iter()
        .take(live)
        .any(|thruster| thruster.throttle.is_finite() && thruster.throttle > 0.0)
}

fn apply_space(world: &mut World, index: usize, dt: f32) {
    let Some(vehicle) = world.vehicles.get(index).copied() else {
        return;
    };
    let Some(craft) = vehicle.space else {
        return;
    };
    let Some(mut body) = world.bodies.get(vehicle.chassis).copied() else {
        return;
    };
    if body.presence != Presence::Live || body.inverse_mass == 0.0 || body.sleeping {
        return;
    }
    let live = usize::from(craft.count).min(craft.thrusters.len());
    for thruster in craft.thrusters.iter().take(live) {
        if let Some((force, arm)) = thruster_wrench(&body, thruster) {
            integrate_wrench(&mut body, force, arm, dt);
        }
    }
    if let Some(stored) = world.bodies.get_mut(vehicle.chassis) {
        stored.velocity = body.velocity;
        stored.angular_velocity = body.angular_velocity;
    }
}

/// An open thruster that would write a finite non-zero force.
fn space_has_live_thrust(body: &Body, craft: &SpaceCraft) -> bool {
    let live = usize::from(craft.count).min(craft.thrusters.len());
    craft
        .thrusters
        .iter()
        .take(live)
        .any(|thruster| thruster.throttle > 0.0 && thruster_wrench(body, thruster).is_some())
}

/// World force and the chassis-frame arm, or nothing when the inputs are not finite.
fn thruster_wrench(body: &Body, thruster: &Thruster) -> Option<(Vec3, Vec3)> {
    if !thruster.throttle.is_finite() || !thruster.max_force.is_finite() {
        return None;
    }
    let magnitude = thruster.throttle * thruster.max_force;
    if !magnitude.is_finite() || magnitude == 0.0 {
        return None;
    }
    if !finite_vec(thruster.local_direction) || !finite_vec(thruster.local_position) {
        return None;
    }
    let direction = body
        .orientation
        .rotate(thruster.local_direction)
        .normalize();
    if !finite_vec(direction) || direction.length_squared() == 0.0 {
        return None;
    }
    let force = direction * magnitude;
    let arm = body.orientation.rotate(thruster.local_position);
    if !finite_vec(force) || !finite_vec(arm) {
        return None;
    }
    Some((force, arm))
}

/// Semi-implicit wrench on the chassis. `integrate_body` runs the gyroscopic step after this.
///
/// A non-finite result is left unwritten. Ground keeps its own copy of the same
/// update so a later aircraft stage can call this function without editing the ray path.
fn integrate_wrench(body: &mut Body, force: Vec3, arm: Vec3, dt: f32) {
    let velocity = body.velocity + force * (body.inverse_mass * dt);
    let angular = body.angular_velocity + world_inverse_inertia(body, arm.cross(force)) * dt;
    if !finite_vec(velocity) || !finite_vec(angular) {
        return;
    }
    body.velocity = velocity;
    body.angular_velocity = angular;
}

fn finite_vec(value: Vec3) -> bool {
    value.x.is_finite() && value.y.is_finite() && value.z.is_finite()
}

fn apply_ground(world: &mut World, index: usize, dt: f32) -> u32 {
    let Some(vehicle) = world.vehicles.get(index).copied() else {
        return 0;
    };
    let Some(GroundContact::Wheeled(mut ground)) = vehicle.ground else {
        return 0;
    };
    let Some(chassis) = world.bodies.get(vehicle.chassis).copied() else {
        return 0;
    };
    if chassis.presence != Presence::Live || chassis.inverse_mass == 0.0 || chassis.sleeping {
        return 0;
    }
    let mut pushes = [None; 4];
    for (wheel_index, wheel) in ground.wheels.iter().enumerate() {
        let (length, push) = sample_wheel(world, vehicle.chassis, &chassis, wheel);
        if let Some(stored) = ground.lengths.get_mut(wheel_index) {
            *stored = length;
        }
        if let Some(slot) = pushes.get_mut(wheel_index) {
            *slot = push;
        }
    }
    let mut body = chassis;
    for push in pushes {
        let Some((force, arm)) = push else {
            continue;
        };
        body.velocity += force * (body.inverse_mass * dt);
        let torque = arm.cross(force);
        body.angular_velocity += world_inverse_inertia(&body, torque) * dt;
    }
    if let Some(stored) = world.bodies.get_mut(vehicle.chassis) {
        stored.velocity = body.velocity;
        stored.angular_velocity = body.angular_velocity;
    }
    if let Some(stored) = world.vehicles.get_mut(index) {
        if let Some(GroundContact::Wheeled(slot)) = &mut stored.ground {
            slot.lengths = ground.lengths;
        }
    }
    u32::try_from(ground.wheels.len()).unwrap_or(0)
}

fn sample_wheel(
    world: &World,
    chassis: usize,
    body: &Body,
    wheel: &Wheel,
) -> (f32, Option<(Vec3, Vec3)>) {
    let Some(direction) = aim(body, wheel.direction) else {
        return (wheel.max_length, None);
    };
    let origin = body.position + body.orientation.rotate(wheel.local_position);
    let Some(length) = wheel_ray(
        world,
        chassis,
        origin,
        direction,
        wheel.min_length,
        wheel.max_length,
    ) else {
        return (wheel.max_length, None);
    };
    let arm = body.orientation.rotate(wheel.local_position) + direction * length;
    let force = push(body, wheel, direction, length, arm);
    (length, force.map(|force| (force, arm)))
}

fn aim(body: &Body, local_direction: Vec3) -> Option<Vec3> {
    let direction = body.orientation.rotate(local_direction).normalize();
    (direction.length_squared() > 0.0).then_some(direction)
}

/// Explicit spring-damper. It pushes only, and it is not [`Body::spring`](crate::Body::spring).
fn push(body: &Body, wheel: &Wheel, direction: Vec3, length: f32, arm: Vec3) -> Option<Vec3> {
    if !(length >= wheel.min_length && length <= wheel.max_length) {
        return None;
    }
    let compression = wheel.rest_length - length;
    if compression <= 0.0 {
        return None;
    }
    let count = u8::try_from(4).unwrap_or(1);
    let suspended = body.mass() / f32::from(count);
    let omega = 2.0 * std::f32::consts::PI * wheel.hz;
    let stiffness = suspended * omega * omega;
    let damping = 2.0 * wheel.damping_ratio * suspended * omega;
    let at_point = body.velocity + body.angular_velocity.cross(arm);
    let compression_rate = -direction.dot(at_point);
    let magnitude = stiffness * compression - damping * compression_rate;
    if !magnitude.is_finite() || magnitude <= 0.0 {
        return None;
    }
    Some(-direction * magnitude)
}

fn wheel_ray(
    world: &World,
    chassis: usize,
    origin: Vec3,
    direction: Vec3,
    min_length: f32,
    max_length: f32,
) -> Option<f32> {
    let mut best: Option<f32> = None;
    for (index, body) in world.bodies.iter().enumerate() {
        if index == chassis || body.presence == Presence::Culled {
            continue;
        }
        let hit = match body.shape {
            Shape::Plane { normal, offset } => {
                ray_plane(origin, direction, normal, offset, min_length, max_length)
            }
            Shape::Box { half_extents } => ray_box(
                body,
                half_extents,
                origin,
                direction,
                min_length,
                max_length,
            ),
            Shape::Sphere { .. } | Shape::Capsule { .. } | Shape::Mesh(_) => None,
        };
        if let Some(distance) = hit {
            best = Some(best.map_or(
                distance,
                |so_far| {
                    if distance < so_far {
                        distance
                    } else {
                        so_far
                    }
                },
            ));
        }
    }
    best
}

fn ray_plane(
    origin: Vec3,
    direction: Vec3,
    normal: Vec3,
    offset: f32,
    min_length: f32,
    max_length: f32,
) -> Option<f32> {
    let denom = normal.dot(direction);
    if denom.abs() <= 1.0e-8 {
        return None;
    }
    let distance = (offset - normal.dot(origin)) / denom;
    (distance >= min_length && distance <= max_length).then_some(distance)
}

fn ray_box(
    body: &Body,
    half: Vec3,
    origin: Vec3,
    direction: Vec3,
    min_length: f32,
    max_length: f32,
) -> Option<f32> {
    let frame = conjugate(body.orientation);
    let local_origin = frame.rotate(origin - body.position);
    let local_dir = frame.rotate(direction);
    slab(local_origin, local_dir, half, min_length, max_length)
}

fn slab(
    origin: Vec3,
    direction: Vec3,
    half: Vec3,
    min_length: f32,
    max_length: f32,
) -> Option<f32> {
    let mut enter = min_length;
    let mut exit = max_length;
    let axes = [
        (origin.x, direction.x, half.x),
        (origin.y, direction.y, half.y),
        (origin.z, direction.z, half.z),
    ];
    for (at, along, extent) in axes {
        if !clip_axis(at, along, extent, &mut enter, &mut exit) {
            return None;
        }
    }
    if enter >= min_length && enter <= max_length {
        Some(enter)
    } else if exit >= min_length && enter < min_length {
        Some(min_length)
    } else {
        None
    }
}

fn clip_axis(origin: f32, direction: f32, half: f32, enter: &mut f32, exit: &mut f32) -> bool {
    if direction.abs() <= 1.0e-8 {
        return (-half..=half).contains(&origin);
    }
    let inverse = 1.0 / direction;
    let mut near = (-half - origin) * inverse;
    let mut far = (half - origin) * inverse;
    if near > far {
        std::mem::swap(&mut near, &mut far);
    }
    *enter = enter.max(near);
    *exit = exit.min(far);
    *enter <= *exit
}

fn conjugate(orientation: Quat) -> Quat {
    Quat::new(
        -orientation.x,
        -orientation.y,
        -orientation.z,
        orientation.w,
    )
}
