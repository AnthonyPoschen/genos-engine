//! Domain forces on one chassis body.
//!
//! The record holds a ground payload, an aircraft payload, and a space payload.
//! This stage applies the wheeled ground spring-damper, the space thrusters, and
//! the wide orbit on that space payload. Aircraft lift is a later stage and is
//! not applied here. Sea is not a field.
//! A thruster writes force and torque through [`integrate_wrench`]. The existing
//! gyroscopic step then consumes that angular velocity. An orbit integrates its
//! own wide position and replaces the chassis linear pose. It does not replace
//! the gyroscopic step, and it does not pull a body that has no orbit.

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

/// Metres stored in `f64`. This is not a [`genos_math::Vec3`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WidePos {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl WidePos {
    /// The origin frame. [`World::new`](crate::World::new) starts here.
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    #[must_use]
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    fn length_squared(self) -> f64 {
        self.x * self.x + self.y * self.y + self.z * self.z
    }

    fn translate(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }

    fn offset(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }

    fn scale(self, factor: f64) -> Self {
        Self::new(self.x * factor, self.y * factor, self.z * factor)
    }
}

/// Wide orbit on one chassis. The step does not read [`Body::position`](crate::Body::position)
/// as this state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Orbit {
    pub position: WidePos,
    pub velocity: WidePos,
    /// Gravitational parameter, m³/s². Zero coasts.
    pub gm: f64,
}

/// A short thruster list. `count` is how many entries are live.
///
/// `orbit` is the S2 wide state. `None` leaves linear motion on the thruster path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpaceCraft {
    pub thrusters: [Thruster; 8],
    pub count: u8,
    pub orbit: Option<Orbit>,
}

/// Gravitational parameter of the S2 fixture, m³/s².
pub const ORBIT_GM: f64 = 3.986e14;

/// Circular radius of the S2 fixture, metres.
pub const ORBIT_RADIUS_M: f64 = 1.0e7;

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
            orbit: None,
        }),
    });
    world
}

