# 0040: A vehicle is domain forces on one chassis body

Status: **Proposed** (7 Nov 2026). This file does not accept itself. G1, the four-wheel ground payload, is in the step as a trial. P28's slip curve is in the step on that chassis. Aircraft forces, space forces, the landing stage, and sea are not in that trial.

Design for review. Ground, aircraft, and space are the domains this proposal would commit to, each with its own payload and its own fixture. Sea is written down as buoyancy and quadratic drag on that same body, and it is not a domain, not a type, and not a stage.

## Context

The pipeline plan sketches a wheeled car (P27–P31) and a thruster plus a wide orbit (P32). Those boxes were merged with the plan. They are not a reviewed vehicle decision. [ADR 0033](0033-physics-scale-broadphase-and-sleep.md) says vehicles are not decided there. No open pull request implements a vehicle.

Master already has the pieces a vehicle has to live on:

- [ADR 0016](0016-physics-ccd-destruction.md). `MotionQuality` is `Discrete`, `Speculative`, or `Swept`. Swept time of impact uses conservative advancement. A wheel ray is a different query.
- [ADR 0034](0034-rotational-contact-solver.md). Contact applies an impulse at a point. Inertia is diagonal in the body frame. The implicit gyroscopic step is the attitude integrator. `Body.spring` pushes along the line to a fixed anchor and does not pull. A body with a spring or a motor does not sleep.
- [ADR 0033](0033-physics-scale-broadphase-and-sleep.md). Statics and dynamics sit in separate AABB trees. An island sleeps after 0.5 s under 0.12 m/s and 0.25 rad/s. Asleep bodies are not integrated and are not solved.
- `debug/physics` (`genos-physics`) is the headless harness: scenes in code, one JSON line per step, FNV hash, `--replay`, `--checkpoint`. It has no window and no vehicle scene.

P27 cannot be implemented as written. It asks for a `BodyId`, a ray or cylinder cast, and the P10 soft constraint. `World::insert` returns a `usize`. There is no ray query. P10's sub-stepped soft solver is not in `step`. The contact solver is the sequential point solver from ADR 0034.

Anthony's scope for this decision: ground, aircraft, and space are first-class. Sea may exist later, and it means buoyancy. The common record must not assume wheels.

## Decision

