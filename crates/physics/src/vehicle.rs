//! Domain forces on one chassis body.
//!
//! The record holds a ground payload, an aircraft payload, and a space payload.
//! This stage applies the wheeled ground spring-damper, the three-point slip
//! curve when a wheel has a positive radius, a gearbox and an open differential
//! when the ground payload has a drive block, and the space thrusters. Aircraft
//! lift is a later stage and is not applied here. Sea is not a field. A thruster
//! writes force and torque through [`integrate_wrench`]. The existing gyroscopic
//! step then consumes that angular velocity. There is no second integrator.

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
/// game writes `omega` unless the ground payload has a [`Drive`]. With a drive
/// block, this stage integrates `omega` from the axle share and the tyre reaction.
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
    /// The open differential writes this wheel a share of the axle torque.
    pub driven: bool,
    /// Wheel inertia, kg·m². Not positive means the step does not integrate `omega`.
    pub inertia: f32,
}

/// Four wheel rays and the last length of each ray.
///
/// A hit stores the distance along the ray. A miss stores `max_length`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WheeledGround {
    pub wheels: [Wheel; 4],
    pub lengths: [f32; 4],
    /// Engine, gearbox, and open differential. `None` leaves `omega` as the game wrote it.
    pub drive: Option<Drive>,
}

/// Torque curve, gearbox, and clutch on one chassis.
///
/// `curve_rpm` and `curve_torque` are a table of `(rpm, N·m)`, in ascending
/// rpm, length `curve_count` (at most 8). `ratios` includes a negative reverse.
/// `gear` is an index the game sets. `clutch` is 0 or 1: only the bit pattern
/// of `1.0` engages. Engine speed is the mean driven-wheel `omega` through the
/// engaged ratio and `final_drive`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drive {
    pub throttle: f32,
    pub clutch: f32,
    pub gear: i8,
    pub final_drive: f32,
    pub ratios: [f32; 8],
    pub ratio_count: u8,
    pub curve_rpm: [f32; 8],
    pub curve_torque: [f32; 8],
    pub curve_count: u8,
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

/// The P29 fixture. The settled G1 chassis at rest, all four wheels driven.
///
/// Radius 0.3 m, inertia 0.5 kg·m². Throttle 1, clutch engaged, gear 0,
/// ratio 3, reverse ratio −3, final drive 1. The curve is 200 N·m from 0 rpm
/// to 5 000 rpm. `sleep_time` is 10 s.
#[must_use]
pub fn first_gear_world() -> World {
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
    let index = world.insert(chassis);
    let mut vehicle = g1_vehicle(index);
    if let Some(GroundContact::Wheeled(ref mut ground)) = vehicle.ground {
        for wheel in &mut ground.wheels {
            wheel.radius = 0.3;
            wheel.driven = true;
            wheel.inertia = 0.5;
        }
        ground.drive = Some(plan_drive());
    }
    world.vehicles.push(vehicle);
    world
}

