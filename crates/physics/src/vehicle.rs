//! Domain forces on one chassis body.
//!
//! The record holds a ground payload, an aircraft payload, and a space payload.
//! This stage applies the wheeled ground spring-damper and the space thrusters.
//! Aircraft lift is a later stage and is not applied here. Sea is not a field.
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

/// Steady wing. Stored for a later stage. This stage does not apply it.
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
    let wheel = |local_position: Vec3| Wheel {
        local_position,
        direction: Vec3::new(0.0, -1.0, 0.0),
        rest_length: 0.4,
        min_length: 0.1,
        max_length: 0.6,
        hz: 4.0,
        damping_ratio: 0.7,
    };
    let wheels = [
        wheel(Vec3::new(1.0, 0.0, 0.5)),
        wheel(Vec3::new(1.0, 0.0, -0.5)),
        wheel(Vec3::new(-1.0, 0.0, 0.5)),
        wheel(Vec3::new(-1.0, 0.0, -0.5)),
    ];
    Vehicle {
        chassis,
        ground: Some(GroundContact::Wheeled(WheeledGround {
            wheels,
            lengths: [0.4, 0.4, 0.4, 0.4],
        })),
        aircraft: None,
        space: None,
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
/// Order is aircraft, then space, then ground. Aircraft is not applied in
/// this stage. [`integrate_wrench`] is the one velocity update those stages share.
pub(crate) fn apply(world: &mut World, dt: f32) {
    world.step_stats.vehicle_count = u32::try_from(world.vehicles.len()).unwrap_or(u32::MAX);
    let mut rays = 0_u32;
    let count = world.vehicles.len();
    for index in 0..count {
        apply_space(world, index, dt);
        rays = rays.saturating_add(apply_ground(world, index, dt));
    }
    world.step_stats.wheel_rays = rays;
}

/// A thruster with `throttle > 0` holds the chassis awake the way `Body.motor` does.
///
/// This does not wake a sleeper. `wake_sleeping_thrusters` does that, and only
/// for a finite non-zero force. Aircraft throttle joins this check in its
/// stage. The function does not set `Body.motor`.
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
