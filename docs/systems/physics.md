# Physics

The physics step moves bodies. It applies gravity, a move wish, contact, and springs.

## Goal

A game can simulate a falling mass, a bounce, a hit between two masses, friction, and one spring. A capsule, a box, and a simplified mesh take part in that hit.

## Intent

Gravity is `GRAVITY`, along -Y. There is no air drag. `step` takes a world and a time step and updates that world in place. The first step may size the contact lists. A later step does not allocate.

Each body has a motion quality. `Discrete` is the default and resolves an overlap that already exists. `Speculative` grows the contact distance by `min(speed * dt, speculative margin)` before the move, and the solver stops that approach. The default margin is 0.25 m. `Swept` runs time of impact on the pairs whose bounds meet that body. The advance is `gap / (|v_a − v_b| + |ω_a| r_a + |ω_b| r_b)`. `angular_velocity` is world-space, radians per second. `r` is the bounding radius. There is no scale factor. A pair whose relative travel over the tick exceeds the smaller bounding radius advances in slices of at most that radius. The loop stops at a 1 mm gap or when the tick is used. After 16 advances with the gap still above 1 mm, the pair stays at the last safe time and `toi_failed` increments. Pairs past `swept_pair_cap` (default 8) use the speculative margin for that tick and increment `toi_deferred`. Bodies that are not in a swept pair keep the one integration.

When `motor` is set, the step copies `wish.x` and `wish.z` into the velocity before contact. Contact then removes the part of that wish that points into a surface.

Contact restitution is the larger value on the two bodies. Friction is the larger value too. A spring pushes while the body is closer than `rest_length`. It does not pull. Rest compression on a vertical spring is `mass * -GRAVITY.y / stiffness`.

A breakable starts as one body from a cooked `.gfract` asset. Chunks are not bodies until a bond's health reaches 0. `apply_damage` is the one call, whether the amount is a contact impulse or a game value. At health 0 the unsupported leaves spawn with the intact body's linear and angular velocity. A chunk that can still walk a live bond to `WORLD` stays inside the intact body, and that body stays static. Children of a still-supported parent stay unspawned. A spawned chunk under 0.02 m³ is debris. Debris does not collide with debris. It sleeps after 0.1 s under 0.4 m/s. Past `debris_budget`, the smallest asleep debris are tombstoned in place so later body indices do not move. `allow_slice` stays false. `step` does not slice. There is no fixed-joint solver, so a remaining bond between two dynamic chunks does not pull them together. A contact impulse on the intact body is applied to the largest non-world bond. Chunk-versus-chunk contacts do not call `apply_damage`. A world bond has no face, so its health is `health_per_area * 10`.

A capsule is a segment along local Y plus a radius. `half_height` is the segment half-length. A box uses `half_extents` on the local axes. `orientation` aims the capsule segment and the box. The step turns `orientation` only when `angular_velocity` is non-zero: `dq/dt = ½ ω q`, after an implicit gyroscopic step in the body frame so a free spin does not gain energy. Contact writes a torque. Each narrow-phase hit carries up to four world-space points. The normal impulse uses `v + ω × r` and the body-frame inertia rotated into the world. Friction is two tangent rows at the point, clamped to a Coulomb box. Restitution applies only when the closing speed is above 1 m/s. Position correction moves the centres and does not change velocity. A dynamic body with no spring and no motor sleeps after 0.5 s under 0.12 m/s and 0.25 rad/s, even if a neighbour is still awake. Bodies that were in contact share an island id. `wake` and `set_position` wake that island. A contact faster than 0.25 m/s or 0.4 rad/s wakes the sleeper it touches and leaves the rest of the island asleep, so a settled pile is not thawed by one restless cube. Holding every contact awake until the fastest body is calm matches sleep disabled. A frictionless yawed hull pile then keeps the overlap's normal impulse as horizontal speed and is about 3.6 m/s at 4 s. The resting-pile scene sets friction to 0.6, and that run stays under 0.5 m/s with sleep off. `remove` does not. Asleep bodies are not integrated and are not solved. Statics live in one AABB tree and dynamics in another. `insert_many` builds those trees in one pass. Object layer 0 is static, 1 is dynamic, 2 is debris, and debris does not meet debris. Rotational energy is `½ ωᵀ I ω` with `I` diagonal in the body frame. A box uses `Ixx = m/3 (hy² + hz²)` on the half extents. A sphere uses `2/5 m r²`. A mesh uses the volume integral of its convex pieces.

`Shape::from_triangles` builds a mesh collider before the step. A convex mesh becomes one hull of at most 12 vertices. A concave mesh is split into at most 6 boxes, so a hole in the source stays open. Contact tests those pieces. The pieces stay on the occupied volume.

## Code

The crate is `crates/physics`, package `genos-physics`.