1. A vehicle is extra state on one chassis `Body`. The chassis index is the `usize` from `World::insert`. A wheel, a wing, and a thruster are not bodies. There is no new crate. [ADR 0007](0007-native-calls-go-to-the-system-and-vulkan.md) stays in force.
2. The common record is the chassis index plus three optional payloads: ground, aircraft, and space. Wheels exist only inside the ground payload. Throttle and steer exist only inside the payload that uses them. The common record has no wheel list and no shared stick.
3. Ground's first payload is four wheel rays and a spring-damper along each ray. The spring-damper is an explicit force, using the frequency and damping ratio from Erin Catto's soft constraint (`ω = 2π hz`, `k = m ω²`, `c = 2 ζ m ω`). It is not a P10 sub-step and it is not `Body.spring`.
4. Aircraft's first payload is steady lift, drag, and thrust through the centre of mass. Lift coefficient is a stored constant. Angle of attack is not read.
5. Space's first payload is a short list of thrusters. Force is `throttle * max_force` along the local direction. Torque is `r × F` on the chassis. The existing integrator consumes that torque. There is no second integrator and no wide-orbit state.
6. Two payloads may be set together only in the landing stage: aircraft plus wheeled ground. Force order is aircraft, then space, then ground. Space combined with either other payload is outside this proposal.
7. Sea is not decided. The representation, if a later ADR accepts it, is buoyancy from displaced volume plus quadratic water drag, both applied to the chassis body. It is not a ground variant and it has no wheels. This ADR does not add the field.
8. The first code, after acceptance, lands as separate stages. G1, A1, and S1 each have a fixture and none of them requires the other. T1 waits until G1 and A1 are both in the tree. S2 (the plan's orbit) waits until wide positions exist. Sea has no stage.
9. An empty vehicle list leaves `step` as it is. `World::new` stays a const function. A later step of a world with no vehicles still does not allocate.

## Types

These structs are the shape of the implementing commit. They are not in the crate now.

```rust
pub struct Vehicle {
    pub chassis: usize,
    pub ground: Option<GroundContact>,
    pub aircraft: Option<Aircraft>,
    pub space: Option<SpaceCraft>,
}

pub enum GroundContact {
    Wheeled(WheeledGround),
}

pub struct Wheel {
    pub local_position: Vec3,
    pub direction: Vec3,
    pub rest_length: f32,
    pub min_length: f32,
    pub max_length: f32,
    pub hz: f32,
    pub damping_ratio: f32,
}

pub struct WheeledGround {
    pub wheels: [Wheel; 4],
}

pub struct Aircraft {
    pub wing_area: f32,
    pub cl: f32,
    pub cd0: f32,
    pub induced: f32,
    pub thrust_max: f32,
    pub throttle: f32,
    pub air_density: f32,
}

pub struct Thruster {
    pub local_position: Vec3,
    pub local_direction: Vec3,
    pub max_force: f32,
    pub throttle: f32,
}

pub struct SpaceCraft {
    pub thrusters: [Thruster; 8],
    pub count: u8,
}
```

`World` would store `vehicles: Vec<Vehicle>`, empty from `World::new`. `World`'s `PartialEq` would include that list. Scratch for the last wheel length sits on the vehicle and is updated in place.

A later sea ADR would add one more optional field, and only then:

```rust
pub struct Displacement {
    pub water_density: f32,
    pub water_y: f32,
    pub drag_coeff: f32,
    pub drag_area: f32,
}
```

That field is not part of `Vehicle` under this proposal.

## Invariants

- The chassis is one dynamic body in the dynamic tree. Vehicle code does not change `MotionQuality`, does not write `ccd_applied`, `toi_failed`, or `pair_slices`, and does not insert a body for a wheel, a wing, or a thruster.
- A wheel ray is a segment from the attach point along `direction`, from `min_length` to `max_length`, excluding the chassis index. It is not a speculative contact and it is not a time of impact. G1 resolves a plane (`normal · point = offset`) and an axis-aligned box (slab test in the box frame). A mesh or a hull is a miss in G1. Hull SAT in `contact.rs` stays as it is.
- Suspension pushes only. With `direction` pointing from the chassis toward the surface, `length` is the hit distance. `compression = rest_length - length`. Force is zero when the ray misses, when `length` is outside `[min_length, max_length]`, or when `compression <= 0`. Otherwise `magnitude = k * compression - c * compression_rate`, with `compression_rate = -(direction · v_attach)` and `v_attach = v + ω × r`. The push is `force = -direction * magnitude` at the hit point. The vehicle code integrates it itself: `velocity += force * inverse_mass * dt`, and `angular_velocity += I⁻¹_world (r × force) * dt`, where `I⁻¹_world` is the ADR 0034 world inverse inertia. `integrate_body` then runs the gyroscopic step. The contact solver is not the suspension path.
- Suspended mass is `chassis_mass / wheels.len()`. `k = m_s * (2π hz)²`. `c = 2 * damping_ratio * m_s * (2π hz)`.
- Aircraft forward is local +X. `q = 0.5 * air_density * speed * speed`, `speed = |v|`. `lift = cl * q * wing_area` along world +Y. `cd = cd0 + induced * cl * cl`. `drag_force = cd * q * wing_area` along `-v` (zero when speed is zero). `thrust = throttle * thrust_max` along local +X. All three act at the centre of mass, so aircraft torque is zero in A1.
- A thruster's world force is `orientation * normalize(local_direction) * throttle * max_force`. World torque is `(orientation * local_position) × force`. The same `velocity` and `angular_velocity` update as the suspension force consumes it. Gravity is `World.gravity`. The space payload does not zero it. The S1 fixture sets gravity to zero. Aircraft lift, drag, and thrust use that linear update with zero torque.
- Domain code does not set `Body.spring` or `Body.motor`. A parked chassis can sleep under the existing rule. While it sleeps, the wheel-ray count is 0 and suspension force is 0. An aircraft with `throttle > 0`, or a thruster with `throttle > 0`, holds the chassis awake the way a motor does.
- Force order in the existing dynamic loop, after `apply_forces` and before `integrate_body`: aircraft, space, ground. Torque is added to `angular_velocity` before `implicit_gyroscopic`. Swept motion still integrates inside `ccd.rs`.
- Wheel index order and thruster index order are the only orders. There is no hash-map walk. `atan2` does not order wheels, contacts, or actuators. P28 uses it inside one wheel for the slip angle from the direction of travel, `atan2(lateral speed, |forward speed|)`, so straight reverse at zero sideslip is not a lateral force. Two runs in one process match `f32` bits of pose, velocity, and stored wheel lengths.
- A world with no vehicles does no vehicle work. The allocation bench stays green.

## Stages and fixtures

G1 is in the tree as a trial. This ADR stays proposed. A1, S1, and T1 are not implemented. `dt` is `1/60` s. `GRAVITY` is `(0, -9.81, 0)`.

| Stage | Payload | Waits on | Fixture |
| --- | --- | --- | --- |
| G1 | `Wheeled` only | in the step as a trial | `a_chassis_rests_on_four_springs`, `a_parked_chassis_sleeps_and_stops_casting` |
| A1 | `Aircraft` only | acceptance | `a_trimmed_aircraft_holds_altitude_and_speed` |
| S1 | `SpaceCraft` only | acceptance | `a_thruster_in_zero_gravity_matches_force_and_torque` |
| T1 | aircraft and wheeled ground | G1 and A1 | `a_gear_that_misses_matches_the_aircraft`, `a_gear_on_the_plane_shares_the_weight` |
| S2 | orbit | wide positions, which are not in `step` | the plan's `a_circular_orbit_at_1e7_metres_keeps_its_radius` |
| Sea | none | a later ADR | none |

P28 (tyre slip) is in the step on the G1 chassis as the plan's three-point curve. This ADR does not restate that curve. P29 (gearbox and open differential) stays in the plan and waits. Shape-cast wheels, anti-roll bars, and Pacejka stay out.

### G1. Four wheel rays

Chassis mass 1 200 kg, box half extents `(1.2, 0.15, 0.7)` m, start `(0, 0.45, 0)`, identity orientation, friction 0.6, restitution 0, `MotionQuality::Discrete`. Four wheels at local `(±1.0, 0, ±0.5)`, direction `(0, -1, 0)`, rest 0.4 m, min 0.1 m, max 0.6 m, `hz = 4`, damping ratio 0.7. One static plane, normal +Y, offset 0. No aircraft payload and no space payload.

Suspended mass is 300 kg. `ω = 2π * 4`. `k = 300 ω² = 189496.4045` N/m. Compression at rest is `(1 200 * 9.81 / 4) / k = 0.015531` m. Settled centre height is `0.4 - 0.015531 = 0.384469` m. The box bottom is then `0.384469 - 0.15 = 0.234` m above the plane, so the chassis shape is not the support. `wheel_rays` is the counter the plan called `wheel_cast_count`.

`a_chassis_rests_on_four_springs`. `World.sleep_time = 10` so 3 s cannot sleep the chassis. After 180 steps: centre `y` within 0.02 m of 0.384469, `|velocity.y| < 0.05`, the angle between body up and world up under 0.01 rad, `wheel_rays == 4`, `normal_impulse == 0`, `ccd_applied` is false, `toi_failed == 0`, `pair_slices == 0`. A second world matches the chassis pose and velocity bit for bit.

`a_parked_chassis_sleeps_and_stops_casting`. The same world with the default `sleep_time` of 0.5 s. After 180 steps the chassis is sleeping, centre `y` is still within 0.02 m of 0.384469, and `wheel_rays` on the last step is 0.

`debug/physics --scene chassis_rest` runs the awake fixture for 180 steps. The JSON line gains `wheel_rays` in that commit. The hash gains each wheel length's bits in that commit.

### A1. Trimmed aircraft

Mass 1 000 kg, any box, position `(0, 100, 0)`, velocity `(50, 0, 0)`, identity orientation, `Discrete`. No plane. No ground payload and no space payload. `wing_area = 10`, `air_density = 1.225`, `cd0 = 0.02`, `induced = 0.05`, `throttle = 1`.

The test and the force use this association, where every name is an `f32` and `weight = mass * -gravity.y`:

```text
q = 0.5 * air_density * (speed * speed)
cl = weight / (q * wing_area)
cd = cd0 + induced * (cl * cl)
thrust_max = (q * wing_area) * cd
```

Those `f32` operations produce `cl ≈ 0.640653` and `thrust_max = drag ≈ 620.490` N, and in that association lift equals weight and thrust equals drag, so the residual acceleration is 0.

`a_trimmed_aircraft_holds_altitude_and_speed`. After 120 steps: `position.y` bits equal the start, `velocity.x` bits equal 50, `velocity.y` bits equal 0, `angular_velocity` bits equal 0, `wheel_rays == 0`. A second run matches bits. Position `x` moves with `velocity * dt` and is not compared to 100.

`debug/physics --scene aero_trim` runs those 120 steps.

### S1. One thruster

Mass 1 000 kg, box half extents `(1, 1, 1)`, origin, identity orientation. `World.gravity` is zero. No ground payload and no aircraft payload. One thruster, `max_force = 1000`, `throttle = 1`, direction local +X.

Box inertia is `Ixx = m/3 (hy² + hz²)`. Here `I = 2000/3` kg·m² on each axis. A cube's principal moments are equal, so the implicit gyroscopic term leaves a principal-axis spin unchanged.

`a_thruster_in_zero_gravity_matches_force_and_torque`, 60 steps.

- Through the centre, local position 0. `|velocity.x - 1| < 1e-3`, `|velocity.y|` and `|velocity.z| < 1e-4`, `|angular_velocity| < 1e-4`.
- Offset local position `(0, 2, 0)`. Torque in the world stays `(0, 0, -2000)` N·m while the body yaws about Z, and the thrust yaws with the body, so `velocity.x` is not 1. `|ωz + 3| / 3 <= 0.01`, `|ωx|` and `|ωy| < 1e-3`. Speed stays finite.
- `wheel_rays == 0`. A second run matches bits. No `f64` state and no origin shift.

`debug/physics --scene thruster` runs the offset case for 60 steps.

### T1. Gear on an aircraft

`a_gear_that_misses_matches_the_aircraft`. The A1 aircraft, plus four wheels with `max_length = 0.5` and the G1 spring numbers. No plane. Centre `y` is 100, so every ray misses. After 120 steps the chassis pose and velocity match the A1 twin bit for bit, and `wheel_rays == 4`.

`a_gear_on_the_plane_shares_the_weight`. Mass 1 000 kg, so suspended mass is 250 kg and `k = 157913.670` N/m. The same wheels as G1. The plane from G1. Forward speed 20 m/s. `cl` stays the A1 constant. `q = 0.5 * 1.225 * 400 = 245`. Lift is 1 569.6 N. The springs carry `9810 - 1569.6 = 8240.4` N. Compression per wheel is 0.013046 m. Settled centre height is 0.386954 m. `thrust_max` is the drag at 20 m/s from the A1 `cd` formula, `throttle = 1`. After 180 steps, with `sleep_time = 10`: centre `y` within 0.02 m of 0.386954, `|velocity.y| < 0.05`, `|velocity.x - 20| < 0.05`, body up within 0.01 rad of world up. A second run matches bits.

## Sea

Sea is an open question, not a commitment. Anthony described it as buoyancy. This section is the representation a later ADR would accept or drop. No fixture here is a gate. No type here is added. No stage is numbered.

The force sits on the chassis body, the same rigid body the other payloads already move. Gravity still comes from `World.gravity`. There is no water wheel, no planing mode, and no ground variant called a boat.

Water is a flat plane `y = water_y` and a density. The first future fixture is an upright box. Submerged height is `clamp(water_y - (centre_y - hy), 0, 2 hy)`. Displaced volume is that height times the horizontal face area `4 * hx * hz`. A fully covered box uses its full volume. Buoyancy is `(0, density * g * volume, 0)` with `g = -gravity.y`, applied at the centroid of the submerged slice. Upright, that centroid is on the vertical through the centre, so the torque is zero. Drag is `-0.5 * density * drag_coeff * drag_area * |v| * v`, applied at the centre of mass, so drag adds no torque. There is no angular drag, no free surface, no added mass, no grid, and no particle fluid.

The numbers a later ADR can turn into tests, checked here in `f32`:

| Check | Setup | Result |
| --- | --- | --- |
| Draft cancels weight | mass 1 000 kg, half extents `(1, 0.5, 1)`, density 1 000, `water_y = 0`, centre `y = 0.25`, velocity 0 | submerged height 0.25 m, volume 1 m³, buoyancy 9 810 N, one step leaves `velocity.y` at 0 |
| Horizontal drag | the same pose, `velocity.x = 4`, `drag_coeff = 1`, `drag_area = 1` | drag is −8 000 N, one step leaves `velocity.x` at the `f32` value of `4 + (-8) * dt` (3.866667 at `f32` `dt`) and `velocity.y` at 0 |

A multi-second drop is not a proposed gate. With `drag_coeff = 1` and `drag_area = 4`, a semi-implicit heave from centre `y = 1` is still 0.22 m/s after 5 s, because quadratic drag fades as the speed falls. A settle assertion on that setup would miss or would be written loosely enough to pass while still moving. The later ADR picks a drag and a check together.

An amphibian, a hull that also rolls on wheels, would compose `Displacement` with `Wheeled` the way T1 composes aircraft with `Wheeled`. That composition is not proposed here.

## Debug harness

The implementing stage extends `debug/physics`. It does not add `genos-vehicle-view` and it does not open a window. Scenes stay in `debug/physics/src/scenes.rs`.

| Failure | Tunable | Probe |
| --- | --- | --- |
| The chassis rests at the wrong height | `hz`, wheel rest length, mass | `a_chassis_rests_on_four_springs`, `chassis_rest` |
| A parked chassis keeps casting | `sleep_time` 0.5 s | `a_parked_chassis_sleeps_and_stops_casting`, `wheel_rays` |
| A wheel ray is treated as swept collision | `MotionQuality::Discrete` | `ccd_applied`, `toi_failed`, `pair_slices` on the G1 fixture |
| Trim drift or a surprise moment | `cl`, `thrust_max`, `induced` | `aero_trim` and the A1 bit match |
| A thruster misses force or torque | `max_force`, local position | `thruster` and the S1 bounds |
| Gear that misses still pushes | `max_length` | `a_gear_that_misses_matches_the_aircraft` |
| Two runs diverge | actuator order | the second world in each fixture |
| A locked wheel does not slow the chassis | `mu_peak`, `radius`, wheel `omega` | `a_locked_wheel_slows_the_car_and_a_free_wheel_rolls`, `locked_wheel`, `tyre_contacts` |
| A free wheel or a zero curve is braked | `omega`, `mu_peak` 0 | the free-wheel and zero-mu runs in that test |
| Straight reverse gains a lateral speed | slip angle from the travel direction | `a_matched_roll_in_reverse_stays_straight_and_sideslip_restores` |

`wheel_rays` and `vehicle_count` join the JSON line in the first commit that has a vehicle. Until that commit the line is unchanged. P28 adds `tyre_contacts` on that line and mixes each wheel's `omega` into the hash. `--replay` and `--checkpoint` keep their current meaning.

## Networking

Gameplay networking still waits. This proposal does not open a socket, does not define `InputFrame`, and does not add a snapshot codec. The core-networking plan owns those. The five lines the implementing commit copies into `docs/systems/physics.md`:

- Replicated state: the chassis uses the same quantized pose and velocity as any dynamic body. G1 adds each wheel's suspension length as `i16`. A1 adds throttle as `i16`. S1 adds each thruster throttle as `i16`. Contact scratch, slip, the lift sample, and the thruster force stay local. Sea is absent, so draft and drag are not on the wire.
- Authority: the server runs `step` and owns the chassis pose. The owning client sends input. Clients do not accept one another's vehicle state.
- Prediction / rollback: none. A driven chassis is interpolated. Predicting it would roll back contacts with the world.
- Determinism: one process, `step` on one thread, fixed `1/60` s, the force order above. Two runs match bits. The wire compares quantized values. Cross-platform bit identity is not required.
- Headless server: the same `step`, no window and no Vulkan device. `debug/physics` is already that path.

## Papers and departures

- Erin Catto, "Soft Constraints: Reinventing The Spring" (GDC 2011), the mapping `ω = 2π f`, `k = m ω²`, `c = 2 ζ m ω`. Departure: the force is applied along the wheel ray on the chassis. It is not a constraint row, and the world is not sub-stepped. P10 is not in `step`.
- Jolt `VehicleConstraint` / `WheeledVehicleController`: the wheel is not a body. Departure: G1 has no constraint object in the contact solver. Cylinder casts, anti-roll, slip, and the drivetrain are not in G1. The plan's `WheelCast::Shape` waits.
- John D. Anderson, *Introduction to Flight*, the definitions `q = ½ ρ V²`, `L = q S CL`, `D = q S CD`. Departure: `CL` is constant, angle of attack is not sampled, there is no control derivative, and every aircraft force acts at the centre of mass.
- Euler's equation for a rigid body, torque `r × F`, integrated by the implicit gyroscopic step already accepted in ADR 0034 (Catto, "Numerical Methods for Rigid Body Dynamics", 2015). Departure from plan P32: S1 does not integrate `WidePos`, does not use velocity Verlet, and does not take a gravitational parameter. The orbit fixture stays S2.
- Archimedes, *On Floating Bodies*: the buoyant force equals the weight of the displaced liquid. Quadratic drag is `½ ρ Cd A v²`. Departure, and only inside the undecided sea section: an upright box, drag at the centre, no free surface. Sea itself is not accepted.

## Rejected

- A common `Vehicle` that stores a wheel array and leaves it empty for aircraft and spacecraft. That makes every domain a car.
- A trait object, a force registry, or a `Vec` of boxed actuators. The payloads are three named fields.
- Sea as a fourth accepted payload, as a wheeled watercraft, or as a fluid simulation.
- Adopting Rapier, Jolt, or PhysX. ADR 0007 forbids that.
- Blocking G1 on P10, or blocking A1 and S1 on G1.
- A new `genos-vehicle-view` binary before the scenes exist on `debug/physics`.
- Predicting the chassis.
- Putting the orbit helper in S1 while positions are still `f32` world coordinates.
- Marking this proposal accepted in the commit that adds it.

## Consequences

`docs/systems/physics.md` records that a vehicle is not in the step. The commit that implements a stage updates the game use, the limits, and the five networking lines, and it adds the public words (`vehicle`, chassis payload, wheel ray) to `CONTEXT.md`.

Existing scenes stay the regression floor: resting pile, hull edges, ship lanes, the 10k static grid, sleep islands, and `cargo test -p genos-physics`. A vehicle stage that needs a looser assertion on those tests is wrong.

Acceptance of this ADR is Anthony's. Until then the stage table is a design, and the boxes in the pipeline plan stay unchecked.