fn plan_drive() -> Drive {
    let mut ratios = [0.0; 8];
    ratios[0] = 3.0;
    ratios[1] = -3.0;
    let mut curve_rpm = [0.0; 8];
    curve_rpm[1] = 5_000.0;
    let mut curve_torque = [0.0; 8];
    curve_torque[0] = 200.0;
    curve_torque[1] = 200.0;
    Drive {
        throttle: 1.0,
        clutch: 1.0,
        gear: 0,
        final_drive: 1.0,
        ratios,
        ratio_count: 2,
        curve_rpm,
        curve_torque,
        curve_count: 2,
    }
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
        radius: 0.0,
        omega: 0.0,
        mu_peak: 1.0,
        slip_peak: 0.1,
        mu_slide: 0.7,
        slip_denom: 1.0,
        driven: false,
        inertia: 0.0,
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
            drive: None,
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
/// this stage. [`integrate_wrench`] is the one velocity update the space stage
/// uses. Wheel index order is the only ground order. Each wheel applies its
/// spring, then its slip force. Both use the velocity from the start of this
/// pass. With a drive block, wheel spin is integrated before the slip force,
/// from that same velocity, and the force uses the updated spin.
pub(crate) fn apply(world: &mut World, dt: f32) {
    world.step_stats.vehicle_count = u32::try_from(world.vehicles.len()).unwrap_or(u32::MAX);
    let mut rays = 0_u32;
    let mut contacts = 0_u32;
    let mut driven = 0_u32;
    let count = world.vehicles.len();
    for index in 0..count {
        apply_space(world, index, dt);
        let (cast, slips, wrote) = apply_ground(world, index, dt);
        rays = rays.saturating_add(cast);
        contacts = contacts.saturating_add(slips);
        driven = driven.saturating_add(wrote);
    }
    world.step_stats.wheel_rays = rays;
    world.step_stats.tyre_contacts = contacts;
    world.step_stats.drive_wheels = driven;
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

fn apply_ground(world: &mut World, index: usize, dt: f32) -> (u32, u32, u32) {
    let Some(vehicle) = world.vehicles.get(index).copied() else {
        return (0, 0, 0);
    };
    let Some(GroundContact::Wheeled(mut ground)) = vehicle.ground else {
        return (0, 0, 0);
    };
    let Some(chassis) = world.bodies.get(vehicle.chassis).copied() else {
        return (0, 0, 0);
    };
    if chassis.presence != Presence::Live || chassis.inverse_mass == 0.0 || chassis.sleeping {
        return (0, 0, 0);
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
    for push in &pushes {
        let Some(push) = push else {
            continue;
        };
        accelerate(&mut body, push.arm, push.suspension, dt);
    }
    let wrote = spin_wheels(&mut ground, &chassis, &pushes, dt);
    // A driven wheel's slip force is the force at the updated spin. Using the
    // pre-update spin applies the tyre's full slope one step late and the
    // chassis speed oscillates through zero.
    let driven = ground.drive.is_some();
    for (index, push) in pushes.iter().enumerate() {
        let Some(push) = push else {
            continue;
        };
        let slip = if driven {
            ground.wheels.get(index).map_or(Vec3::ZERO, |wheel| {
                slip_force(
                    &chassis,
                    wheel,
                    push.direction,
                    push.arm,
                    push.suspension.length(),
                )
            })
        } else {
            push.slip
        };
        if slip.length_squared() > 0.0 {
            accelerate(&mut body, push.arm, slip, dt);
            slips = slips.saturating_add(1);
        }
    }
    if let Some(stored) = world.bodies.get_mut(vehicle.chassis) {
        stored.velocity = body.velocity;
        stored.angular_velocity = body.angular_velocity;
    }
    if let Some(stored) = world.vehicles.get_mut(index) {
        if let Some(GroundContact::Wheeled(slot)) = &mut stored.ground {
            *slot = ground;
        }
    }
    (
        u32::try_from(ground.wheels.len()).unwrap_or(0),
        slips,
        wrote,
    )
}

#[derive(Clone, Copy)]
struct ContactPush {
    arm: Vec3,
    suspension: Vec3,
    slip: Vec3,
    direction: Vec3,
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
            direction,
        }),
    )
}

/// Axle share plus the tyre reaction. No drive block leaves `omega` alone.
///
/// Where the drive force fits in the rising part of the slip curve, the
/// reaction slope is integrated implicitly. Explicit Euler at `1/60` steps
/// over that slope and flips the sign of `omega`.
fn spin_wheels(
    ground: &mut WheeledGround,
    chassis: &Body,
    pushes: &[Option<ContactPush>; 4],
    dt: f32,
) -> u32 {
    let Some(drive) = ground.drive else {
        return 0;
    };
    let share = drive_share(&drive, ground);
    let mut wrote = 0_u32;
    for (index, wheel) in ground.wheels.iter_mut().enumerate() {
        if !(wheel.inertia > 0.0) || !wheel.inertia.is_finite() {
            continue;
        }
        let drive_torque = if wheel.driven { share } else { 0.0 };
        let push = pushes.get(index).and_then(|item| item.as_ref());
        let reaction = push.map_or(0.0, |item| tyre_reaction(chassis, wheel, item));
        let next = advance_omega(
            wheel.omega,
            drive_torque,
            reaction,
            wheel,
            chassis,
            push,
            dt,
        );
        if next.is_finite() {
            wheel.omega = next;
        }
        if wheel.driven && drive_torque.abs() > 0.0 {
            wrote = wrote.saturating_add(1);
        }
    }
    wrote
}