/// The S2 fixture: a circular orbit at [`ORBIT_RADIUS_M`], gravity zero.
///
/// The chassis integrates `WidePos` with velocity Verlet. A second body sits
/// 1 000 m further along +X and has no orbit, so the helper does not pull it.
/// `sleep_time` of 10 keeps a calm body awake for a few seconds. The 200 s
/// radius check outlasts that, and a calm body stays where it is.
#[must_use]
pub fn circular_orbit_world() -> World {
    let mut world = World::new(Vec3::ZERO);
    world.sleep_time = 10.0;
    let speed = (ORBIT_GM / ORBIT_RADIUS_M).sqrt();
    let position = WidePos::new(ORBIT_RADIUS_M, 0.0, 0.0);
    let velocity = WidePos::new(0.0, speed, 0.0);
    let mut body = Body::cuboid(
        pose_of(position).unwrap_or(Vec3::ZERO),
        1_000.0,
        Vec3::new(1.0, 1.0, 1.0),
    );
    body.velocity = pose_of(velocity).unwrap_or(Vec3::ZERO);
    body.friction = 0.0;
    body.restitution = 0.0;
    body.motion = MotionQuality::Discrete;
    let index = world.insert(body);
    let mut nearby = Body::cuboid(
        pose_of(WidePos::new(ORBIT_RADIUS_M + 1_000.0, 0.0, 0.0)).unwrap_or(Vec3::ZERO),
        1_000.0,
        Vec3::new(1.0, 1.0, 1.0),
    );
    nearby.friction = 0.0;
    nearby.restitution = 0.0;
    nearby.motion = MotionQuality::Discrete;
    world.insert(nearby);
    world.vehicles.push(Vehicle {
        chassis: index,
        ground: None,
        aircraft: None,
        space: Some(SpaceCraft {
            thrusters: [Thruster {
                local_position: Vec3::ZERO,
                local_direction: Vec3::X,
                max_force: 0.0,
                throttle: 0.0,
            }; 8],
            count: 0,
            orbit: Some(Orbit {
                position,
                velocity,
                gm: ORBIT_GM,
            }),
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

/// Integrate each wide orbit and write the chassis linear pose.
///
/// Runs after [`integrate_body`](crate::integrate_body), so the `f32` step does
/// not keep a second copy of the same translation. A sleeping chassis is left
/// alone. A non-finite parameter leaves the stored wide state unchanged.
pub(crate) fn sync_orbits(world: &mut World, dt: f32) {
    if world.vehicles.is_empty() || !(dt > 0.0) {
        return;
    }
    if !world.vehicles.iter().any(has_orbit) {
        return;
    }
    let dt64 = f64::from(dt);
    let origin = world.origin;
    let count = world.vehicles.len();
    for index in 0..count {
        let Some(vehicle) = world.vehicles.get(index).copied() else {
            continue;
        };
        let Some(mut craft) = vehicle.space else {
            continue;
        };
        let Some(mut orbit) = craft.orbit else {
            continue;
        };
        let Some(body) = world.bodies.get(vehicle.chassis).copied() else {
            continue;
        };
        if body.presence != Presence::Live || body.inverse_mass == 0.0 || body.sleeping {
            continue;
        }
        if !advance_orbit(&mut orbit, dt64) {
            continue;
        }
        let Some(position) = pose_of(orbit.position.offset(origin)) else {
            continue;
        };
        let Some(velocity) = pose_of(orbit.velocity) else {
            continue;
        };
        craft.orbit = Some(orbit);
        if let Some(stored) = world.vehicles.get_mut(index) {
            stored.space = Some(craft);
        }
        if let Some(stored) = world.bodies.get_mut(vehicle.chassis) {
            stored.position = position;
            stored.velocity = velocity;
        }
    }
}

/// Move the `f32` poses into `origin`. Wide positions stay absolute.
pub(crate) fn shift_origin(world: &mut World, origin: WidePos) {
    if !origin.is_finite() || same_wide(world.origin, origin) {
        return;
    }
    let delta = origin.offset(world.origin);
    world.origin = origin;
    for body in &mut world.bodies {
        body.position = Vec3::new(
            rebase_axis(body.position.x, delta.x),
            rebase_axis(body.position.y, delta.y),
            rebase_axis(body.position.z, delta.z),
        );
    }
    write_orbit_poses(world);
    world.refit();
}

fn has_orbit(vehicle: &Vehicle) -> bool {
    vehicle.space.is_some_and(|craft| craft.orbit.is_some())
}

fn write_orbit_poses(world: &mut World) {
    let origin = world.origin;
    let count = world.vehicles.len();
    for index in 0..count {
        let Some(vehicle) = world.vehicles.get(index).copied() else {
            continue;
        };
        let Some(orbit) = vehicle.space.and_then(|craft| craft.orbit) else {
            continue;
        };
        let Some(position) = pose_of(orbit.position.offset(origin)) else {
            continue;
        };
        if let Some(body) = world.bodies.get_mut(vehicle.chassis) {
            body.position = position;
        }
    }
}

/// Velocity Verlet. `a = -GM * r / |r|³`.
fn advance_orbit(orbit: &mut Orbit, dt: f64) -> bool {
    if !dt.is_finite() || dt <= 0.0 {
        return false;
    }
    if !orbit.gm.is_finite() || !orbit.position.is_finite() || !orbit.velocity.is_finite() {
        return false;
    }
    let Some(kick) = pull(orbit.position, orbit.gm) else {
        return false;
    };
    let half = dt * 0.5;
    let midway = orbit.velocity.translate(kick.scale(half));
    let next = orbit.position.translate(midway.scale(dt));
    let Some(land) = pull(next, orbit.gm) else {
        return false;
    };
    let velocity = midway.translate(land.scale(half));
    if !next.is_finite() || !velocity.is_finite() {
        return false;
    }
    orbit.position = next;
    orbit.velocity = velocity;
    true
}

fn pull(position: WidePos, gm: f64) -> Option<WidePos> {
    let r2 = position.length_squared();
    if !(r2 > 0.0) || !r2.is_finite() {
        return None;
    }
    let radius = r2.sqrt();
    let cube = r2 * radius;
    if !(cube > 0.0) || !cube.is_finite() {
        return None;
    }
    let accel = position.scale(-gm / cube);
    accel.is_finite().then_some(accel)
}

fn same_wide(left: WidePos, right: WidePos) -> bool {
    left.x.to_bits() == right.x.to_bits()
        && left.y.to_bits() == right.y.to_bits()
        && left.z.to_bits() == right.z.to_bits()
}

fn rebase_axis(current: f32, delta: f64) -> f32 {
    let next = pose_axis(f64::from(current) - delta);
    if next.is_finite() {
        next
    } else {
        current
    }
}

fn pose_of(value: WidePos) -> Option<Vec3> {
    if !value.is_finite() {
        return None;
    }
    let pose = Vec3::new(pose_axis(value.x), pose_axis(value.y), pose_axis(value.z));
    finite_vec(pose).then_some(pose)
}

/// Chassis pose is `f32`. A subnormal metre becomes zero; the wide state stays `f64`.
fn pose_axis(value: f64) -> f32 {
    if !value.is_finite() {
        if value.is_nan() {
            return f32::NAN;
        }
        return if value.is_sign_negative() {
            f32::NEG_INFINITY
        } else {
            f32::INFINITY
        };
    }
    let bits = value.to_bits();
    let sign = bits >> 63;
    let exp64 = (bits >> 52) & 0x7ff;
    let mant = bits & 0x000f_ffff_ffff_ffff;
    if exp64 < 897 {
        return signed_zero(sign);
    }
    if exp64 > 1150 {
        return signed_infinity(sign);
    }
    let mut exp = exp64 - 896;
    let mut frac = mant >> 29;
    let round = (mant >> 28) & 1;
    let sticky = mant & 0x0fff_ffff;
    let odd = (frac & 1) == 1;
    let tie_up = sticky != 0 || odd;
    if round == 1 && tie_up {
        frac += 1;
        if frac == 0x80_0000 {
            frac = 0;
            exp += 1;
            if exp >= 255 {
                return signed_infinity(sign);
            }
        }
    }
    let packed = (sign << 31) | (exp << 23) | frac;
    let Ok(narrow) = u32::try_from(packed) else {
        return signed_infinity(sign);
    };
    f32::from_bits(narrow)
}

fn signed_zero(sign: u64) -> f32 {
    if sign == 0 {
        0.0
    } else {
        -0.0
    }
}

fn signed_infinity(sign: u64) -> f32 {
    if sign == 0 {
        f32::INFINITY
    } else {
        f32::NEG_INFINITY
    }
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

#[cfg(test)]
mod pose_tests {
    use super::pose_axis;

    #[test]
    fn a_finite_metre_narrows_to_the_chassis_pose() {
        assert_eq!(pose_axis(0.0).to_bits(), 0);
        assert_eq!(pose_axis(-0.0).to_bits(), 0x8000_0000);
        assert_eq!(pose_axis(1.0).to_bits(), 0x3f80_0000);
        assert_eq!(pose_axis(-1.0).to_bits(), 0xbf80_0000);
        assert_eq!(pose_axis(0.5).to_bits(), 0x3f00_0000);
        assert_eq!(pose_axis(-2.5).to_bits(), 0xc020_0000);
        assert_eq!(pose_axis(1.0e7).to_bits(), 0x4b18_9680);
        assert_eq!(pose_axis(6_313.477_647_065_839).to_bits(), 0x45c5_4bd2);
        assert!(pose_axis(f64::NAN).is_nan());
        assert_eq!(pose_axis(f64::INFINITY), f32::INFINITY);
        assert_eq!(pose_axis(f64::NEG_INFINITY), f32::NEG_INFINITY);
        assert_eq!(pose_axis(1.0e-40).to_bits(), 0);
    }
}