- `Body`, `World`, `Spring`, `Shape`, `MotionQuality`, `Presence`, and `step` are in `src/lib.rs`.
- A vehicle is in `src/vehicle.rs`: `Vehicle`, `GroundContact`, `Wheel`, `WheeledGround`. `Aircraft` and `SpaceCraft` are stored and are not forced. `chassis_rest_world` builds the G1 fixture.
- Contact is in `src/contact.rs`. The pair list comes from `src/broadphase.rs`. Debris-debris pairs are dropped by the layer table.
- Islands and sleep are in `src/island.rs`.
- Speculative contacts, the GJK gap, and swept time of impact are in `src/ccd.rs`.
- Hot-swap, `apply_damage`, debris sleep, and the debris cull are in `src/destruction.rs`.
- The cook is `crates/fracture`, package `genos-fracture`. `cook`, `Asset`, and `CookRequest` are re-exported from this crate. See [fracture.md](fracture.md).
- Mesh build is in `src/mesh.rs`.
- `GRAVITY` is `(0, -9.81, 0)`.
- `World.bodies` is a `Vec`. The crate does not cap the body count.
- `MAX_HULL_VERTS` is 12. `MAX_MESH_PIECES` is 6.

## Game use

Create a world with `World::new(GRAVITY)`. Insert each body with `World::insert`. Call `step(&mut world, dt)`. The world is updated in place. Read `position`, `orientation`, and `velocity` on `bodies[..count]`.

For a character, set `motor` and write the desired horizontal velocity in `wish`. Use `Body::capsule`, `Body::cuboid`, or `Body::mesh`. Build a mesh with `Shape::from_triangles` or `Body::mesh`. `contact_faces` reports the reduced face count.

For a bullet whose gap at the start of the step is inside the margin, set `motion` to `MotionQuality::Speculative`. `World.speculative_margin` is the cap, 0.25 m unless a caller changes it. For a ship, set `motion` to `MotionQuality::Swept`. `World.swept_pair_cap` is the pair cap, 8 unless a caller changes it. `step_stats.ccd_applied` is true when any body is speculative or swept. `step_stats.toi_failed`, `toi_deferred`, `pair_slices`, and `dynamic_pair_impulses` are the counters for that step.

For a wheeled chassis, insert one dynamic body and push a `Vehicle` whose `chassis` is that index. Set `ground` to `GroundContact::Wheeled` with four `Wheel` rays. `direction` is in the chassis frame. `step` casts each ray against a plane or a box, excluding the chassis, and pushes a spring-damper along the ray. It does not set `Body.spring` or `Body.motor`. `step_stats.wheel_rays` is the cast count. A sleeping chassis casts 0. `chassis_rest_world` is the 1 200 kg fixture. Aircraft and space payloads can be stored. This step does not apply them.

For a breakable, cook the asset, then `spawn_breakable(&mut world, asset, debris_budget)`. `apply_damage(&mut world, breakable, bond, amount)` subtracts health and swaps when the bond reaches 0. `destruction_report` returns `bonds_broken`, `debris_alive`, `debris_asleep`, and `floor_penetration_m`. `allows_slice` stays false. `intact_body` and `chunk_body` are the body indices.

After `step`, `change_records` holds this tick's chunk records in `(breakable, chunk)` order: spawned, moved, slept, or culled. Empty bounds are the zero vector on both corners. `emit_change_records` defaults to true. A headless server sets it false and the list stays empty. `CHANGE_RECORDS_AWAIT_LIGHTING` is true. Lighting does not read the records. `gi_cache_dirty` stays false, so a break does not mark a global stillness hash.

## Limits

There is no island solver and no air drag. G1 is the wheeled ground payload from [ADR 0040](../adr/0040-vehicle-domains.md): four rays and an explicit spring-damper on one chassis. The ADR stays proposed. Aircraft forces, space forces, the landing stage, tyre slip, gearbox, steering, and sea buoyancy are not in this step. A mesh or a hull is a miss for a wheel ray. A miss stores `max_length` and applies no force. An empty vehicle list does no vehicle work and does not allocate. A large `dt` drifts from `½gt²`. A sleeping body is not integrated. A hull's inertia is the volume integral of its tetrahedra about the local origin. Position correction does not rotate.