/// Implicit step along the linear tyre slope when that slope can balance the drive.
/// Otherwise one explicit step. Neither path allocates.
fn advance_omega(
    omega: f32,
    drive_torque: f32,
    reaction: f32,
    wheel: &Wheel,
    chassis: &Body,
    push: Option<&ContactPush>,
    dt: f32,
) -> f32 {
    let explicit = explicit_spin(omega, drive_torque, reaction, wheel.inertia, dt);
    let Some(push) = push else {
        return explicit;
    };
    let Some(sample) = tyre_sample(chassis, wheel, push) else {
        return explicit;
    };
    let Some(slope) = grip_slope(wheel, sample.denom, sample.load) else {
        return explicit;
    };
    let target = drive_torque / wheel.radius;
    if !target.is_finite() || target.abs() > sample.max_longitudinal {
        return explicit;
    }
    let free = sample.forward_speed / wheel.radius;
    let step = dt / wheel.inertia;
    let denom = 1.0 - step * slope;
    if !(denom > 1.0e-6) || !free.is_finite() {
        return explicit;
    }
    let next = (omega + step * (drive_torque - slope * free)) / denom;
    if next.is_finite() {
        next
    } else {
        explicit
    }
}

fn explicit_spin(omega: f32, drive_torque: f32, reaction: f32, inertia: f32, dt: f32) -> f32 {
    if !(inertia > 0.0) {
        return omega;
    }
    let delta = (drive_torque + reaction) * dt / inertia;
    if delta.is_finite() {
        omega + delta
    } else {
        omega
    }
}

struct TyreSample {
    forward_speed: f32,
    denom: f32,
    load: f32,
    max_longitudinal: f32,
}

fn tyre_sample(chassis: &Body, wheel: &Wheel, push: &ContactPush) -> Option<TyreSample> {
    let heading = plane_heading(chassis, push.direction)?;
    let lateral = push.direction.cross(heading).normalize();
    if lateral.length_squared() == 0.0 || !(wheel.radius > 0.0) {
        return None;
    }
    let at_point = chassis.velocity + chassis.angular_velocity.cross(push.arm);
    let planar = at_point - push.direction * at_point.dot(push.direction);
    let forward_speed = planar.dot(heading);
    let lateral_speed = planar.dot(lateral);
    let denom = planar.length().max(wheel.slip_denom);
    let load = push.suspension.length();
    if !(load > 0.0) || !(denom > 1.0e-8) || !forward_speed.is_finite() {
        return None;
    }
    let mu_lat = signed_curve(slip_angle(lateral_speed, forward_speed), wheel).abs();
    let room = if mu_lat < 1.0 {
        (1.0 - mu_lat * mu_lat).sqrt()
    } else {
        0.0
    };
    let max_mu = wheel.mu_peak.min(room).max(0.0);
    Some(TyreSample {
        forward_speed,
        denom,
        load,
        max_longitudinal: (max_mu * load).min(load.abs()),
    })
}

/// `d(reaction) / d(omega)` on the rising part of the curve. Negative.
fn grip_slope(wheel: &Wheel, denom: f32, load: f32) -> Option<f32> {
    if !(wheel.radius > 0.0)
        || !(wheel.slip_peak > 0.0)
        || !(wheel.mu_peak > 0.0)
        || !(denom > 1.0e-8)
        || !(load > 0.0)
    {
        return None;
    }
    let slope = -wheel.radius * wheel.radius * load * wheel.mu_peak / (wheel.slip_peak * denom);
    (slope.is_finite() && slope < 0.0).then_some(slope)
}

fn drive_share(drive: &Drive, ground: &WheeledGround) -> f32 {
    let axle = axle_torque(drive, ground);
    if !axle.is_finite() {
        return 0.0;
    }
    let mut count: u8 = 0;
    for wheel in &ground.wheels {
        if wheel.driven && wheel.inertia > 0.0 && wheel.inertia.is_finite() {
            count = count.saturating_add(1);
        }
    }
    if count == 0 {
        0.0
    } else {
        axle / f32::from(count)
    }
}

