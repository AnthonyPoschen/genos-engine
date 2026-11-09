//! Domain forces on one chassis body.
//!
//! The record holds a ground payload, an aircraft payload, and a space payload.
//! This stage applies the wheeled ground spring-damper and, when a wheel has a
//! positive radius, the plan's three-point slip curve. Aircraft lift and
//! thruster force are later stages and are not applied here. Sea is not a field.

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
///
/// Slip force is off when `radius` is not positive. G1 leaves it at 0. The
/// game writes `omega`. This stage does not integrate it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wheel {
    pub local_position: Vec3,
    pub direction: Vec3,
    pub rest_length: f32,
    pub min_length: f32,
    pub max_length: f32,
    pub hz: f32,
    pub damping_ratio: f32,
    pub radius: f32,
    pub omega: f32,
    /// Peak of the three-point curve. The plan default is 1.
    pub mu_peak: f32,
    /// Absolute slip where the curve reaches `mu_peak`. The plan default is 0.1.
    pub slip_peak: f32,
    /// Curve value at absolute slip 1, and beyond. The plan default is 0.7.
    pub mu_slide: f32,
    /// Metres per second. Longitudinal slip divides by
    /// `max(planar speed, slip_denom)`. The plan names this term and does not
    /// give a number. The default here is 1.
    pub slip_denom: f32,
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

/// One space thruster. Stored for a later stage. This stage does not apply it.
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