`Discrete` still tunnels when the travel is longer than the body. `Speculative` does not cover a gap larger than the margin, so a ship at 3 000 m/s has to be `Swept`. With protection on, the metrics fixture marks the bullet speculative and both ships swept, and `tunnel_count` is 0. With protection off, the same speeds still tunnel and store no ship contact. A pair that hits the 16-advance cap counts in `toi_failed` and in `tunnel_count`. A pair past the cap counts in `toi_deferred` and can still tunnel. The knobs are `World.speculative_margin` and `World.swept_pair_cap`. The probe is `cargo test -p genos-physics --test metrics -- --fixture good` and `--fixture bad`. `ccd_in_step` is true when a body asked for speculative or swept collision. `--fixture spin` is one step of a long thin box, half extents `(0.02, 1.5, 0.02)` m, spinning a quarter turn about Z beside a block. `tunnel_count` is 1 when that block is still at rest. The closing bound's `|ω| r` term is what strikes it. `--fixture wide` is the same swept ships with centres 40 m apart, one step and no gravity. The 36 m gap needs more than one radius slice, so a time-of-impact cap of 1 counts in `tunnel_count`. `--fixture rotor` flies a 3 kg cube, half extents 1 m, at 2 rad/s about Z with no gravity. `rotational_energy_j` is `1/2 Izz ωz²` from that size and mass, 4 J.

A mesh collider is a few convex pieces, not the source triangles. Two concave meshes do not get a triangle-versus-triangle test. The piece count stops at 6. A very thin sheet can be missed by the voxel sample.

The camera calls this step from the scene crate. Lights have no collider.

`debug/physics` (`genos-physics`) is the debug binary: headless, stats only, with the ship lanes, hull edge contacts, resting pile, 10k static grid, and `chassis_rest` scenes, per-step JSON stats, bitwise replay, checkpoint restore, and the `physics_*` MCP commands. See [debug/physics/README.md](../../debug/physics/README.md). The destruction metrics probe is `cargo test -p genos-physics --test metrics`. `--fixture broken` is the overhang: seam broken at step 30, 90 steps, `debris_budget` 8. `--fixture budget` is that overhang with 12 children per chunk and `debris_budget` 1. After it settles, `debris_alive` is at most 1 and `debris_asleep` is at least 1. The ship fixtures do not print the destruction rows.

| Failure | Tunable | Probe |
|---|---|---|
| Spin bound drops the angular radius term | closing bound | `--fixture spin` `tunnel_count` |
| Time-of-impact cap is 1 | 16 advances | `--fixture wide` `tunnel_count` |
| Rotational energy drops out | box mass and half extents | `--fixture rotor` `rotational_energy_j` |
| Bond stays whole | `health_per_area`, the `apply_damage` amount | `bonds_broken` on `--fixture broken`. `an_undamaged_breakable_stays_one_body` stays at 0. |
| Supported branch spawns | support flags, the world bond | `apply_damage_spawns_only_the_unsupported_children` |
| Debris falls through the floor | contact, the debris pair filter | `floor_penetration_m` `<=0` |
| Debris ignores sleep or the budget | 0.1 s, 0.4 m/s, `debris_budget` | `--fixture budget` `debris_alive` `<=1` and `debris_asleep` `>=1`. `debris_sleeps_and_the_budget_keeps_the_largest` keeps the largest of 12. |
| `step` slices | `allow_slice` | `allows_slice` is false |
| A cull moves later bodies | tombstone in place | culled presence is `Culled`; the index stays |
| A break dirties the whole GI cache | `emit_change_records` | `a_break_emits_local_bounds_and_does_not_mark_gi_dirty`. The server probe is `the_server_emits_no_change_records`. |
| A filtered layer still solves | `World::set_layer_pair`, object layer | `pair_tests` on `debug/physics --scene layer_filter` and `a_filtered_pair_never_reaches_the_contact_count` |
| A static enters the dynamic tree | broadphase layer | `in_dynamic_tree` on `static_grid_10k`; `a_dynamic_sphere_among_ten_thousand_statics_tests_a_handful_of_pairs` |
| Batch insert diverges | `insert_many`, `refit` | `ten_thousand_batch_matches_one_by_one_positions` |
| One body in a pile wakes alone | `sleep_linear` 0.12, `sleep_angular` 0.25, `sleep_time` 0.5 | `solver_bodies`, `sleeping` on `debug/physics --scene sleep_island` and `two_piles_sleep_on_their_own_and_a_touch_wakes_one` |
| A box past an edge does not tip | contact point, inverse inertia | `a_box_past_the_edge_tips_and_one_over_the_ledge_stays` |
| Spin on a floor does not slow | tangent rows, `friction` | `a_spinning_box_on_the_floor_slows_down` |
| A hull pile jitters or never sleeps | 0.5 s, 0.12 m/s, 0.25 rad/s | `a_hull_pile_settles_and_sleeps_without_jitter`. `sleeping_bodies` on the debug step line |
| Mechanical energy grows | implicit gyroscopic step, split position correction | `mechanical_energy_does_not_grow_over_six_hundred_steps` |
| A resting stack walks off | four narrow-phase passes, centre projection | `a_resting_stack_stays_put` |
| The chassis rests at the wrong height | `hz`, wheel rest length, mass | `a_chassis_rests_on_four_springs`, `debug/physics --scene chassis_rest` |
| A parked chassis keeps casting | `sleep_time` 0.5 s | `a_parked_chassis_sleeps_and_stops_casting`, `wheel_rays` |
| A wheel ray is treated as swept collision | `MotionQuality::Discrete` | `ccd_applied`, `toi_failed`, `pair_slices` on the G1 fixture |
| A miss or a mesh still pushes | `max_length`, shape | `a_ray_that_misses_matches_free_fall`, `a_mesh_under_the_wheels_is_a_miss` |
| Two runs diverge | wheel index order | the second world in `a_chassis_rests_on_four_springs` |