fn axle_torque(drive: &Drive, ground: &WheeledGround) -> f32 {
    if drive.clutch.to_bits() != 1.0_f32.to_bits() || !drive.throttle.is_finite() {
        return 0.0;
    }
    let Some(ratio) = engaged_ratio(drive) else {
        return 0.0;
    };
    if !drive.final_drive.is_finite() {
        return 0.0;
    }
    let engine = mean_driven_omega(ground) * ratio * drive.final_drive;
    let rpm = engine.abs() * (60.0 / std::f32::consts::TAU);
    let axle = drive.throttle * curve_torque(drive, rpm) * ratio * drive.final_drive;
    if axle.is_finite() {
        axle
    } else {
        0.0
    }
}

fn engaged_ratio(drive: &Drive) -> Option<f32> {
    let index = usize::try_from(drive.gear).ok()?;
    let count = usize::from(drive.ratio_count);
    if index >= count || index >= drive.ratios.len() {
        return None;
    }
    let ratio = drive.ratios[index];
    ratio.is_finite().then_some(ratio)
}

fn mean_driven_omega(ground: &WheeledGround) -> f32 {
    let mut sum = 0.0;
    let mut count: u8 = 0;
    for wheel in &ground.wheels {
        if wheel.driven && wheel.omega.is_finite() {
            sum += wheel.omega;
            count = count.saturating_add(1);
        }
    }
    if count == 0 {
        0.0
    } else {
        sum / f32::from(count)
    }
}

/// Linear interpolation. Outside the table the end samples hold.
fn curve_torque(drive: &Drive, rpm: f32) -> f32 {
    let count = usize::from(drive.curve_count).min(drive.curve_rpm.len());
    if count == 0 || !rpm.is_finite() {
        return 0.0;
    }
    let rpm = rpm.abs();
    if rpm <= drive.curve_rpm[0] {
        return drive.curve_torque[0];
    }
    let last = count - 1;
    if rpm >= drive.curve_rpm[last] {
        return drive.curve_torque[last];
    }
    let mut index = 0;
    while index < last {
        let right = drive.curve_rpm[index + 1];
        if rpm <= right {
            let left = drive.curve_rpm[index];
            let span = right - left;
            let t = if span > 1.0e-8 {
                (rpm - left) / span
            } else {
                0.0
            };
            let low = drive.curve_torque[index];
            let high = drive.curve_torque[index + 1];
            return low + (high - low) * t;
        }
        index += 1;
    }
    drive.curve_torque[last]
}

/// Ground reaction on the wheel, N·m. A forward slip force slows a positive spin.
fn tyre_reaction(chassis: &Body, wheel: &Wheel, push: &ContactPush) -> f32 {
    let Some(heading) = plane_heading(chassis, push.direction) else {
        return 0.0;
    };
    let longitudinal = push.slip.dot(heading);
    if longitudinal.is_finite() && wheel.radius.is_finite() {
        -longitudinal * wheel.radius
    } else {
        0.0
    }
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
mod curve_tests {
    use super::{curve_torque, Drive};

    fn sloped() -> Drive {
        let mut curve_rpm = [0.0; 8];
        curve_rpm[0] = 1_000.0;
        curve_rpm[1] = 5_000.0;
        let mut torque = [0.0; 8];
        torque[0] = 100.0;
        torque[1] = 300.0;
        Drive {
            throttle: 1.0,
            clutch: 1.0,
            gear: 0,
            final_drive: 1.0,
            ratios: [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            ratio_count: 1,
            curve_rpm,
            curve_torque: torque,
            curve_count: 2,
        }
    }

    #[test]
    fn torque_table_interpolates_inside_and_holds_at_the_ends() {
        let drive = sloped();
        assert!((curve_torque(&drive, 0.0) - 100.0).abs() < 1.0e-4);
        assert!((curve_torque(&drive, -500.0) - 100.0).abs() < 1.0e-4);
        assert!((curve_torque(&drive, 1_000.0) - 100.0).abs() < 1.0e-4);
        assert!((curve_torque(&drive, 3_000.0) - 200.0).abs() < 1.0e-3);
        assert!((curve_torque(&drive, 5_000.0) - 300.0).abs() < 1.0e-4);
        assert!((curve_torque(&drive, 9_000.0) - 300.0).abs() < 1.0e-4);
    }
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
            driven: false,
            inertia: 0.0,
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
