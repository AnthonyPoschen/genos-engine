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

A capsule is a segment along local Y plus a radius. `half_height` is the segment half-length. A box uses `half_extents` on the local axes. `orientation` aims the capsule segment and the box. The step turns `orientation` only when `angular_velocity` is non-zero: `dq/dt = ½ ω q`. Contact does not write a torque, so an impact leaves `angular_velocity` unchanged. Rotational energy is `½ ωᵀ I ω` with `I` diagonal in the body frame. A box uses `Ixx = m/3 (hy² + hz²)` on the half extents. A sphere uses `2/5 m r²`. A mesh uses the bounding-sphere moment.

`Shape::from_triangles` builds a mesh collider before the step. A convex mesh becomes one hull of at most 12 vertices. A concave mesh is split into at most 6 boxes, so a hole in the source stays open. Contact tests those pieces. The pieces stay on the occupied volume.

## Code

The crate is `crates/physics`, package `genos-physics`.

- `Body`, `World`, `Spring`, `Shape`, `MotionQuality`, `Presence`, and `step` are in `src/lib.rs`.
- Contact is in `src/contact.rs`. Debris-debris pairs are dropped there.
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

For a breakable, cook the asset, then `spawn_breakable(&mut world, asset, debris_budget)`. `apply_damage(&mut world, breakable, bond, amount)` subtracts health and swaps when the bond reaches 0. `destruction_report` returns `bonds_broken`, `debris_alive`, and `floor_penetration_m`. `allows_slice` stays false. `intact_body` and `chunk_body` are the body indices.

After `step`, `change_records` holds this tick's chunk records in `(breakable, chunk)` order: spawned, moved, slept, or culled. Empty bounds are the zero vector on both corners. `emit_change_records` defaults to true. A headless server sets it false and the list stays empty. `CHANGE_RECORDS_AWAIT_LIGHTING` is true. Lighting does not read the records. `gi_cache_dirty` stays false, so a break does not mark a global stillness hash.

## Limits

There is no stack solver and no air drag. A large `dt` drifts from `½gt²`. Contact does not change angular velocity.

`Discrete` still tunnels when the travel is longer than the body. `Speculative` does not cover a gap larger than the margin, so a ship at 3 000 m/s has to be `Swept`. With protection on, the metrics fixture marks the bullet speculative and both ships swept, and `tunnel_count` is 0. With protection off, the same speeds still tunnel and store no ship contact. A pair that hits the 16-advance cap counts in `toi_failed` and in `tunnel_count`. A pair past the cap counts in `toi_deferred` and can still tunnel. The knobs are `World.speculative_margin` and `World.swept_pair_cap`. The probe is `cargo test -p genos-physics --test metrics -- --fixture good` and `--fixture bad`. `ccd_in_step` is true when a body asked for speculative or swept collision. `--fixture spin` is one step of a long thin box, half extents `(0.02, 1.5, 0.02)` m, spinning a quarter turn about Z beside a block. `tunnel_count` is 1 when that block is still at rest. The closing bound's `|ω| r` term is what strikes it. `--fixture wide` is the same swept ships with centres 40 m apart, one step and no gravity. The 36 m gap needs more than one radius slice, so a time-of-impact cap of 1 counts in `tunnel_count`. `--fixture rotor` flies a 3 kg cube, half extents 1 m, at 2 rad/s about Z with no gravity. `rotational_energy_j` is `1/2 Izz ωz²` from that size and mass, 4 J.

A mesh collider is a few convex pieces, not the source triangles. Two concave meshes do not get a triangle-versus-triangle test. The piece count stops at 6. A very thin sheet can be missed by the voxel sample.

The camera calls this step from the scene crate. Lights have no collider.

`debug/physics` is not in this change. The headless probe is `cargo test -p genos-physics --test metrics`. `--fixture broken` is the overhang: seam broken at step 30, 90 steps, `debris_budget` 8. The ship fixtures do not print the destruction rows.

| Failure | Tunable | Probe |
|---|---|---|
| Spin bound drops the angular radius term | closing bound | `--fixture spin` `tunnel_count` |
| Time-of-impact cap is 1 | 16 advances | `--fixture wide` `tunnel_count` |
| Rotational energy drops out | box mass and half extents | `--fixture rotor` `rotational_energy_j` |
| Bond stays whole | `health_per_area`, the `apply_damage` amount | `bonds_broken` on `--fixture broken`. `an_undamaged_breakable_stays_one_body` stays at 0. |
| Supported branch spawns | support flags, the world bond | `apply_damage_spawns_only_the_unsupported_children` |
| Debris falls through the floor | contact, the debris pair filter | `floor_penetration_m` `<=0` |
| Debris ignores sleep or the budget | 0.1 s, 0.4 m/s, `debris_budget` | `debris_sleeps_and_the_budget_keeps_the_largest` |
| `step` slices | `allow_slice` | `allows_slice` is false |
| A cull moves later bodies | tombstone in place | culled presence is `Culled`; the index stays |
| A break dirties the whole GI cache | `emit_change_records` | `a_break_emits_local_bounds_and_does_not_mark_gi_dirty`. The server probe is `the_server_emits_no_change_records`. |

## Plan

The next pipeline is [physics/plan.md](physics/plan.md). That file is a plan. It does not change this step.

Continuous collision and destruction are decided in [physics-ccd-destruction-design.md](physics-ccd-destruction-design.md) and [ADR 0016](../adr/0016-physics-ccd-destruction.md). That decision is accepted. Speculative contacts, swept time of impact, and the fracture hot-swap are in this step. `slice_chunk` is not.

## Networking

The step does not replicate. Gameplay networking waits ([Architecture](../architecture.md)). The contract the pipeline will meet:

- Replicated state: the `.gfract` content hash when the client has the file, otherwise the seeds, child counts, interior material, support flags, and spawn pose. Then `BondBreak { breakable, bond, amount_milli, tick }` in tick order. Awake chunk poses use the same quantized body record as any dynamic body. A slice, when allowed, replicates the plane. Asleep debris and the generated vertices stay local. Gravity sources (`GM`, clamp, influence, attachment) replicate only when that world has opted into the field. A uniform-gravity world sends the static vector and no source block. Scene-script gravity volumes stay local, like walls.
- Authority: the server runs `step` and is the only side that decides a bond broke. Clients apply the server's `BondBreak` through `apply_damage`. They do not accept one another's breaks. The server owns body state. The owning client sends input for the view body.
- Prediction / rollback: none for a breakable or a chunk. The owned view body stays the only predicted body. A client that sees a chunk pose before it has spawned that chunk applies `break_baseline` first.
- Determinism: one process, `step` on one thread, fixed `1/60` s, the same `.gfract` bytes, seed order, and bond order. `step_parallel` may run on the server only after it hash-matches `step`. The wire compares quantized poses and bond ids. Cross-platform bit identity is not required.
- Headless server: `step` and the sim subset. No window, no Vulkan device. The server sets `emit_change_records` false and builds no change records. The client builds them after the step. Lighting does not consume them yet.

## Decisions

- [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md) keeps this step in the repository. There is no physics middleware crate.
- [ADR 0016](../adr/0016-physics-ccd-destruction.md) is the continuous-collision and destruction decision. The departures from that design's papers are named in [the design](physics-ccd-destruction-design.md). This step adds these: supported chunks stay in the intact static body, culled debris are tombstoned, the intact contact impulse hits the largest seam, and a world bond's health is `health_per_area * 10`.