/// The P28 fixture. The G1 chassis at the settled height, 10 m/s on +X.
///
/// Each wheel has radius 0.3 m, omega 0, `mu_peak` 1, `slip_peak` 0.1,
/// `mu_slide` 0.7, and `slip_denom` 1. `sleep_time` is 10 s.
#[must_use]
pub fn locked_wheel_world() -> World {
    let mut world = World::new(GRAVITY);
    world.sleep_time = 10.0;
    world.insert(Body::plane(Vec3::Y, 0.0));
    let mut chassis = Body::cuboid(
        Vec3::new(0.0, 0.384_469, 0.0),
        1_200.0,
        Vec3::new(1.2, 0.15, 0.7),
    );
    chassis.friction = 0.6;
    chassis.restitution = 0.0;
    chassis.motion = MotionQuality::Discrete;
    chassis.velocity = Vec3::new(10.0, 0.0, 0.0);
    let index = world.insert(chassis);
    let mut vehicle = g1_vehicle(index);
    if let Some(GroundContact::Wheeled(ref mut ground)) = vehicle.ground {
        for wheel in &mut ground.wheels {
            wheel.radius = 0.3;
        }
    }
    world.vehicles.push(vehicle);
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
        radius: 0.0,
        omega: 0.0,
        mu_peak: 1.0,
        slip_peak: 0.1,
        mu_slide: 0.7,
        slip_denom: 1.0,
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

/// Ground forces, after gravity and before the chassis is integrated.
///
/// Wheel index order is the only order. Each wheel applies its spring, then
/// its slip force. Both use the velocity from the start of this pass.
pub(crate) fn apply(world: &mut World, dt: f32) {
    world.step_stats.vehicle_count = u32::try_from(world.vehicles.len()).unwrap_or(u32::MAX);
    let mut rays = 0_u32;
    let mut contacts = 0_u32;
    let count = world.vehicles.len();
    for index in 0..count {
        let (cast, slips) = apply_ground(world, index, dt);
        rays = rays.saturating_add(cast);
        contacts = contacts.saturating_add(slips);
    }
    world.step_stats.wheel_rays = rays;
    world.step_stats.tyre_contacts = contacts;
}

fn apply_ground(world: &mut World, index: usize, dt: f32) -> (u32, u32) {
    let Some(vehicle) = world.vehicles.get(index).copied() else {
        return (0, 0);
    };
    let Some(GroundContact::Wheeled(mut ground)) = vehicle.ground else {
        return (0, 0);
    };
    let Some(chassis) = world.bodies.get(vehicle.chassis).copied() else {
        return (0, 0);
    };
    if chassis.presence != Presence::Live || chassis.inverse_mass == 0.0 || chassis.sleeping {
        return (0, 0);
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
    let mut slips = 0_u32;
    for push in pushes {
        let Some(push) = push else {
            continue;
        };
        accelerate(&mut body, push.arm, push.suspension, dt);
        if push.slip.length_squared() > 0.0 {
            accelerate(&mut body, push.arm, push.slip, dt);
            slips = slips.saturating_add(1);
        }
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
    (u32::try_from(ground.wheels.len()).unwrap_or(0), slips)
}

#[derive(Clone, Copy)]
struct ContactPush {
    arm: Vec3,
    suspension: Vec3,
    slip: Vec3,
}

fn accelerate(body: &mut Body, arm: Vec3, force: Vec3, dt: f32) {
    body.velocity += force * (body.inverse_mass * dt);
    let torque = arm.cross(force);
    body.angular_velocity += world_inverse_inertia(body, torque) * dt;
}

fn sample_wheel(
    world: &World,
    chassis: usize,
    body: &Body,
    wheel: &Wheel,
) -> (f32, Option<ContactPush>) {
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
    let Some(suspension) = push(body, wheel, direction, length, arm) else {
        return (length, None);
    };
    let slip = slip_force(body, wheel, direction, arm, suspension.length());
    (
        length,
        Some(ContactPush {
            arm,
            suspension,
            slip,
        }),
    )
}

/// In-plane force from the three-point slip curve, capped by the spring load.
///
/// Longitudinal slip is `(omega * radius - forward speed) / max(planar speed, slip_denom)`.
/// Lateral slip is the angle from the direction of travel, `atan2(lateral speed, |forward speed|)`.
/// `atan2(lateral, forward)` is π in straight reverse and would apply `mu_slide`
/// sideways at zero sideslip. The sign follows the lateral speed, so the force
/// opposes sideslip in both directions. The call does not order wheels.
/// `|force|` is capped by the spring load, so the slip impulse over `dt` does
/// not exceed the spring impulse.
fn slip_force(body: &Body, wheel: &Wheel, direction: Vec3, arm: Vec3, load: f32) -> Vec3 {
    if wheel.radius <= 0.0 || load <= 0.0 || !wheel.radius.is_finite() || !wheel.omega.is_finite() {
        return Vec3::ZERO;
    }
    let Some(heading) = plane_heading(body, direction) else {
        return Vec3::ZERO;
    };
    let lateral = direction.cross(heading).normalize();
    if lateral.length_squared() == 0.0 {
        return Vec3::ZERO;
    }
    let at_point = body.velocity + body.angular_velocity.cross(arm);
    let planar = at_point - direction * at_point.dot(direction);
    let forward_speed = planar.dot(heading);
    let lateral_speed = planar.dot(lateral);
    let denom = planar.length().max(wheel.slip_denom);
    let slip_long = ratio(wheel.omega * wheel.radius - forward_speed, denom);
    let slip_lat = slip_angle(lateral_speed, forward_speed);
    let force = heading * (signed_curve(slip_long, wheel) * load)
        - lateral * (signed_curve(slip_lat, wheel) * load);
    let capped = cap_to_load(force, load);
    if capped.x.is_finite() && capped.y.is_finite() && capped.z.is_finite() {
        capped
    } else {
        Vec3::ZERO
    }
}

/// Angle of the contact velocity from the wheel's travel direction.
///
/// Zero when the chassis rolls straight ahead or straight back. Positive when
/// the lateral speed is positive, so a single restoring sign works both ways.
fn slip_angle(lateral_speed: f32, forward_speed: f32) -> f32 {
    if lateral_speed.is_finite() && forward_speed.is_finite() {
        lateral_speed.atan2(forward_speed.abs())
    } else {
        0.0
    }
}

fn plane_heading(body: &Body, direction: Vec3) -> Option<Vec3> {
    let aimed = body.orientation.rotate(Vec3::X);
    let heading = (aimed - direction * aimed.dot(direction)).normalize();
    (heading.length_squared() > 0.0).then_some(heading)
}

fn ratio(numerator: f32, denom: f32) -> f32 {
    if denom > 1.0e-8 && numerator.is_finite() {
        numerator / denom
    } else {
        0.0
    }
}

fn signed_curve(slip: f32, wheel: &Wheel) -> f32 {
    let magnitude = slip_curve(slip.abs(), wheel);
    if slip > 0.0 {
        magnitude
    } else if slip < 0.0 {
        -magnitude
    } else {
        0.0
    }
}

/// Rises from 0 to `mu_peak` at `slip_peak`, then falls to `mu_slide` at slip 1.
fn slip_curve(abs_slip: f32, wheel: &Wheel) -> f32 {
    if !(abs_slip > 0.0) || !abs_slip.is_finite() {
        return 0.0;
    }
    let peak = wheel.slip_peak;
    if !(peak > 0.0) || peak >= 1.0 {
        return if abs_slip >= 1.0 {
            wheel.mu_slide
        } else {
            wheel.mu_peak
        };
    }
    if abs_slip < peak {
        return wheel.mu_peak * (abs_slip / peak);
    }
    if abs_slip >= 1.0 {
        return wheel.mu_slide;
    }
    let t = (abs_slip - peak) / (1.0 - peak);
    wheel.mu_peak + (wheel.mu_slide - wheel.mu_peak) * t
}

fn cap_to_load(force: Vec3, load: f32) -> Vec3 {
    let cap = load.abs();
    let mag_sq = force.length_squared();
    if mag_sq <= cap * cap || mag_sq <= f32::MIN_POSITIVE {
        return force;
    }
    force * (cap / mag_sq.sqrt())
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

#[cfg(test)]
mod slip_angle_tests {
    use super::{plane_heading, slip_force, Wheel};
    use crate::{Body, MotionQuality};
    use genos_math::Vec3;

    fn wheel(omega: f32) -> Wheel {
        Wheel {
            local_position: Vec3::new(1.0, 0.0, 0.5),
            direction: Vec3::new(0.0, -1.0, 0.0),
            rest_length: 0.4,
            min_length: 0.1,
            max_length: 0.6,
            hz: 4.0,
            damping_ratio: 0.7,
            radius: 0.3,
            omega,
            mu_peak: 1.0,
            slip_peak: 0.1,
            mu_slide: 0.7,
            slip_denom: 1.0,
        }
    }

    fn rolling(velocity: Vec3) -> Body {
        let mut body = Body::cuboid(Vec3::ZERO, 1_200.0, Vec3::new(1.2, 0.15, 0.7));
        body.velocity = velocity;
        body.motion = MotionQuality::Discrete;
        body
    }

    fn lateral_of(body: &Body) -> Vec3 {
        let direction = Vec3::new(0.0, -1.0, 0.0);
        let Some(heading) = plane_heading(body, direction) else {
            return Vec3::ZERO;
        };
        direction.cross(heading).normalize()
    }

    #[test]
    fn straight_reverse_matched_roll_has_no_lateral_force() {
        let body = rolling(Vec3::new(-10.0, 0.0, 0.0));
        let force = slip_force(
            &body,
            &wheel(-10.0 / 0.3),
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::ZERO,
            3_000.0,
        );
        let lateral = force.dot(lateral_of(&body));
        assert!(
            lateral.abs() < 1.0e-3,
            "straight reverse lateral force {lateral}"
        );
        assert!(
            force.length() < 1.0e-2,
            "matched reverse roll force {force:?}"
        );
    }

    #[test]
    fn reverse_and_forward_sideslip_forces_oppose_the_lateral_speed() {
        let direction = Vec3::new(0.0, -1.0, 0.0);
        let reverse = rolling(Vec3::new(-10.0, 0.0, 2.0));
        let reverse_force = slip_force(
            &reverse,
            &wheel(-10.0 / 0.3),
            direction,
            Vec3::ZERO,
            3_000.0,
        );
        let reverse_lateral = reverse_force.dot(lateral_of(&reverse));
        assert!(
            reverse_lateral < -100.0,
            "reverse sideslip force {reverse_lateral}"
        );
        let forward = rolling(Vec3::new(10.0, 0.0, 2.0));
        let forward_force =
            slip_force(&forward, &wheel(10.0 / 0.3), direction, Vec3::ZERO, 3_000.0);
        let forward_lateral = forward_force.dot(lateral_of(&forward));
        assert!(
            forward_lateral < -100.0,
            "forward sideslip force {forward_lateral}"
        );
    }
}