## Plan

The next pipeline is [physics/plan.md](physics/plan.md). That file is a plan. G1 is the wheeled trial in this step. Later vehicle boxes in that plan still wait. [ADR 0040](../adr/0040-vehicle-domains.md) stays proposed.

Continuous collision and destruction are decided in [physics-ccd-destruction-design.md](physics-ccd-destruction-design.md) and [ADR 0016](../adr/0016-physics-ccd-destruction.md). That decision is accepted. Speculative contacts, swept time of impact, and the fracture hot-swap are in this step. `slice_chunk` is not.

## Networking

The step does not replicate. Gameplay networking waits ([Architecture](../architecture.md)). The contract the pipeline will meet:

- Replicated state: the sleep bit on a dynamic body, with the quantized pose when the body is awake. An asleep body with an unchanged sleep bit sends nothing. Island ids and tree nodes stay local. The `.gfract` content hash when the client has the file, otherwise the seeds, child counts, interior material, support flags, and spawn pose. Then `BondBreak { breakable, bond, amount_milli, tick }` in tick order. Awake chunk poses use the same quantized body record as any dynamic body. A slice, when allowed, replicates the plane. Asleep debris and the generated vertices stay local. Gravity sources (`GM`, clamp, influence, attachment) replicate only when that world has opted into the field. A uniform-gravity world sends the static vector and no source block. Scene-script gravity volumes stay local, like walls.
- Authority: the server runs `step` and is the only side that decides a bond broke. Clients apply the server's `BondBreak` through `apply_damage`. They do not accept one another's breaks. The server owns body state. The owning client sends input for the view body.
- Prediction / rollback: none for a breakable or a chunk. The owned view body stays the only predicted body. A client that sees a chunk pose before it has spawned that chunk applies `break_baseline` first.
- Determinism: one process, `step` on one thread, fixed `1/60` s, the same `.gfract` bytes, seed order, and bond order. `step_parallel` may run on the server only after it hash-matches `step`. The wire compares quantized poses and bond ids. Cross-platform bit identity is not required.
- Headless server: `step` and the sim subset. No window, no Vulkan device. The server sets `emit_change_records` false and builds no change records. The client builds them after the step. Lighting does not consume them yet.

Vehicle contract, copied from ADR 0040. The codec is not in this step. Gameplay networking still waits.

- Replicated state: the chassis uses the same quantized pose and velocity as any dynamic body. G1 adds each wheel's suspension length as `i16`. A1 adds throttle as `i16`. S1 adds each thruster throttle as `i16`. Contact scratch, slip, the lift sample, and the thruster force stay local. Sea is absent, so draft and drag are not on the wire.
- Authority: the server runs `step` and owns the chassis pose. The owning client sends input. Clients do not accept one another's vehicle state.
- Prediction / rollback: none. A driven chassis is interpolated. Predicting it would roll back contacts with the world.
- Determinism: one process, `step` on one thread, fixed `1/60` s, aircraft then space then ground. Two runs match bits. The wire compares quantized values. Cross-platform bit identity is not required.
- Headless server: the same `step`, no window and no Vulkan device. `debug/physics` is already that path.

## Decisions

- [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md) keeps this step in the repository. There is no physics middleware crate.
- [ADR 0033](../adr/0033-physics-scale-broadphase-and-sleep.md) is the layered broadphase and island sleep. Departures from the AABB-tree paper and from Box2D's rotations are in that ADR.
- [ADR 0034](../adr/0034-rotational-contact-solver.md) is the rotational contact solver. Joints, rolling friction, and a full island solver stay out of that decision. It does not define a vehicle.
- [ADR 0040](../adr/0040-vehicle-domains.md) is the vehicle proposal: ground, aircraft, and space payloads on one chassis body. Status is proposed. G1, the wheeled ground payload, is in this step as a trial. Aircraft forces, space forces, and sea buoyancy are not.
- [ADR 0016](../adr/0016-physics-ccd-destruction.md) is the continuous-collision and destruction decision. The departures from that design's papers are named in [the design](physics-ccd-destruction-design.md). This step adds these: supported chunks stay in the intact static body, culled debris are tombstoned, the intact contact impulse hits the largest seam, and a world bond's health is `health_per_area * 10`.
