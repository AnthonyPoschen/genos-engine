# Physics pipeline implementation plan

> **For agentic workers:** Implement one step per commit, in order. Do not start the next step in the same commit. Each step has a deterministic headless test. Check the box when that test passes.

**Goal:** Replace the single discrete contact pass with a physics pipeline that can run a large world, a person, a vehicle, a fast ship, a pre-fractured breakable, and a space scene with moving gravity wells, with work gated by what is awake, and with a server-authoritative replication contract for each of those systems.

**Architecture:** Rigid bodies stay on the CPU. A layered AABB tree feeds GJK/EPA manifolds into a soft-step solver. Islands sleep. Characters, vehicles, CCD, and destruction are opt-in on that pipeline. Poses of awake bodies are the only per-frame upload. A GPU solver waits on a profile. Offline Voronoi fracture builds the chunk asset; the step only swaps and simulates the broken branch. The server owns body state. The owning client predicts the view body and rolls that body back. Vehicles and destruction replicate as snapshots and events. Each of those systems has a debug binary with the lighting command, script, screenshot, and metrics path.

**Tech stack:** Rust, `genos-math` (`f32` `Vec3` / `Quat`), `std::thread::scope`, Vulkan only for the debug picture and the awake-pose buffer. ADR 0007 forbids a physics middleware crate.

Written against `gi-v2` parent `e61b89d`. This file does not change `step`.

## Global constraints

- ADR 0007. No Rapier, Parry, nalgebra, Jolt, PhysX, Blast, or Chaos crate. Those engines are the design source. The code stays in this repository.
- ADR 0009. Rhai stays the game script. The fracture tool and the headless tests are Rust.
- `genos-math` stays `f32`. Wide positions live in the physics crate. [Math](../math.md) says there is no `f64` path in that crate.
- Y is up for the room proof. Each world picks a gravity mode. The default is `GravityMode::Uniform`: the static vector on `World.gravity`, which `World::new(GRAVITY)` sets to `(0, -9.81, 0)`. That path does not call the wells code. Lengths are metres, mass is kilograms, time is seconds.
- The frame interval is the measured `dt`. The camera already caps one frame at 0.25 s (`crates/scene/src/camera.rs`). Do not add a minimum step count, a minimum active set, or a minimum GPU dispatch. An asleep world does no narrowphase and no solver work.
- The physics schedule inside an accumulated frame is a fixed `1/60` s step, matching `update` in `crates/scene/src/camera.rs`. Sub-steps divide that step. They do not rerun the broadphase.
- The character controller is the action map in `crates/input/src/action.rs` (`character_controller`). It is not a body. The simulated capsule is a body. Do not rename one to the other. `CONTEXT.md` defines both.
- Codimation stays the looping solid path in `crates/scene`. It is not a skeletal clip.
- New public words (`BodyId`, island, bond, fracture asset, foot plant, vehicle) are added to `CONTEXT.md` in the commit that first publishes them.
- A new crate sets `[lints] workspace = true`, joins the workspace `Cargo.toml`, and is named in `docs/architecture.md` in that same commit.
- Every step keeps `cargo test -p genos-physics` and `cargo test -p genos-scene` green, plus the tests of any other crate that step touches. `cargo build --workspace` passes. The workspace denies warnings.
- Headless tests use a fixed `dt`, a fixed call order, and no window. Two runs in one process match bitwise on the values the step names. Do not check thread scheduling or wall-clock time.
- `step(&mut World, dt)` stays the call the scene crate uses. New behavior has defaults that keep the existing assertions in `crates/physics/tests/scenarios.rs`, `crates/physics/tests/colliders.rs`, and `crates/physics/tests/bench.rs`.
- Gameplay networking still waits ([Architecture](../../architecture.md)). The session, the socket, interest, and the snapshot codec belong to [core-networking/plan.md](../core-networking/plan.md). Physics does not open a socket. If a name in that file differs from this one, that file wins.
- The recorded server tick and the rollback resim call `step`, the one-thread color order. `step_parallel` may run on a server only after it hash-matches `step`.
- Every new system in this plan states replicated state, authority, prediction or rollback, determinism, and headless-server behavior in its system record in the commit that first implements them.

## Current state

The crate is `crates/physics`, package `genos-physics`. The living record is [physics.md](../physics.md). The scene owns the only caller (`crates/scene/src/camera.rs`).

### What the step does

`step` (`crates/physics/src/lib.rs`, `step`) takes a positive `dt` and then:

1. `apply_forces` adds gravity and, while a spring is compressed, a push. A spring does not pull. Rest compression is `mass * -gravity.y / stiffness`.
2. If `motor` is set, `wish.x` and `wish.z` replace the horizontal velocity.
3. Dynamic bodies (`inverse_mass != 0`) integrate position with `velocity * dt`. Orientation is not integrated.
4. `contact::collide` runs four times.

`World` stores `bodies: Vec<Body>` and a reused `contact::Pairs` scratch list. The first `step` can allocate that list. `timed_math_and_physics_batch_does_not_allocate` (`crates/physics/tests/bench.rs`) asserts that a later step of 32 spheres allocates nothing.

`Body` (`lib.rs`) is `Copy`: `position`, `orientation`, `velocity`, `inverse_mass`, `restitution`, `friction`, `shape`, `spring`, `motor`, `wish`. There is no angular velocity, torque, inertia tensor, or body id. `World::insert` returns a `usize` index.

Shapes (`lib.rs`, `Shape`): sphere, capsule (segment along local Y plus radius), oriented box, plane (`normal · point = offset`), and a mesh. A convex triangle mesh becomes one hull of at most `MAX_HULL_VERTS` (12) vertices and `MAX_HULL_FACES` (20) triangles (`mesh.rs`, `hull_from_points`, `fill_faces`). A concave mesh is voxelized into at most `MAX_MESH_PIECES` (6) boxes (`mesh.rs`, `voxel_boxes`). `order_around` sorts face vertices with `atan2`.

### Broadphase

There is no tree. `contact::collide` (`contact.rs`) builds an AABB per body, expands it by `BOUNDS_MARGIN` (0.02), sorts by minimum X with `total_cmp`, and sweeps. Planes have no bounds and pair with every bounded body. Two static bodies are skipped. Pairs are sorted with `sort_unstable` and solved in that order. `a_ball_lands_on_its_own_pillar_among_hundreds` builds 400 static pillars plus one sphere (401 bodies) and the sphere rests on its own pillar.

### Narrowphase and solver

Primitives use closest points (`primitive_contact`). A mesh tests each convex piece and keeps the deepest hit only (`deeper`). Hull versus hull tests one hull's vertices against the other (`hull_hull`); it is not a full convex test. `solve_contact` applies one normal impulse, a friction impulse clamped to `friction * normal_impulse` (friction and restitution are the larger of the two bodies), and a position correction of `PERCENT` (0.8) past `SLOP` (0.001). No manifold, no warm start, no joints, no stacking solver. The record's limit still says there is no stack solver and no continuous collision.

### Scene, view body, animation

`Camera::attach_scene` inserts the floor, every wall, every solid, and one view capsule (`camera.rs`). The capsule is mass 1, radius 0.3, half-height 0.55, with `motor` set. `update` writes a yaw-relative wish at 4 m/s and runs `step` at `1/60` s when the accumulator reaches that step. Codimation writes a wish on a solid and writes the resolved position back.

The character controller (`crates/input/src/action.rs`, `character_controller`) binds `move` and `look`. It does not create a body.

`genos-load` animation (`crates/load/src/animation.rs`) is one transform track (translation, quaternion, scale). [Loading](../loading.md) says glTF skins, morph targets, and glTF animations are ignored, and the text animation does not move a mesh. There is no skeleton, so nothing can plant a foot.

`genos-math` components are `f32` (`crates/math/src/lib.rs`). The camera far plane is 200 m (`CAMERA_FAR` in `camera.rs`).

`genos-mesh` has a triangle BVH (`crates/mesh/src/bvh.rs`) for rays and baking. The physics step does not use it.

`genos-bake` builds GI SDFs and cards. It is not a fracture tool. [Bake](../bake.md) keeps that crate on mesh content for lighting.

### Debug binaries

`examples/camera` (`genos-camera`) and `examples/stress` (`genos-stress`) are the programs. Stress flies with `update(..., 0.0)` so the step does not run (`examples/stress/src/main.rs`, `fly`). Pictures go through `genos_debug::png` and the MCP `take_screenshot` path. There is no shared scene catalog and no physics debug binary.

### Records corrected with this plan

`docs/systems/physics.md`, `docs/systems/scene.md`, and `examples/stress/README.md` used to say the world held `MAX_BODIES` (32) and dropped the rest. The crate has no such cap. Those sentences now match the code. The stress camera still flies because of the zero interval, not because of a cap.

## Gaps

| Pipeline piece | In the tree today | Missing |
| --- | --- | --- |
| Body model | Dynamic if `inverse_mass > 0`, else static. Index handles. Inline shapes. | Kinematic motion, `BodyId`, angular motion, shared shape store, convex margin, static triangle mesh that is not the 6-box reducer. |
| Layers | Every overlapping pair is tested. | Object layers, a static / dynamic / debris broadphase split, pair filters. |
| Broadphase | One X-sweep over current AABBs. | Layered AABB tree, batch insert, dirty expand, background refit, swept AABBs. |
| Narrowphase | Analytic closest points, deepest hit, one-sided hull test. | GJK, EPA, clipped manifolds, contact cache. |
| Solver | Four passes of one impulse plus position correction. One push spring. | Soft constraints (Hz, damping), sub-steps, warm start, joints, a sequential-impulse fallback that stays available. |
| Sleep and islands | Every dynamic body is integrated. | Islands, island sleep, whole-island wake, solver skipped for the asleep set. |
| Multithreading | One thread. | Color the constraint graph. Parallel color passes and a sorted parallel narrowphase. Same hash as the one-thread color order. |
| CCD | Discrete only. A large `dt` tunnels. | Swept AABBs, speculative contacts, conservative-advancement time of impact for two moving convexes, extra sub-steps on a fast pair. |
| Determinism | X-sort uses `total_cmp`. Hull faces use `atan2`. | A state hash, fixed body and contact order, in-process replay. |
| Character | Wish overwrites horizontal velocity. Walls cancel the blocked part. | Wall slide, stairs, ground stick, moving platforms, as a kinematic body the action map drives. |
| Foot plant | No skeleton. | Ray or shape cast, two-bone IK, pelvis offset, slope alignment, wired to the view body and the action map. |
| Vehicles | The push spring is the only suspension-like force. | Ray or shape-cast wheels, spring-damper, slip friction, torque curve, gearbox, differential, anti-roll bar. Separate 6-DOF thrusters, zero gravity, large-orbit precision. |
| Scale | Hundreds of static pillars in one test. `f32` world positions. | Thousands of active bodies, origin shift, awake-only GPU poses, a CPU solver that does not touch the asleep set. |
| Gravity | One uniform vector on the world. | Per-body field from moving sources, inverse-square with a clamp and a sphere of influence, volume overrides, local up, wide-orbit integration, source replication. |
| Destruction | A mesh collider is a few convex pieces. | Offline Voronoi chunks, interior faces, hierarchy, bond graph, hot-swap, one damage API, debris sleep and cull. Runtime slicing only when asked. |
| Networking | No gameplay net. The loopback MCP endpoint edits the live scene. [Architecture](../../architecture.md) says gameplay networking waits. | Server authority, owned-body prediction and rollback, quantized replication, vehicle snapshots, fracture asset id plus bond-break events. |
| Debug | `genos-stress` is the lighting binary: one command set over MCP, HTTP, and Rhai; scripts; knobs; screenshots; `report.md`. | The same shape for rigid bodies, the view body, vehicles, and fracture, plus a shared scene catalog and a reset. |

## Principles

- Performance first. Wheels are casts, not bodies. An intact breakable is one body. Debris-debris pairs are filtered. A large island is colored so one pile can use every core.
- Work is gated by need. Asleep islands are absent from narrowphase and the solver. Intact chunks are not simulated. A sleeping debris chunk is a cull candidate. CCD runs on bodies marked swept or speculative, not on the whole world.
- GPU-resident where it pays. Awake poses and spawned chunk meshes live in a buffer the renderer can read. The CPU writes a delta when the awake set changes. The solver stays on the CPU until the gate at the end of this plan.
- No fixed per-frame floor. Variable frame `dt`, fixed physics step only when time has accumulated, zero solver iterations when nothing is awake.
- The server tick is that same `1/60` s step. A client renders between snapshots. It does not invent a second physics rate.
- Gravity mode is per world. `Uniform` is the default and is the only gravity work a simple game does: add `World.gravity` in the force loop. `Field` is opt-in. A `Uniform` step does not allocate a source list, query a tree, sample a wide position, or enter `gravity.rs`.

## Networking

[core-networking/plan.md](../core-networking/plan.md) owns the session, the UDP socket, `NetId`, `InputFrame`, the snapshot ring, interest, bandwidth, and the session debug binary (one server world and N client worlds in one process, with latency, loss, and jitter). This section is the physics contract that file calls. Physics stores `NetId` on a body and reads `InputFrame`. It does not define the codec and it does not resend packets.

The co-design this section matches: the server is the only state authority; the sim tick is `1/60` s; snapshots go out at 20 Hz; the owning client predicts only its view body; everyone else is interpolated; same-process determinism is required; cross-platform bit identity of `f32` physics is not.

### Authority

The headless server runs `step` with no window, no Vulkan device, and no audio device. Clients do not accept one another's body state. An owning client has input authority for one view body, or for one vehicle it drives: it sends `InputFrame` samples. The server applies those samples and the resulting pose is the authority.

A client does not predict a push against a dynamic body. On the predicting client that body stays on the latest snapshot. The server is the first place the push exists.

### What replicates

Replication is dirty tracking on the P25 pose buffer. An asleep body with an unchanged sleep bit sends nothing. Static geometry comes from the scene script on each peer, the same way the camera scene loads `scene.rhai`. Shape bytes, manifolds, warm-start impulses, island ids, and broadphase nodes stay local.

| Piece | On the wire | Stays local |
| --- | --- | --- |
| Dynamic or kinematic body | `NetId`, millimetre position relative to `World::origin` (`i32`), smallest-three quaternion (`3 × i16`), linear velocity in cm/s (`i16`), angular velocity in milliradians/s (`i16`), sleep bit, `Motion` | `f32` pose, contact cache, inertia |
| Origin | The `f64` origin, sent on the step that shifts it | The shifted `f32` working set |
| View body input | `InputFrame` for that owner (tick, actions, sequence), defined by the core-networking plan | The wish after it is decoded |
| Foot plant | Nothing | Casts, two-bone solve, pelvis lift. It runs on the rendered pose |
| Vehicle | Chassis quantized pose and velocity, engine rpm (`u16`), gear (`i8`), steer (`i16`), each wheel's omega and suspension length (`i16`) | Suspension contact cache, tyre slip scratch, the torque table (content) |
| Vehicle input | Throttle, brake, steer, gear request inside the owner's `InputFrame` | The solved wheel forces |
| Destruction spawn | The pre-fracture seeds, child counts, interior material, support flags, and spawn pose. A content hash of the `.gfract` file replaces that payload when the client already has the file | Generated vertices and interior faces. The client rebuilds them from the seeds |
| Destruction break | `BondBreak { breakable, bond, amount_milli, tick }` | The chunk meshes. Clients rebuild them from the asset |
| Slice | `Slice { breakable, chunk, normal_i16, offset_mm, tick }` when `allow_slice` is set | The clip result, rebuilt from the plane |
| Debris | Awake debris poses, under the same interest budget the core-networking plan applies | Asleep debris. Cull stays local |
| Gravity source | `GravitySourceId`, `GM` as `f64` bits, clamp radius, influence radius, attachment `NetId` or a wide position. Clients recompute acceleration from these | Per-body acceleration, the octree, contact gravity scratch |
| Gravity override | A volume attached to a body: kind, uniform vector or spin rate. A volume placed by the scene script is content, like a wall | The sampled field |

Quantized fields are what reconciliation compares. The solver keeps `f32`.

`break_baseline(breakable)` returns the set of broken bond ids and the quantized poses of awake chunks. The core-networking plan sends that when a client has no prior snapshot. Physics does not invent a second reliable channel.

### Prediction and rollback

The owned view body is the only predicted body.

Each client tick samples the character-controller action map once into an `InputFrame` and stores it. The client steps the view body ahead of the server and keeps a ring of 128 predicted poses, keyed by tick. Remote bodies and vehicles are interpolated between the two snapshots that bracket render time (the core-networking plan uses a 100 ms delay). They are not resimulated.

When a snapshot arrives for tick `T`, the client compares the quantized server pose of the owned body with the stored prediction at `T`. On a mismatch it restores the owned body to the server pose at `T`, then replays `InputFrame`s `T+1 … now` through `step` for that body only. Other islands are not rolled back. Rhai does not run inside the replay. Codimation on a remote solid is server state, not a predicted wish.

Foot plant runs after the replay, on the pose that will be drawn. A correction can move the hips. The feet recast. The plant is not part of the replicated checksum.

A driven vehicle in v1 is server-authoritative and interpolated, including wheel spin and suspension length. Predicting a chassis would roll back contacts with the world. That waits until the owned-body test is green, and even then the predicted set is the chassis only, with the same ring and the same rule that other dynamics stay interpolated.

### Determinism

P14's hash is the replay gate. Two worlds in one process, given the same spawns, the same `.gfract` bytes, the same `InputFrame` sequence, and `step` at `1/60`, produce the same hash. That is what makes the owned-body replay match the server.

The server tick uses `step`, not `step_parallel`. Hull face order on the solver path does not call `atan2`. Cross-platform bit identity is not a gate: `genos-math` is `f32`, and this plan does not add a `libm` crate. A Windows server and a Linux client compare quantized poses, not raw `f32` bits.

Body storage order for the hash and for the snapshot is `NetId` order once bodies live on entities. `BodyId` remains the in-world handle. The map is one `NetId` per replicated body, assigned by the server.

### Vehicles and destruction on the wire

A car replicates as one chassis body plus the small wheel block above. Clients do not run the tyre solver. They draw the wheels from the replicated omega and suspension length. A missed snapshot interpolates those scalars the same way it interpolates position.

A breakable replicates as seeds plus events. The spawn message carries the Voronoi seeds, the child counts, the interior material, and the support flags. Both peers run the same fracture build, so the chunk meshes match and the vertices are not sent. When the client already has that `.gfract` file, the spawn sends the content hash instead of the seeds. The server simulates contacts and emits a `BondBreak` when `apply_damage` crosses zero. The client applies that event through the same `apply_damage`, in tick order. Chunk transforms of the spawned branch then replicate as ordinary awake bodies. A client that sees a pose for a chunk it has not spawned yet applies the baseline bond set first, then the pose.

A health event and an impulse event are the same `BondBreak`. The amount is the damage in milli-units. The client does not need to know which caller crossed the threshold.

Slicing replicates the plane, not the new triangles. Both peers run `slice_chunk` from that plane. A peer with `allow_slice: false` ignores the event.

### Observability

The model is [Debugging](../debugging.md): one command set, the same JSON over MCP `tools/call`, `POST /cmd/<name>`, and a Rhai script; `genos-debug` in the frame loop; a script that writes `report.md`, `result.json`, and images; knobs; a reset; a screenshot. `genos-stress` is the lighting binary that already does this. Each physics system gets its own binary with that shape. They share `genos-scenes` and the command names below. They do not mount the lighting reference tracer.

| System | Binary | Default scenes | Knobs | View modes |
| --- | --- | --- | --- | --- |
| Rigid bodies, CCD, islands, gravity | `genos-physics-view` | `stack`, `pile_4k`, `asleep_grid`, `ship_impact`, `gravity_well` | `dt_scale`, `solver` (`soft` or `sequential`), `substeps`, `sleep`, `gravity_scale` | `bodies`, `contacts`, `islands`, `sleep`, `sweeps`, `gravity` |
| View body and foot plant | `genos-character-view` | `view_stairs`, `foot_slope`, `gravity_well` | `step_height`, `stick`, `wish_speed` | `bodies`, `casts`, `feet`, `prediction` |
| Vehicles | `genos-vehicle-view` | `car_flat`, `car_bump`, `orbit`, `ship_impact` | `throttle`, `gear`, `mu`, `anti_roll` | `bodies`, `wheels`, `sweeps`, `gravity` |
| Fracture | `genos-fracture-view` | `destruction_wall`, `ship_station` | `debris_budget`, `bond_health`, `allow_slice` | `bodies`, `bonds`, `chunks` |

Shared commands, same arguments on every binary: `physics_metrics`, `take_screenshot`, `time` (pause, step, fixed step), `knob`, `reset`, `scene`, `view`. `reset` restores the catalog start, including bond health, gear, and the prediction ring. `key_r` calls that command. It is not added to `character_controller()`.

Headless: `binary --headless --scene NAME --frames N --metrics PATH` and `binary --script FILE --report DIR`. The script functions match the lighting shorts that apply here: `step(n)`, `shot(name)`, `knob(name, value)`, `check`, `require`. Exit code 1 when a check fails.

The session binary from the core-networking plan is the multi-client view. Physics publishes `replicated_checksum` (FNV of the quantized replicated fields in `NetId` order) and the prediction error (server quantized position minus the rendered position). That binary draws the error. Physics does not open a second session window.

### Headless tests the contract adds

These tests are in-process. They do not bind a UDP socket. The session binary calls the same functions later.

- `owned_body_reconciles_after_a_dropped_input`
- `a_predicted_capsule_does_not_move_a_dynamic_box`
- `vehicle_bytes_are_the_chassis_and_the_wheels`
- `bond_breaks_replicate_without_the_mesh`
- `replicated_checksum_ignores_foot_plant`

## Gravity

Gravity mode is chosen per world. Two worlds in one process may differ. The default is the cheap path.

| Mode | What a body gets | What `step` does |
| --- | --- | --- |
| `Uniform` (default) | `World.gravity`, a static `Vec3` | The force loop that exists today. `gravity.rs` is not called. |
| `Field` (opt-in) | `World.gravity` plus sources, then a volume may replace that sum | Sphere query, Barnes-Hut only past 32 unbounded sources, wide Verlet for free bodies. |

`World::new(GRAVITY)` is `Uniform`. `free_fall_matches_half_gt_squared_and_gt` stays on that path. `set_gravity_mode(Field)` is the switch. Adding a source or a volume while the mode is `Uniform` leaves the mode unchanged and does not store the source. The caller sets `Field` first. A simple game never calls that setter, so it pays nothing for the wells feature: no source allocation, no tree, no wide-position sample, no extra branch inside the force loop.

`Field` is the mixed setup. Outside volumes the acceleration is the global vector plus the in-range sources. Inside a volume the override replaces both. A zero-g cabin in a world that still has `GRAVITY` is that: `Field`, the global vector, and a zero volume. A space scene sets the global vector to zero and adds wells. Kinematic bodies still ignore gravity in either mode.

### Sources

A source is a planet, a moon, a ship, or a black hole. It has a wide position, or it is attached to a `BodyId` and reads that body's wide position every step. `GM` is m³/s². The acceleration at a wide point is toward the source:

`a = GM / r_eff²`, `r_eff = max(distance, clamp_radius)`.

`clamp_radius` keeps the center finite. A black hole is this same falloff with a small clamp and a large `GM`. There is no event horizon and no relativity. Past `influence` (the sphere of influence) the contribution is zero. Overlapping spheres add, in `GravitySourceId` order, so the sum is deterministic. An override volume replaces that sum inside itself. It does not add to it.

### Overrides

| Kind | Acceleration inside the volume |
| --- | --- |
| Zero | `0`. A cabin, a hollow moon, a zero-g bay. |
| Uniform | A fixed vector in the volume's local frame. Artificial gravity on a ship. |
| Spin | Centrifugal acceleration in the rotating frame: `ω²` times the distance from the axis, pointing away from the axis. The floor is the outer hull. Local up points toward the axis. |

The innermost containing volume wins. Volumes may be attached to a body so a spinning station and a ship's deck move with it.

### Local up

In `Uniform` mode, up is world `+Y` when `World.gravity` points along `-Y`, and `-normalize(World.gravity)` otherwise. The view body does not call `gravity_at`. In `Field` mode, `local_up` is `-normalize(a)` when `|a| > 0.05` m/s². Below that the last up is kept. The character controller stays the action map. The view body turns `move` into a wish on the plane perpendicular to local up: forward is the look direction projected onto that plane. The capsule segment aligns to local up. Foot casts go along `-local_up`. A uniform `GRAVITY` field still has up along `+Y`, so the room camera does not change.

### Orbits

Free bodies (no contact) integrate in `f64` with velocity Verlet, which is the symplectic kick-drift for this field:

`v += a(x) * dt / 2`, `x += v * dt`, `a = field(x)`, `v += a * dt / 2`.

The `f32` pose is that wide position minus `World::origin`. Bodies in contact take one sample of `a` per sub-step and feed it to the existing semi-implicit solver as their gravity vector. P32's private orbit helper becomes this path. Its `1e7` m circular-orbit test stays.

A moving source wakes a sleeper when the sampled acceleration differs from the value stored at sleep by more than `0.05` m/s².

### Many sources

These queries exist only in `Field`. A source with a finite influence radius is stored by that sphere. A body queries the spheres that contain it. Sources outside the query are not evaluated. A `Field` world with nothing in range returns `World.gravity` and builds no tree. A `Uniform` world does not reach this query.

Sources with infinite influence, and only when more than 32 of them are awake, use a Barnes-Hut octree on their `GM`. A cell is walked into when `size / distance > theta`, with `theta = 0.5`. Otherwise the cell's monopole is one source. Finite-influence sources stay on the sphere query and are not folded into the tree. The tree is rebuilt when a source moves. An asleep body outside every finite sphere is not a query.

### Replication

The mode byte is part of the world snapshot. A `Uniform` snapshot has no source block. The client stays on the vector add and does not evaluate `gravity_at`. A `Field` snapshot carries the source parameters in the table above. The client evaluates `gravity_at` from those parameters, from `World.gravity`, and from scene-script volumes, the same way it builds static walls locally. During owned-body rollback the field uses the source positions from the snapshot at that tick. Planets are not resimulated inside the replay. The server does not invent `GM`.

`replicated_checksum` includes the source parameters. Two worlds with the same sources and the same wide positions produce the same `f64` acceleration in one process.

### Scene and the field view

`gravity_well` is one catalog scene with three lanes:

- A circular orbit at `1e4` m around a fixed source, uniform field `0`.
- A slingshot past a kinematic moon.
- A small planet, radius `100` m, surface gravity `9.81` (`GM = 98100`), a view body walking on the side of the sphere, a zero-g box just off the surface, and a spin habitat.

`genos-physics-view` view mode `gravity` draws a grid of arrows on the view plane (length from `gravity_scale` and the log of `|a|`), a wire sphere for each influence radius, and the override volumes. `examples/physics-view/scripts/gravity_well.rhai` selects that view, shots the frame, and checks the metrics. `genos-character-view` loads the same scene for the walk. `genos-vehicle-view` keeps `orbit` and can select `gravity` too.

## Locked names

Later steps use these names. A step that introduces one publishes it and adds the word to `CONTEXT.md` if a game will say it.

```rust
pub struct BodyId { index: u32, generation: u32 }

pub enum Motion { Static, Kinematic, Dynamic }

pub enum MotionQuality { Discrete, Speculative, Swept }

pub struct ObjectLayer(pub u16);
pub struct BroadPhaseLayer(pub u8); // 0 static, 1 dynamic, 2 debris

pub fn step(world: &mut World, dt: f32);
pub fn step_parallel(world: &mut World, dt: f32, workers: usize);

pub struct CastHit {
    pub body: BodyId,
    pub point: Vec3,
    pub normal: Vec3,
    pub fraction: f32,
}

pub struct VehicleId(pub u32);
pub struct WheelId(pub u32);
pub struct ThrusterId(pub u32);

pub struct BreakableId(pub u32);
pub struct BondId(pub u32);
pub fn apply_damage(world: &mut World, breakable: BreakableId, bond: BondId, amount: f32);

pub struct FootHit { pub point: Vec3, pub normal: Vec3 }
```

`BroadPhaseLayer` 0 collides with 1 and 2. Layer 2 does not collide with layer 2. The pair filter can reject a pair after the layer test.

## File map

| Path | Owns |
| --- | --- |
| `crates/physics/src/lib.rs` | `World`, `step`, re-exports. Shrinks as modules land. |
| `crates/physics/src/body.rs` | `Body`, `BodyId`, `Motion`, `MotionQuality`. From step 1. |
| `crates/physics/src/broadphase.rs` | Two AABB trees, swept bounds, pair list. From step 5. |
| `crates/physics/src/narrowphase.rs` | GJK, EPA. From step 7. |
| `crates/physics/src/manifold.rs` | Clipped contacts and the cache. From step 9. |
| `crates/physics/src/solver.rs` | Soft step, sub-steps, warm start, sequential-impulse fallback. From step 10. |
| `crates/physics/src/joint.rs` | Fixed, hinge, distance. From step 11. |
| `crates/physics/src/island.rs` | Union-find, sleep. From step 12. |
| `crates/physics/src/query.rs` | Ray cast, shape cast. From step 13. |
| `crates/physics/src/hash.rs` | State hash. From step 14. |
| `crates/physics/src/ccd.rs` | Speculative margin, time of impact, fast-pair sub-steps. From step 18. |
| `crates/physics/src/origin.rs` | Wide position, origin shift. From step 21. |
| `crates/physics/src/color.rs` | Constraint colors. From step 22. |
| `crates/physics/src/upload.rs` | Awake pose buffer. From step 25. |
| `crates/physics/src/replicate.rs` | Quantized poses, bond events, checksum. From step 42. |
| `crates/physics/src/predict.rs` | Owned-body ring and rollback. From step 43. |
| `crates/physics/src/gravity.rs` | Sources, overrides, field query, wide Verlet. From step 47. |
| `crates/physics/src/vehicle.rs` | Wheeled vehicle and thrusters. From step 27. |
| `crates/physics/src/destruction.rs` | Runtime breakable, hot-swap, debris. From step 36. |
| `crates/physics/src/contact.rs` | Stays until step 9. Analytic oracles stay in tests. |
| `crates/physics/src/mesh.rs` | The 12-vertex / 6-piece reducer stays for `Shape::from_triangles`. |
| `crates/fracture/` | Offline Voronoi, `.gfract`. Package `genos-fracture`. From step 33. |
| `crates/anim/` | Skeleton and foot plant. Package `genos-anim`. From step 16. |
| `crates/scenes/` | Shared scene catalog. Package `genos-scenes`. From step 40. |
| `examples/physics-view/` | Debug binary `genos-physics-view`. From step 41. |
| `examples/character-view/` | Debug binary `genos-character-view`. From step 46. |
| `examples/vehicle-view/` | Debug binary `genos-vehicle-view`. From step 46. |
| `examples/fracture-view/` | Debug binary `genos-fracture-view`. From step 46. |

`genos-bake` is not the fracture tool. GI bakes and fracture assets stay apart.

## Steps

### P1. Motion kinds and angular velocity

- [ ] **Files:** create `crates/physics/src/body.rs`. Modify `lib.rs`. Test `crates/physics/tests/motion.rs`.
- **Behavior:** `Motion::Static` never integrates. `Motion::Kinematic` integrates a target linear velocity and a target angular velocity and ignores gravity. `Motion::Dynamic` integrates linear and angular velocity. Gravity and the existing spring still run for dynamic bodies. `orientation` changes only from angular velocity. Existing bodies: `inverse_mass == 0` is static, otherwise dynamic. Default angular velocity is zero, so current tests keep their orientations.
- **Test:** `a_kinematic_box_turns_and_a_dynamic_box_falls`
  - Kinematic box, target angular velocity `(0, π, 0)` rad/s, `dt = 1/60`, 30 steps. Orientation matches `Quat::from_axis_angle(Vec3::Y, π * 0.5)` within `1e-4` on the dot product. Position stays on the start XZ point. Y does not drop.
  - Dynamic sphere, no angular velocity, 400 steps of `dt = 0.001`, still within 1% of `½gt²` (the existing free-fall assertion).
- **Commit:** `physics: integrate kinematic motion and spin`

### P2. BodyId

- [ ] **Files:** `body.rs`, `lib.rs`, `crates/scene/src/camera.rs`. Test `crates/physics/tests/ids.rs` and `crates/scene/tests/room_collision.rs` (update the index checks this commit must keep green).
- **Behavior:** `World::insert` returns `BodyId`. `get` / `get_mut` return `None` after `remove`. Removing one body does not change another id. The scene stores `BodyId` for the view capsule and for solids. `insert` of a fresh body may reuse a slot and must bump the generation.
- **Test:** `body_id_survives_removal_of_another_body`
  - Insert three spheres with masses 1, 2, 3. Remove the middle id. The other two ids still read masses 1 and 3. The removed id reads `None`. A new insert does not compare equal to the removed id.
- **Commit:** `physics: address bodies with generational ids`

### P3. Shape store and convex margin

- [ ] **Files:** `crates/physics/src/shape.rs`. Test `crates/physics/tests/shapes.rs`.
- **Behavior:** Primitives can be shared by id from a store on `World`. A convex hull in the store has no 12-vertex cap. `convex_radius` shrinks the core and is added back in queries. Default radius is 0. `Shape::from_triangles` and `Body::mesh` keep today's reducer so `a_dense_concave_mesh_blocks_only_its_volume` and `a_sphere_and_capsule_stop_outside_every_face_of_a_convex_box_mesh` stay as they are. Add `Body::static_triangles` for a static triangle list that is not reduced and is not used as a dynamic collider.
- **Test:** `two_bodies_share_a_hull_and_a_margin_opens_a_gap`
  - Two dynamics share one hull id. Replacing the store's vertices is visible to both. Freeing one body leaves the other's hull intact.
  - A sphere of radius 0.25 and a box with `convex_radius` 0.02 come to rest with a gap of 0.02 ± 0.005 along the contact normal. The same pair with radius 0 rests inside 0.005 of the box face.
- **Commit:** `physics: share convex shapes and a convex margin`

### P4. Layers and pair filters

- [ ] **Files:** `broadphase.rs` (filter only; the tree is P5), `contact.rs`. Test `crates/physics/tests/layers.rs`.
- **Behavior:** Each body has an `ObjectLayer` and a `BroadPhaseLayer`. The world owns a layer pair table. A rejected pair never reaches `collide_pair`. Defaults put statics on layer 0, dynamics on layer 1, and accept 0–1 and 1–1, so existing scenes do not change. Debris is layer 2, and 2–2 is rejected.
- **Test:** `a_filtered_pair_never_reaches_the_contact_count`
  - Instrument the pair callback with a counter on `World` (test-only `pair_tests: u32`). Overlapping debris spheres: `pair_tests` stays 0 and the spheres keep their overlapping positions after one step. The same spheres on the dynamic layer increment `pair_tests` and separate.
- **Commit:** `physics: filter pairs by object layer`

### P5. Layered AABB trees

- [ ] **Files:** `broadphase.rs`. `contact.rs` asks the tree for pairs. Test `crates/physics/tests/broadphase.rs`.
- **Behavior:** One tree for static, one for dynamic (debris lives in the dynamic tree with its layer bit). Pair order is `(min id, max id)` sorted by `total_cmp` on the ids, which is the order the solver already used. Static-static pairs are not emitted. The hundreds-of-pillars test stays green.
- **Test:** `a_dynamic_sphere_among_ten_thousand_statics_tests_a_handful_of_pairs`
  - 100×100 static boxes on a grid, 2 m apart, one dynamic sphere above one box. After one step `pair_tests` is less than 32. A second run reports the same pair id list bitwise.
- **Commit:** `physics: query pairs with a layered aabb tree`

### P6. Batch insert, dirty expand, refit

- [ ] **Files:** `broadphase.rs`. Test `crates/physics/tests/broadphase.rs`.
- **Behavior:** `insert_many` builds a tree from the whole batch. Single inserts still work and mark the node dirty. A moved body expands its leaf. `refit` rebuilds a tight tree and swaps it in before the step returns. The swap is synchronous in this step. A later parallel step may overlap the rebuild; the pair list after the swap must match a full rebuild.
- **Test:** `batch_insert_matches_a_rebuild_and_a_move_keeps_the_pair`
  - 1 000 statics via `insert_many` and via one-at-a-time insertion produce the same pair list against one dynamic (the one-at-a-time tree may be fatter: assert its node count is greater than or equal, and the pair lists are equal after `refit`).
  - Move the dynamic by 3 m with `set_position`. The expanded leaf still reports the new overlapping static. After `refit`, the pair list equals a world built at the new pose.
- **Commit:** `physics: refit the aabb tree after a batch insert`

### P7. GJK distance

- [ ] **Files:** `narrowphase.rs`. Test `crates/physics/tests/gjk.rs`.
- **Behavior:** GJK distance and closest points for two convexes (sphere, capsule, box, hull). No contact response in this step. Analytic `primitive_contact` stays on the response path.
- **Test:** `gjk_distance_matches_separated_spheres_and_boxes`
  - Spheres radius 0.5 at `(0,0,0)` and `(2,0,0)`: distance 1 ± `1e-4`, closest points at x = 0.5 and x = 1.5.
  - Boxes half-extent 0.5, centers 3 m apart on X: distance 2 ± `1e-4`.
  - The same pairs a second time return bitwise-equal points.
- **Commit:** `physics: measure convex distance with gjk`

### P8. EPA penetration

- [ ] **Files:** `narrowphase.rs`. Test `crates/physics/tests/gjk.rs`.
- **Behavior:** When GJK reports overlap, EPA returns depth and normal. A test oracle runs the existing analytic box path (`box_box` in `contact.rs`, made `pub(crate)` for the test or reimplemented as a private test helper that calls the same function). Do not switch `solve_contact` over yet.
- **Test:** `epa_depth_matches_the_box_sat`
  - Two boxes half-extent `(0.5, 0.5, 0.5)`, centers 0.8 apart on X: depth 0.2 ± `1e-3`, normal along ±X.
  - A sphere of radius 0.5 whose center is 0.2 inside a box face: depth 0.2 ± `1e-3`.
- **Commit:** `physics: recover penetration with epa`

### P9. Manifolds and a contact cache

- [ ] **Files:** `manifold.rs`. `collide` writes the manifold into the solver. Test `crates/physics/tests/manifold.rs`.
- **Behavior:** Clip the incident face against the reference face (box and hull). At most four points. Persist points across steps by a feature id and reuse the stored impulse as the warm start. The deepest-only path remains available behind `World::single_contact` for the old tests that need one point. Default for new worlds is the manifold. Existing scene tests stay on the single-contact path until P10 flips the default and rechecks them.
- **Test:** `a_resting_box_keeps_four_contacts_and_its_orientation`
  - Dynamic box half-extent 0.5, identity orientation, dropped from 0.01 m onto a static box. After 120 steps of `1/60`, at least three contact points, orientation dot with identity > `1 - 1e-3`, vertical speed < 0.05.
- **Commit:** `physics: clip contact manifolds and warm start them`

### P10. Soft step

- [ ] **Files:** `solver.rs`. Test `crates/physics/tests/solver.rs`.
- **Behavior:** A contact is a soft constraint: frequency `hz` and damping ratio map to the bias and the impulse coefficient used by Box2D Soft Step and Bepu (spring omega, damping, effective mass). Default 4 sub-steps per `1/60` step, 4 velocity iterations. Sub-steps integrate and solve. They do not call the broadphase (`broadphase_builds` counts once per `step`). Warm-start impulses scale by the ratio of sub-step counts when the count changes. `World::solver = Sequential` keeps one impulse pass and is what the restitution tests use. Default becomes `Soft` only after `a_static_surface_bounce_keeps_restitution` still passes within 1% on `Sequential`, and the soft path has its own assertion below. Do not change `dt` inside an existing test.
- **Test:** `a_fifty_to_one_stack_stays_up_and_substeps_build_the_tree_once`
  - Five dynamic boxes, mass 1, on one dynamic box of mass 50, on a static plane. `hz = 30`, damping ratio 1, 4 sub-steps, 2 seconds. The bottom dynamic box's center stays within 0.15 m of its rest height. No orientation dot with identity falls below 0.95. `broadphase_builds == 120` for 120 outer steps, not 480.
  - `changing_substep_count_scales_the_cached_impulse`: one warm contact, switch from 4 to 2 sub-steps, the stored impulse is halved ± `1e-5`.
- **Commit:** `physics: solve contacts with a soft sub-step`

### P11. Joints

- [ ] **Files:** `joint.rs`. Test `crates/physics/tests/joints.rs`.
- **Behavior:** Fixed, hinge, and distance joints are soft constraints on the same solver. A joint connects two `BodyId`s and builds an island edge in P12. Limits are a min and max angle on the hinge.
- **Test:** `a_hinge_stays_in_its_plane_and_a_fixed_joint_holds`
  - Hinge along Y, gravity on, 1 second. The child position's Y equals the anchor Y ± 0.02. The offset on the hinge axis stays within 0.02 of the start.
  - Fixed joint, child mass 1, parent static, 1 second. Child position stays within 0.01 of the anchored pose.
- **Commit:** `physics: add soft fixed and hinge joints`

### P12. Islands and sleep

- [ ] **Files:** `island.rs`. Test `crates/physics/tests/sleep.rs`.
- **Behavior:** Union-find over contacts and joints during the narrowphase. An island sleeps when every dynamic member stays under a linear and angular speed for `sleep_time` seconds (defaults 0.1 m/s, 0.1 rad/s, 0.5 s). Wake is the whole island. `wake(id)` and a force or teleport wake it. `remove` does not wake neighbors. Asleep bodies are not integrated and their contacts are not solved (`solver_bodies` counts who the solver touched).
- **Test:** `two_piles_sleep_on_their_own_and_a_touch_wakes_one`
  - Two separated stacks of two boxes. After 3 seconds both islands are asleep and `solver_bodies` is 0 on the next step.
  - A kinematic sphere overlaps stack A. Stack A wakes (`solver_bodies` includes both of A's boxes) and stack B stays asleep.
  - `remove` of a body that only touched stack B leaves A asleep.
- **Commit:** `physics: sleep and wake whole islands`

### P13. Ray cast and shape cast

- [ ] **Files:** `query.rs`. Test `crates/physics/tests/query.rs`.
- **Behavior:** `ray_cast` and `shape_cast` walk the broadphase. The filter can reject a layer or a body. The nearest hit wins. Hits are bitwise-stable for the same world.
- **Test:** `a_ray_hits_the_near_box_and_a_capsule_cast_stops_at_a_wall`
  - Three boxes along X. A ray from the left hits the nearest face point within `1e-4` and reports that `BodyId`. A debris-only filter misses the dynamic boxes.
  - A capsule cast along +X stops with the capsule surface on the wall, fraction in `(0, 1)`, normal along -X.
- **Commit:** `physics: cast rays and shapes`

### P14. State hash

- [ ] **Files:** `hash.rs`. Test `crates/physics/tests/hash.rs`.
- **Behavior:** `hash_world` folds body id order (not `Vec` order), motion, pose, velocities, and cached contact impulses with FNV-1a. `atan2` is not used on this path. Two `step` loops in one process match. This is the determinism gate for later steps. A cross-OS golden hash is not claimed: `libm` is not added (ADR 0007), and `std` trig stays off the solver path.
- **Test:** `the_same_drop_hashes_equal_across_two_runs`
  - Sphere, box, and hinge. Clone the world after setup. Step both clones 60 times at `1/60`. The hashes match.
- **Commit:** `physics: hash the world for replay`

### P15. View body

- [ ] **Files:** `crates/physics/src/character.rs`, `crates/scene/src/camera.rs`. Test `crates/physics/tests/character.rs` and the existing `crates/scene/tests/codimation.rs`, `room_collision.rs`.
- **Behavior:** `ViewBody` is a kinematic capsule driven by a velocity wish. The character controller does not change: `update` still turns the `move` action into a wish. The capsule slides along walls, steps up a rise lower than `step_height` (default 0.3 m), sticks to a floor when the drop is under `stick` (default 0.2 m), and takes the horizontal velocity of the kinematic surface it stands on. Codimation keeps working by writing the solid's target velocity as kinematic motion. The record in `docs/systems/physics.md` and `docs/systems/scene.md` gains this game use in this commit.
- **Test:** `the_view_capsule_slides_steps_and_rides`
  - Wish `(2, 0, 0)` into a vertical wall: after 1 second the capsule center's X has not entered the wall, and Z is unchanged.
  - A 0.2 m step: the capsule ends on top, feet clearance within 0.02 m.
  - A platform moving at `(1, 0, 0)`: after it has carried the capsule, the capsule X matches the platform X ± 0.05 with a zero wish.
  - A downward slope of 20 degrees: with a zero wish the capsule's feet stay within 0.02 m of the surface after 1 second (it does not launch, and it does not sink).
- **Commit:** `physics: slide and step the view capsule`

### P16. Foot plant

- [ ] **Files:** create `crates/anim` (`src/lib.rs`, `src/foot.rs`). Test `crates/anim/tests/foot.rs` and `crates/scene/tests/foot_plant.rs`. Scene dev-dependency on `genos-anim` and `genos-input`. New record `docs/systems/anim.md`.
- **Behavior:** `Skeleton` is a list of joints (local translation, local rotation). `plant_foot` takes thigh length, shin length, hip position, pole vector, animated ankle, and a `FootHit`. Two-bone IK uses the law of cosines. Reach is clamped to `(abs(L1 - L2) + 1e-4, L1 + L2 - 1e-4)`. The knee bends toward the pole. Pelvis lift is the larger of the two feet's shortfall `(distance - (L1 + L2))` when the hit is farther than the leg, and it is also raised so an ankle that would sit under the hit is moved up onto the hit. Foot rotation's up axis aligns with `FootHit.normal`. The cast stays in physics: the scene test casts downward. `genos-load` is unchanged. GlTF skins stay ignored. The scene test links the action map: `InputSystem::set_key(InputCode::key_w, true)`, then `character_controller().axis_2d(&input, "move").y` is `1` (`key_w` is the up binding). `examples/camera` stores that value in `Actions.forward`. The test passes that forward value into `update`.
- **Test:** `feet_stay_on_a_slope_and_the_bones_keep_their_length` (`crates/anim`)
  - `L1 = L2 = 0.45`. Slope normal `(sin 20°, cos 20°, 0)`. Animated ankles 0.05 m under the plane. After `plant_foot` on both legs, each ankle is within `1e-3` of the plane on the outside (penetration deeper than `-1e-3` fails). Thigh and shin lengths stay within `1e-4`. `foot_up.dot(normal) > cos(1°)`. Pelvis Y is greater than the animated pelvis Y.
- **Test:** `the_view_body_casts_the_ground_the_planter_uses` (`crates/scene`)
  - Static box rotated 20° about Z. View capsule. One `update` of `1/60` with `forward` taken from the `move` axis above. Downward casts from both animated ankles hit the box. `plant_foot` results pass the same penetration, length, and normal assertions.
- **Commit:** `anim: plant feet on a cast hit`

### P17. Swept AABBs

- [ ] **Files:** `broadphase.rs`. Test `crates/physics/tests/broadphase.rs`.
- **Behavior:** A body with `MotionQuality::Swept` or `Speculative` contributes the AABB of its start pose expanded by linear travel over the step and by a bounding-sphere arc for angular travel (`radius * angle`). Discrete bodies contribute the current AABB. Pair tests use those bounds.
- **Test:** `two_boxes_that_swap_places_still_form_a_pair`
  - Box A at x = 0, velocity `(+60, 0, 0)`. Box B at x = 10, velocity `(-60, 0, 0)`. Half-extent 0.5. `dt = 1/60`. Start AABBs are disjoint. End AABBs are disjoint from the other start. Swept bounds overlap. The pair list contains `(A, B)`. The same setup with both `Discrete` does not contain the pair.
- **Commit:** `physics: pair bodies by their swept bounds`

### P18. Speculative contacts

- [ ] **Files:** `ccd.rs`. Test `crates/physics/tests/ccd.rs`.
- **Behavior:** `MotionQuality::Speculative` grows the contact distance by `min(speed * dt, speculative_margin)` with default margin 0.25 m. The solver stops the motion. This is the bullet path. It is not the ship path.
- **Test:** `a_bullet_inside_the_margin_does_not_tunnel`
  - Sphere radius 0.05, speed 10 m/s (travel 0.167 m at `1/60`, under the 0.25 m margin), static wall at 0.2 m. After one step the sphere center is on the impact side of the wall.
  - The same sphere at 3 000 m/s is not this test. P19 owns that speed.
- **Commit:** `physics: stop modest bullets with speculative contacts`

### P19. Time of impact for two moving convexes

- [ ] **Files:** `ccd.rs`. Test `crates/physics/tests/ccd.rs`.
- **Behavior:** `MotionQuality::Swept` uses conservative advancement. GJK distance plus the relative velocity of the closest points, including both linear velocities and both angular velocities, produces a time of impact. Both bodies move. The step advances the pair to a fraction of that time (0.8 of the remaining time of impact, minimum progress `1e-4` s), solves the contact, and repeats until the step is consumed or 16 advances have run. Other bodies keep the normal sub-step. A missed root (no progress) leaves the bodies at the last safe time and flags `toi_failed` on the world for the test to read.
- **Test:** `two_convex_ships_do_not_tunnel_head_on`
  - Each ship is a box, half-extents `(2, 1, 1)` (4 m along X). Centers start 40 m apart. Both `Swept`. Each speed in `[30, 120, 600, 3000]` m/s, opposite directions, one `step` of `1/60`. Travel per ship at 3 000 m/s is 50 m, which is more than the body length. Combined closing travel is 100 m.
  - Assert for each speed: the two centers never exchange X order; the inner-face gap stays `>= -0.01` m; `toi_failed` is false; a contact impulse was stored for the pair; a second run matches positions bitwise.
- **Test:** `a_spinning_convex_does_not_tunnel_through_a_wall`
  - Box half-extent 1 m, angular velocity `(0, 0, 40)` rad/s, linear velocity 30 m/s into a static wall 2 m away. After one step the box bounds do not pass the wall plane.
- **Commit:** `physics: resolve swept time of impact for both bodies`

### P20. Fast-pair sub-steps

- [ ] **Files:** `ccd.rs`, `solver.rs`. Test `crates/physics/tests/ccd.rs`.
- **Behavior:** If a pair's relative travel over the outer step exceeds that pair's smaller bounding radius, that pair integrates in extra slices (travel per slice ≤ radius). The rest of the world keeps the P10 sub-step count. Counters: `pair_slices(id_a, id_b)` and `body_slices(id)`.
- **Test:** `only_the_fast_pair_takes_extra_slices`
  - A resting box on a plane, and two swept spheres closing at 600 m/s. After one step the resting box's slice count equals the global sub-step count (4). The fast pair's slice count is greater. The spheres' inner gap stays `>= -0.01`.
- **Commit:** `physics: sub-step a pair that outruns its radius`

### P21. Wide positions and origin shift

- [ ] **Files:** `origin.rs`. Test `crates/physics/tests/origin.rs`. `docs/systems/math.md` gains a sentence that physics wide positions are not `genos-math`.
- **Behavior:** `WidePos` stores `f64` metres. `World::origin: WidePos` is the frame of every `Body.position`. `shift_origin(new_origin)` subtracts the delta from every body and from the broadphase. Contact runs in `f32` relative to that origin. The renderer, when a caller asks, receives the same `f32` positions plus the origin. `genos-math::Vec3` is unchanged.
- **Test:** `a_contact_at_1e7_metres_matches_the_origin_after_a_shift`
  - Two boxes, world origin 0, contact gap after 60 steps recorded. The same setup with `WidePos` x = `1e7` and a shift that brings the pair near 0. The gap matches within `1e-4`. Body `f32` positions after the shift are within 10 m of the origin.
- **Commit:** `physics: shift the origin for large worlds`

### P22. Constraint coloring

- [ ] **Files:** `color.rs`. Test `crates/physics/tests/color.rs`.
- **Behavior:** Build colors so two constraints in one color share no body. Edges are sorted by `(body_a, body_b, joint_index)` before coloring, so the colors are a function of the graph. The solver runs color 0 to completion, then color 1, on one thread. Islands still decide who is awake. A sleeping island adds no edges.
- **Test:** `colors_are_disjoint_and_stable`
  - A pile of 30 boxes. Every color's edges have unique body ids. Two builds produce the same color lists. An asleep second pile adds no edges (`edge_count` equals the awake pile alone).
- **Commit:** `physics: color the constraint graph`

### P23. Parallel color solve

- [ ] **Files:** `solver.rs`, `lib.rs` (`step_parallel`). Test `crates/physics/tests/parallel.rs`.
- **Behavior:** `std::thread::scope` solves one color across workers. Workers share no body. The main thread joins before the next color. `step` is the one-thread color order. `step_parallel` must hash-equal `step` on the same world.
- **Test:** `parallel_colors_match_the_serial_hash`
  - 200 dynamic boxes on a plane, 30 steps, `step_parallel(..., 4)` hash equals `step` hash.
- **Commit:** `physics: solve constraint colors in parallel`

### P24. Parallel narrowphase

- [ ] **Files:** `narrowphase.rs`. Test `crates/physics/tests/parallel.rs`.
- **Behavior:** Pairs are generated on the main thread from the trees (the tree walk stays serial in this step). Narrowphase runs per pair in parallel into pair-indexed slots, then the slots are sorted by body id before the solver. The merged manifold list equals the serial manifold list bitwise.
- **Test:** `parallel_manifolds_match_the_serial_list`
  - The 200-box pile, one step. Manifold feature ids and points match the serial run.
- **Commit:** `physics: build manifolds in parallel and sort them`

### P25. Awake pose upload

- [ ] **Files:** `upload.rs`. Test `crates/physics/tests/upload.rs`.
- **Behavior:** `PoseBuffer` is a `Vec<Pose>` indexed by `BodyId.index`, plus a list of dirty ids. `publish` writes poses of bodies that are awake or that woke, slept, or were removed this step. A fully asleep world publishes an empty dirty list. This buffer is what a Vulkan upload will copy. This step does not call Vulkan. Chunk meshes in P36 reuse the same rule: vertices are written when the chunk spawns, not every step.
- **Test:** `a_sleeping_world_publishes_no_poses`
  - One stack, step until asleep, `publish`, dirty list empty. Wake one body, `publish`, the dirty list is that island's ids only.
- **Commit:** `physics: publish poses for awake bodies only`

### P26. Thousands of active bodies

- [ ] **Files:** test `crates/physics/tests/stress.rs`. The scene builder moves to `genos-scenes` in P40; until then the builder is a function in the test that P40 calls.
- **Behavior:** No new solver feature. This is the scale gate for the CPU path.
- **Test:** `four_thousand_active_spheres_match_across_runs`
  - 64×64 spheres, radius 0.25, spaced 2 m apart, 1 m above a static plane, 30 steps of `1/60`. They stay active (falling or just landed). No position is non-finite. Two runs hash-equal. `pair_tests` is below `4096 * 8` (the plane plus a few neighbors, not the square of the body count).
- **Test:** `an_asleep_grid_does_not_enter_the_solver`
  - 64×64 statics and 8 dynamics that have already come to rest (place them on the plane with zero velocity and step 2 seconds). The next step has `solver_bodies <= 8` before sleep and `solver_bodies == 0` after the sleep timer. The static count does not appear in `solver_bodies`.
- **Commit:** `physics: test thousands of active and asleep bodies`

### P27. Wheel suspension

- [ ] **Files:** `vehicle.rs`. Test `crates/physics/tests/vehicle.rs`. Record the game use in `docs/systems/physics.md`.
- **Behavior:** A wheeled vehicle is a constraint on one chassis `BodyId`, in the sense of Jolt's `VehicleConstraint` / `WheeledVehicleController`: the wheel is not a body. Each wheel has a local position, a suspension direction, min and max length, frequency, and damping. Each step casts a ray, or a cylinder shape if `WheelCast::Shape` is set, excluding the chassis. The suspension spring uses the P10 soft constraint along the cast. Default cast is the ray.
- **Test:** `a_chassis_rests_on_four_springs`
  - Chassis mass 1 200 kg, four wheels, rest length 0.4 m, `hz = 4`, damping ratio 0.7, on a static plane. After 3 seconds the chassis height is within 0.02 m of `rest_length - compression`, where compression is `weight/4 / stiffness` and stiffness comes from the frequency and the wheel's suspended mass. Vertical speed < 0.05. `wheel_cast_count` over the last step equals 4.
- **Commit:** `physics: suspend a chassis on cast wheels`

### P28. Tyre slip

- [ ] **Files:** `vehicle.rs`. Test `crates/physics/tests/vehicle.rs`.
- **Behavior:** Longitudinal slip is `(wheel_omega * radius - contact_speed_forward) / max(|contact_speed|, slip_denom)`. Lateral slip is the angle between the wheel heading and the contact velocity. A simplified curve: friction rises linearly to `mu_peak` at `slip_peak` (defaults 1.0 and 0.1) and falls to `mu_slide` (default 0.7) at slip 1. The impulse is capped by the suspension normal load.
- **Test:** `a_locked_wheel_slows_the_car_and_a_free_wheel_rolls`
  - Chassis at 10 m/s, wheel omega 0, `mu_peak` 1. After 1 second speed is below 6 m/s.
  - The same chassis with omega set each step to `speed / radius`. After 1 second speed stays above 9 m/s on a flat plane with no brake.
- **Commit:** `physics: apply a simplified tyre slip curve`

### P29. Engine, gearbox, differential

- [ ] **Files:** `vehicle.rs`. Test `crates/physics/tests/vehicle.rs`.
- **Behavior:** Torque curve is a table of `(rpm, torque)` in N·m, linearly interpolated. Gearbox is a list of ratios, including a negative reverse, plus a final drive. `gear` is an index the game sets. Engine omega comes from the driven wheels through the engaged ratio. An open differential writes equal drive torque to the driven wheels. Clutch is 0 or 1 in this step.
- **Test:** `first_gear_drives_forward_and_an_open_diff_spins_the_free_wheel`
  - Throttle 1, gear 0, ratio 3, curve `(0 rpm, 200 N·m)` to `(5 000 rpm, 200 N·m)`. After 2 seconds the chassis velocity X is positive and greater than 1 m/s.
  - Gear reverse. After 2 seconds from rest, velocity X is negative.
  - One driven wheel held in the air (no cast hit) and one on the plane. The airborne wheel's omega exceeds the grounded wheel's omega after 1 second of throttle.
- **Commit:** `physics: drive wheels through a gearbox and an open diff`

### P30. Anti-roll bar

- [ ] **Files:** `vehicle.rs`. Test `crates/physics/tests/vehicle.rs`.
- **Behavior:** An anti-roll bar is a soft constraint between the suspension compressions of a left and right wheel. Stiffness is N/m of compression difference. It adds equal-and-opposite vertical forces on the chassis at the two wheel positions.
- **Test:** `an_anti_roll_bar_cuts_the_roll`
  - A 0.1 m step under the left wheels only. Without the bar, after 1 second the roll angle is recorded. With stiffness `20_000`, the roll angle is under half of that recorded angle. The chassis stays finite.
- **Commit:** `physics: add an anti-roll bar`

### P31. Pacejka curve

- [ ] **Files:** `vehicle.rs`. Test `crates/physics/tests/vehicle.rs`.
- **Behavior:** Optional longitudinal and lateral curves. `pacejka(slip, b, c, d, e) = d * sin(c * atan(b * slip - e * (b * slip - atan(b * slip))))`. When a wheel has coefficients, that curve replaces the simplified mu. `sin` and `atan` here are `f32` methods used only inside this force, and the vehicle test hashes the resulting speed so a change is visible. The simplified curve remains the default.
- **Test:** `pacejka_matches_three_known_points`
  - `b = 10`, `c = 1.9`, `d = 1`, `e = 0.97`. Assert `pacejka(0) == 0`, `pacejka` at slip 0.1 is greater than at slip 0.5 (the peak is earlier than the slide), and a second call returns the same bits.
- **Commit:** `physics: offer a pacejka slip curve`

### P32. Thrusters and a large orbit

- [ ] **Files:** `vehicle.rs`. Test `crates/physics/tests/vehicle.rs`.
- **Behavior:** A thruster is a local position, a local direction, and a max force. Throttle is 0 to 1. Force and torque (`r × F`) are applied to the chassis. Gravity may be zero. The orbit helper integrates `WidePos` with `f64` and `GM` (m³/s²): acceleration `-GM * r / |r|³`, velocity Verlet as in the gravity section. It does not use `Body.position` for the integration state. After each physics step the body pose is the orbit state minus `World::origin`. P52 turns this helper into a gravity source. This test stays green after that.
- **Test:** `a_thruster_in_zero_gravity_matches_force_and_torque`
  - Mass 1 000 kg, force 1 000 N through the center, 1 second, gravity 0. Velocity equals `(1, 0, 0)` ± `1e-3`. A second thruster at local `(0, 2, 0)` pushing +X produces angular velocity about Z of `torque / inertia` within 1% (inertia is the box inertia from P1).
- **Test:** `a_circular_orbit_at_1e7_metres_keeps_its_radius`
  - Radius `1e7` m, `GM = 3.986e14`, circular speed `sqrt(GM / r)`, 200 steps of 1 s in `f64`. Radius stays within 0.05 m of `1e7`. The body `f32` position after an origin shift onto the ship is within 1 m of the origin.
- **Commit:** `physics: thrust in zero gravity and integrate a wide orbit`

### P33. Seeded Voronoi cells

- [ ] **Files:** create `crates/fracture` (`src/lib.rs`, `src/voronoi.rs`). Test `crates/fracture/tests/voronoi.rs`. Record `docs/systems/fracture.md` (goal, intent, code, game use, limits). Do not modify `genos-bake`.
- **Behavior:** `voronoi_convex(hull_planes, seeds) -> Vec<Cell>`. Cell `i` is the source hull clipped by the perpendicular-bisector planes against every other seed. Seed order is the cell order. Planes are applied in increasing seed index. No `atan2` and no hashmap iteration. A cell with volume under `1e-8` is dropped and the function returns an error listing the seed, so a bad seed set fails the test instead of disappearing.
- **Test:** `two_seeds_split_a_cube_into_equal_volumes`
  - Cube from -1 to 1. Seeds `( -0.5, 0, 0 )` and `( 0.5, 0, 0 )`. Two cells. Volumes each 4 ± `1e-3` (cube volume 8). A second call returns bitwise-equal vertices.
- **Commit:** `fracture: clip a convex into seeded voronoi cells`

### P34. Interior faces

- [ ] **Files:** `crates/fracture/src/clip.rs`. Test `crates/fracture/tests/clip.rs`.
- **Behavior:** Clip a closed triangle mesh by the same planes. Triangles keep the source material id. Polygons created on a cut plane are fanned into triangles with `interior_material`. Each chunk is closed: every edge is shared by two triangles. Volume is the sum of signed tetrahedra from the origin.
- **Test:** `a_cut_box_mesh_gains_an_interior_cap_and_keeps_its_volume`
  - The 12-triangle cube, side 2, two seeds as in P33. Sum of chunk volumes is 8 ± `1e-2`. Interior material area is greater than 0. Exterior area equals 24 ± `1e-2` (six faces of area 4). The second run matches triangle vertices bitwise.
- **Commit:** `fracture: emit interior faces on the cut`

### P35. Hierarchy and the support graph

- [ ] **Files:** `crates/fracture/src/asset.rs`. Test `crates/fracture/tests/asset.rs`.
- **Behavior:** Level 0 is the coarse seeds. Each coarse cell gets `children_per_chunk` child seeds, placed on a fixed grid inside the cell (not a random source). The asset stores parent, `first_child`, `child_count`. A bond connects two chunks that share a face, with area, centroid, normal, and `health = health_per_area * area`. A chunk flagged support also has a bond whose other end is `WORLD` (`u32::MAX`). `write_gfract` / `read_gfract` use version `1` and little-endian `f32`s. A bad version returns an error.
- **Test:** `coarse_chunks_contain_their_fine_children`
  - Two coarse seeds, two children each. Asset chunk count is 6 (2 coarse + 4 fine). Each fine chunk's parent is the coarse cell that contains its centroid. Fine volumes inside a coarse cell sum to that cell's volume ± `1e-2`. The world bond count equals the support count. Round-trip bytes load equal.
- **Commit:** `fracture: store a hierarchy and a bond graph`

### P36. Hot-swap

- [ ] **Files:** `crates/physics/src/destruction.rs`. Test `crates/physics/tests/destruction.rs`. `docs/systems/physics.md` gains the runtime game use. `docs/systems/fracture.md` points at it.
- **Behavior:** `Breakable` starts as one dynamic or static body (the intact mesh). Chunks are not bodies yet. `activate(chunk)` removes that chunk's body and spawns its children as bodies with the parent's linear and angular velocity, then adds a fixed joint for each remaining child bond (P11). A spawned chunk that can still walk bonds to a support stays `Motion::Static` until that path is gone; a component with no support becomes dynamic. Unbroken coarse chunks stay the parent body. Activation is local: a chunk outside the damaged chunk's neighbor set is not spawned.
- **Test:** `one_hit_spawns_only_the_damaged_branch`
  - Asset from P35, two coarse chunks, four fine. Intact world has 1 breakable body. `activate` on coarse chunk 0. Body count of the breakable becomes 1 (the untouched coarse chunk) + 2 (its children). Coarse chunk 1 has no child bodies. Child velocities equal the parent velocity within `1e-5`.
- **Commit:** `physics: swap a broken chunk for its children`

### P37. One damage API

- [ ] **Files:** `destruction.rs`, `solver.rs` (impulse report). Test `crates/physics/tests/destruction.rs`.
- **Behavior:** `apply_damage(breakable, bond, amount)` subtracts `amount` from bond health. At health `<= 0` the bond and its joint are removed, `activate` runs on the chunks that edge held, and both islands wake. The solver calls `apply_damage` with the normal impulse of a contact whose two bodies are chunks sharing a bond. A game call uses the same function. `remove` of an unrelated body still does not wake this island (P12).
- **Test:** `an_impulse_and_a_health_event_break_the_same_bond`
  - Column of three chunks, bottom is support. `apply_damage` of the top bond's full health with no collision: the top chunk becomes dynamic and falls for 30 steps; the middle chunk stays static; the bottom stays static.
  - A fresh column. A dynamic sphere strikes the top bond. The reported contact impulse over the hit step is passed through `apply_damage`. When that sum exceeds health, the same top chunk is dynamic and the middle chunk is static.
  - Hashes of the two outcomes are not required to match (the sphere is extra). The bond id that breaks is the top bond in both.
- **Commit:** `physics: break bonds from impulses and from health`

### P38. Debris sleep and cull

- [ ] **Files:** `destruction.rs`. Test `crates/physics/tests/destruction.rs`.
- **Behavior:** A spawned chunk with volume under `debris_volume` (default `0.02` m³) is `BroadPhaseLayer` 2 and uses a sleep timer of `0.1` s and a speed threshold of `0.4` m/s. Debris-debris pairs stay filtered (P4). After the step, if that breakable's debris count exceeds `debris_budget`, remove the smallest asleep debris first. Awake debris is kept. Removal of asleep disconnected debris does not wake the parent island. Culled ids publish a removal in the P25 buffer.
- **Test:** `debris_sleeps_fast_and_the_budget_drops_the_smallest`
  - One gameplay box and one debris chunk, both at speed 0.05 m/s. After 6 steps of `1/60` the debris body is asleep and the gameplay box is awake (`sleep_time` 0.5 s has not elapsed).
  - 20 asleep debris, budget 8. After one step, 8 debris remain, and they are the 8 largest volumes. `solver_bodies` for the removed ids is 0. The support island stays asleep.
- **Commit:** `physics: sleep and cull debris`

### P39. Optional slice

- [ ] **Files:** `destruction.rs`, `genos-fracture` clip used as a library call. Test `crates/physics/tests/destruction.rs`.
- **Behavior:** `slice_chunk(breakable, chunk, plane_normal, plane_offset) -> Option<(u32, u32)>` clips one leaf chunk into two, writes interior faces, and spawns both as dynamic bodies with a bond that is already broken. It allocates. `step` does not call it. The breakable must be created with `allow_slice: true`. The default is false, and a false breakable returns `None` without changing the world.
- **Test:** `a_slice_splits_a_box_and_the_step_does_not_slice`
  - Box leaf, plane x = 0, `allow_slice: true`. Two chunks, volumes sum to the box volume ± `1e-2`, each has interior area > 0, both dynamic. They separate under zero gravity when given opposite 0.1 m/s pushes (the test sets the velocities). A second breakable with `allow_slice: false` returns `None` and keeps one body. A world stepped 10 times with no `slice_chunk` call keeps its chunk count.
- **Commit:** `physics: slice one chunk when the game asks`

### P40. Scene catalog

- [ ] **Files:** create `crates/scenes` (`src/lib.rs`, `src/report.rs`, one module per scene). Test `crates/scenes/tests/catalog.rs`. Update `docs/architecture.md`.
- **Behavior:** `SceneCatalog::all()` returns a fixed list. Each entry has a name, a one-line description, a kind (`Physics` or `Room`), and `build() -> PhysicsScene`. `PhysicsScene` holds the start `World`, the extra breakable / vehicle / orbit state, a suggested camera eye and yaw, and `fn metrics(&self) -> Value` using `genos-json`. `run(name, frames) -> Report` clones the start, steps `frames` times at `1/60`, and fills `Report`. `reset` restores that start. Room entries store a path string and do not step. Order is the order in the source match, which is the picker order.
- **Catalog:**

| Name | Kind | What it builds |
| --- | --- | --- |
| `drop` | Physics | One sphere, one plane. |
| `stack` | Physics | The five-on-one stack from P10. |
| `asleep_grid` | Physics | P26 asleep grid. |
| `pile_4k` | Physics | P26 active spheres. |
| `view_stairs` | Physics | View capsule, a 0.2 m step, a 20° slope. |
| `foot_slope` | Physics | P16 slope, two legs, the action-map wish. |
| `car_flat` | Physics | P29 car, throttle 1, gear 0. |
| `car_bump` | Physics | P30 one-side step. |
| `orbit` | Physics | P32 orbit, gravity 0, a second ship at a nearby wide position. |
| `gravity_well` | Physics | A fixed source with a circular orbit, a kinematic moon for a slingshot, a 100 m planet with a walker, a zero-g box, and a spin habitat. |
| `ship_impact` | Physics | Four lanes of P19, speeds 30, 120, 600, 3 000. |
| `destruction_wall` | Physics | Two columns of the P35 asset. Left column is struck by a sphere. Right column takes `apply_damage` on the matching bond at frame 30. `debris_budget` 8. |
| `ship_station` | Physics | One swept ship at 600 m/s and a supported wall of chunks. |
| `camera_room` | Room | `examples/camera/scene.rhai`. |
| `stress_room` | Room | `examples/stress/scenes/room.rhai`. |

`gravity_well` is registered in P54. The P40 order test covers the rows that exist in that commit. P54 extends the list.

- **Test:** `every_physics_scene_steps_and_the_named_gates_hold`
  - Catalog names are unique and in the table order. `run("drop", 10)` is finite and a second `run` hashes equal.
  - `ship_impact` after 5 steps: every lane `tunneled == false`, `contacted == true`, `min_separation >= -0.01`. `passed == true`.
  - `destruction_wall` after 90 steps: `bonds_broken >= 1` on each column, fine chunks exist only on the struck coarse chunk of each column, `debris_alive <= 8`, no chunk bounds center has `y` below the floor by more than its half-extent, `passed == true`.
  - `ship_station` after 5 steps: `tunneled == false` and `bonds_broken >= 1`.
  - `pile_4k` after 5 steps (the catalog test uses 5, not 30): finite, `passed == true`. The 30-step hash remains the P26 test.
  - `foot_slope` after 30 steps: ankle penetration `>= -1e-3`, bone length error `< 1e-4`.
  - `orbit` after 60 steps: radius error `< 0.05` m.
  - `reset` after `run` restores the start hash.
- **Commit:** `scenes: add the shared physics scene catalog`

### P41. Physics debug binary

- [ ] **Files:** `examples/physics-view` (`src/main.rs`, `src/lib.rs`), package `genos-physics-view`, binary name `genos-physics-view`. Depend on `genos-scenes`, `genos-window`, `genos-input`, `genos-render`, `genos-ui`, `genos-debug`, `genos-mcp`, `genos-json`. Modify `examples/stress` and `examples/camera` to depend on `genos-scenes` and to honor `--list-scenes` by printing `SceneCatalog::all()` and exiting. Document the binary in `docs/systems/debugging.md` and `docs/architecture.md`.
- **Behavior:** This binary is the rigid-body system. Character, vehicle, and fracture binaries are P46 and use this same host shape.
  - `--list-scenes` prints `name kind description` and exits 0.
  - `--scene NAME` selects one scene. Unknown name exits 1. The scene is the preset, the way a lighting knob selects a setup.
  - With no `--scene` and a terminal on stdin, print the numbered catalog and read one line. The window, on its first frame, shows the same list with `genos-ui`. `key_up` / `key_down` move, `key_enter` starts, `key_1` through `key_9` jump. This list is the human picker.
  - Commands match [Debugging](../debugging.md): the same JSON over MCP `tools/call`, `POST /cmd/<name>`, and Rhai. Names: `physics_metrics`, `take_screenshot`, `time`, `knob`, `reset`, `scene`, `view`. `genos-debug`'s `Tools` runs in the frame loop (`before_draw` / `after_draw`). Do not mount the lighting reference tracer.
  - `reset` and `key_r` restore the catalog start. Bind `key_r` on a debug action map in this binary. Do not add it to `character_controller()`.
  - `--headless --metrics PATH --frames N` steps without a window and writes the `genos-json` report. Exit 1 when `passed` is false.
  - `--script FILE --report DIR` runs a Rhai script (`step`, `shot`, `knob`, `check`, `require`) and writes `report.md`, `result.json`, and images, as `genos-stress` does.
  - `--shot PATH` writes one PNG through `genos_debug::png::write_png` after the last frame.
  - `view` modes for this binary: `bodies`, `contacts`, `islands`, `sleep`, `sweeps`.
  - Knobs: `dt_scale`, `solver`, `substeps`, `sleep`.
  - One script ships in this commit: `examples/physics-view/scripts/stack.rhai`. It pauses, steps 60 frames, shots `stack`, and `require`s `physics_metrics.passed`.
  - The picture draws bodies, contacts, and island colors. Poses come from the P25 buffer.
  - Frame `dt` is measured. A hitch still clamps the accumulator to 0.25 s. Headless mode steps exactly `--frames` steps of `1/60` and ignores the wall clock.
- **Test:** `crates/scenes/tests/catalog.rs` already covers metrics. Add `examples/physics-view/tests/args.rs`:
  - `parse` of `--headless --scene ship_impact --metrics out.json --frames 5` yields that scene and does not open a window type (the parse result has `headless: true`).
  - `parse` of `--list-scenes` sets `list_only`.
  - The stress and camera `--list-scenes` paths are the same string function `SceneCatalog::list_text()`, tested once in `genos-scenes`.
- **Commit:** `physics-view: pick a scene, reset, write metrics and a screenshot`

### P42. Quantized replication

- [ ] **Files:** `crates/physics/src/replicate.rs`. Test `crates/physics/tests/replicate.rs`. Copy the five networking lines into `docs/systems/physics.md`.
- **Behavior:** `replicate(world) -> Vec<ReplicatedBody>` writes one record per dirty awake body, or per body whose sleep bit changed, in `NetId` order. Positions are millimetres relative to the origin. A fully asleep unchanged world returns an empty vec. `pose_at_tick` stores the last 32 quantized poses per body for the session's hit rewind. `replicated_checksum` hashes those records. Foot-plant output is not an input to the hash.
- **Test:** `an_asleep_world_replicates_nothing_and_a_shift_sends_the_origin`
  - Stack from P12 after it sleeps: `replicate` is empty. Wake one body: the vec is that island only. Shift the origin: the next record includes the `f64` origin and the body's millimetre position matches the unshifted case within 1 mm.
- **Commit:** `physics: publish quantized poses for awake bodies`

### P43. Owned-body prediction and rollback

- [ ] **Files:** `crates/physics/src/predict.rs`. Test `crates/physics/tests/predict.rs`. Update the prediction line in `docs/systems/physics.md`.
- **Behavior:** `predict(view, input)` stores the `InputFrame` and steps only the owned view body. `reconcile(server_pose_at_T)` restores that body when the quantized pose differs, then replays later inputs. Other bodies are left where `apply_snapshot` put them. `apply_snapshot` copies quantized poses onto remote bodies and does not call the solver.
- **Test:** `owned_body_reconciles_after_a_dropped_input`
  - Two worlds, same start, 30 ticks. The client predicts with `forward = 1` on every tick. Tick 10's input is dropped on the server (`forward = 0` for that tick only). The client's quantized position at tick 10 differs from the server. `reconcile` then replay leaves the client owned body within 1 mm of the server. A dynamic box in front of the capsule is moved on the server and, on the client, stays on the snapshot pose from before the local prediction (`a_predicted_capsule_does_not_move_a_dynamic_box`).
- **Test:** `replicated_checksum_ignores_foot_plant`
  - Run `plant_foot` on a copy of the world. The checksum equals the copy that did not plant.
- **Commit:** `physics: roll back the owned view body`

### P44. Vehicle replication

- [ ] **Files:** `replicate.rs`, `vehicle.rs`. Test `crates/physics/tests/replicate.rs`. Vehicle networking lines in `docs/systems/physics.md`.
- **Behavior:** `replicate_vehicle` writes the chassis quantized pose, rpm, gear, steer, and each wheel's omega and suspension length. It does not write the contact cache or the torque-curve samples. `apply_vehicle_snapshot` sets those fields and does not call the wheel solver. Throttle, brake, steer, and gear request are read from the owner's `InputFrame` on the server only.
- **Test:** `vehicle_bytes_are_the_chassis_and_the_wheels`
  - One car, one step of throttle on the server. The serialized record is at most 64 bytes plus 8 bytes per wheel. A client world that only calls `apply_vehicle_snapshot` between two samples has a chassis position on the segment between them, and its `wheel_cast_count` stays 0.
- **Commit:** `physics: replicate a vehicle as a snapshot`

### P45. Destruction events

- [ ] **Files:** `replicate.rs`, `destruction.rs`. Test `crates/physics/tests/replicate.rs`. Fracture networking lines in `docs/systems/fracture.md` and `docs/systems/physics.md`.
- **Behavior:** Spawn replication is the seed list, child counts, interior material, and support flags, plus the spawn pose. A client that already has the file accepts the content hash in place of the seeds and loads that file. A bond that breaks appends a `BondBreak`. A slice appends a `Slice`. `apply_events` feeds those through `apply_damage` and `slice_chunk`. `break_baseline` lists broken bond ids and awake chunk poses.
- **Test:** `bond_breaks_replicate_without_the_mesh`
  - Server and client build from the same seed list. The server's sphere breaks the top bond. The event list contains that bond id and no vertex. The client applies the events. Broken bond ids match. Spawned chunk count matches. A second client that only receives `break_baseline` reaches the same bond set. One break event is at most 32 bytes. A spawn that sends seeds is smaller than the generated vertex buffer.
- **Commit:** `physics: replicate fracture as seeds and bond events`

### P46. Character, vehicle, and fracture debug binaries

- [ ] **Files:** `examples/character-view`, `examples/vehicle-view`, `examples/fracture-view`, each a thin binary on the P41 host. Scripts: `examples/character-view/scripts/foot_slope.rhai`, `examples/vehicle-view/scripts/ship_impact.rhai`, `examples/fracture-view/scripts/destruction_wall.rhai`. Document the four binaries and their commands in `docs/systems/debugging.md`.
- **Behavior:** Each binary defaults to its row in the observability table, exposes that row's knobs and view modes, and supports the shared commands, `--headless`, `--script`, `--report`, the picker, and `key_r`. The session binary is not created here. `replicated_checksum` and the owned-body error are fields on `physics_metrics` so the core-networking session view can read them.
- **Test:** `examples/character-view/tests/args.rs` (and the same parse test in the vehicle and fracture packages): `--script scripts/foot_slope.rhai --report out` sets `script` and `report`. `crates/scenes/tests/catalog.rs` asserts `physics_metrics` for `foot_slope`, `ship_impact`, and `destruction_wall` still carry `passed`.
- **Commit:** `debug: add character, vehicle, and fracture view binaries`

### P47. Inverse-square sources

- [ ] **Files:** `crates/physics/src/gravity.rs`. Test `crates/physics/tests/gravity.rs`. Record the field in `docs/systems/physics.md`. `CONTEXT.md` gains gravity source in this commit.
- **Behavior:** `GravityMode::Uniform` is the default. `step` adds `World.gravity` and does not call `gravity.rs`. `set_gravity_mode(Field)` opts in. In `Field`, `gravity_at` is `World.gravity` plus each in-range source. `a = GM / max(r, clamp_radius)²` toward the source. Outside `influence`, the source adds nothing. Sources sum in id order. `insert_source` while `Uniform` stores nothing.
- **Test:** `a_uniform_world_pays_nothing_for_wells`
  - `World::new(GRAVITY)` reports `Uniform`. After 60 steps, `field_evaluations` is 0 and the source storage has not allocated. `insert_source` leaves the mode `Uniform` and the source count 0. Free fall still matches `½gt²` within 1%. `timed_math_and_physics_batch_does_not_allocate` still passes.
  - A second world, `set_gravity_mode(Field)`, keeps `World.gravity = GRAVITY` and adds one source. Outside any volume the sample is the vector plus the well. Inside a zero volume the sample is `0`.
- **Test:** `inverse_square_matches_surface_gravity_and_a_hyperbola`
  - The world is `Field` with `World.gravity = 0`. Planet `GM = 98100`, `clamp_radius = 1`, `influence = 1e6`. At 100 m, `|a|` is `9.81` ± `1e-3`, direction toward the source. At `r = 0`, `|a|` is `GM / clamp_radius²` and finite.
  - Hyperbola: `GM = 10`, impact parameter `20` m, `v_inf = 1` m/s, uniform field `0`, Verlet at `dt = 0.5` s from 200 m out and back out. The deflection from the incoming direction is `2 atan(GM / (b v_inf²))` = `2 atan(0.5)` within `3°`. Speed returns to within 2% of `v_inf`. A second run matches the wide position bitwise.
- **Commit:** `physics: sample inverse-square gravity per body`

### P48. Moving sources and influence cutoff

- [ ] **Files:** `gravity.rs`. Test `crates/physics/tests/gravity.rs`.
- **Behavior:** A source attached to a body uses that body's wide position. Finite-influence sources live in a sphere index. The query visits containing spheres only. A source that moves rebuilds its sphere. An asleep body whose new sample differs by more than `0.05` m/s² wakes.
- **Test:** `a_ship_source_pulls_from_its_new_position_and_distant_sources_are_skipped`
  - A dynamic ship with an attached source moves 30 m. A test mass 10 m from the new position accelerates toward the new position, not the start.
  - 1 000 sources on a grid, influence radius 1 m, one of them containing the sample point. The evaluation count is at most 8. The acceleration matches that one source.
  - A sleeper inside a source that then moves closer wakes on the next step.
- **Commit:** `physics: attach gravity sources and cut off by influence`

### P49. Barnes-Hut for unbounded sources

- [ ] **Files:** `gravity.rs`. Test `crates/physics/tests/gravity.rs`.
- **Behavior:** More than 32 sources with infinite influence build an octree. `theta = 0.5`. A cell is walked into when `size / distance > theta`. Otherwise it contributes its monopole. Finite-influence sources stay on the sphere query. The tree is skipped when every unbounded source is asleep and unmoved.
- **Test:** `a_distant_cluster_matches_the_direct_sum_within_one_percent`
  - 64 sources of `GM = 1` packed in a 10 m cube, sample point 1 000 m away. Barnes-Hut acceleration is within 1% of the direct sum. A second build matches bitwise. With 32 or fewer unbounded sources the direct sum is used (`tree_builds == 0`).
- **Commit:** `physics: approximate distant gravity with barnes-hut`

### P50. Volume overrides

- [ ] **Files:** `gravity.rs`. Test `crates/physics/tests/gravity.rs`.
- **Behavior:** Zero, uniform, and spin volumes. Innermost volume replaces the field. Spin acceleration is `ω² * r_axis` away from the axis. Attached volumes move with their body.
- **Test:** `overrides_replace_the_field`
  - Inside a zero-g box next to the 100 m planet, `|a| < 1e-4`. Just outside that box, `|a|` is the planet's inverse-square value within `1e-3`.
  - A uniform override `(0, -9.81, 0)` in empty space samples that vector inside and `0` outside.
  - Spin: axis `+Y`, `ω = sqrt(9.81 / 20)`, point `(20, 0, 0)`. Acceleration is `+X * 9.81` within `1e-3`. Local up is `-X` (toward the axis).
- **Commit:** `physics: override gravity inside a volume`

### P51. Local up for the view body

- [ ] **Files:** `gravity.rs`, `character.rs`, `crates/scene/src/camera.rs`, `crates/anim` foot cast direction. Test `crates/physics/tests/gravity.rs` and `crates/scene/tests/gravity_walk.rs`.
- **Behavior:** The view body reads `local_up` each step. The wish from the character-controller `move` axis lies in the tangent plane. The capsule aligns to local up. Foot casts use `-local_up`. Uniform `GRAVITY` still reports up `+Y`. The room tests stay on that path.
- **Test:** `a_walker_stands_on_the_side_of_a_small_planet`
  - Planet radius 100 m, `GM = 98100`, uniform field `0`, static sphere collider. View capsule placed at `(100, 0, 0)` plus its radius. After 2 s with a zero wish, the feet stay within `0.05` m of the surface and `up · radial > cos(5°)`.
  - A forward wish in the tangent plane moves the capsule along the surface. Radius stays within `0.05` m. World `+Y` is not the up axis (`up · Y < 0.2`).
  - The same scene's room-style uniform field, no sources, still has `up = +Y` and the existing eye-height assertion in `crates/scene/tests/camera_controls.rs` passes.
- **Commit:** `physics: align the view body to local gravity`

### P52. Wide Verlet and the slingshot

- [ ] **Files:** `gravity.rs`, `vehicle.rs` (P32 helper calls the field). Test `crates/physics/tests/gravity.rs`. Re-run `a_circular_orbit_at_1e7_metres_keeps_its_radius`.
- **Behavior:** Free bodies use velocity Verlet in `f64`. Contacting bodies receive the sampled acceleration as their solver gravity. Origin shift still rebases the `f32` pose.
- **Test:** `a_slingshot_gains_speed_and_the_wide_orbit_stays_round`
  - Kinematic moon, `GM = 10`, velocity `(1, 0, 0)`, influence 500 m. Probe enters on a trailing flyby with `v_inf = 1` in the moon frame. After it leaves the sphere, its world speed exceeds its entry speed by at least `0.2` m/s. A second run matches bitwise.
  - The P32 circular orbit at `1e7` m, now a source plus a free body, still stays within `0.05` m of that radius over 200 steps of 1 s.
- **Commit:** `physics: integrate free bodies with velocity verlet`

### P53. Replicate gravity sources

- [ ] **Files:** `replicate.rs`, `gravity.rs`. Test `crates/physics/tests/gravity.rs`. Networking lines in `docs/systems/physics.md`.
- **Behavior:** A `Uniform` world writes its gravity vector and no source block. `replicate_sources` on that world is not called. On a `Field` world it writes id, `GM` bits, clamp, influence, and either an attachment `NetId` or a wide position. Scene-script volumes are omitted. An attached override (spin or uniform on a ship) writes its kind and parameters. The client evaluates the field locally. Rollback samples sources from the snapshot at that tick. The checksum covers the source records only in `Field`.
- **Test:** `a_client_matches_the_server_field_from_replicated_sources`
  - A `Uniform` server snapshot contains the gravity vector and zero source bytes. The client stays in `Uniform` and `field_evaluations` stays 0.
  - A `Field` server with one source and one attached spin volume. Client applies the records and no other gravity input. `gravity_at` matches bitwise at three sample points. A scene-script zero-g box is absent from the byte list. Changing `GM` changes the checksum. Replay at tick `T` uses the source position stored for `T`, not the position at `T+5`.
- **Commit:** `physics: replicate gravity sources`

### P54. Gravity scene and field view

- [ ] **Files:** `crates/scenes` scene `gravity_well`. `examples/physics-view` view mode `gravity`, knob `gravity_scale`, script `examples/physics-view/scripts/gravity_well.rhai`. `genos-character-view` lists the scene. Document the view in `docs/systems/debugging.md`.
- **Behavior:** The scene builds the three lanes from the gravity section. Metrics: `orbit_radius_error`, `slingshot_speed_gain`, `surface_up_dot`, `zero_g_accel`, `passed`. View `gravity` draws the arrow grid, influence wires, and override volumes. The script sets that view, shots `gravity-field`, and `require`s `passed`.
- **Test:** `gravity_well_metrics_cover_orbit_slingshot_and_the_walk` in `crates/scenes/tests/catalog.rs`.
  - After the lane durations above, `orbit_radius_error < 0.05` at the scene's orbit radius (`1e4` m, not the `1e7` m P32 case), `slingshot_speed_gain > 0.2`, `surface_up_dot > cos(5°)`, `zero_g_accel < 1e-4`, `passed == true`.
  - `examples/physics-view/tests/args.rs`: `--scene gravity_well --view gravity` selects that mode.
- **Commit:** `physics-view: show the gravity field`

## GPU solver gate

Do not implement a GPU rigid solver in P1–P54.

Open that work only after all of these are true:

1. P26's 4 096 active spheres, in release, miss the frame budget the game is using, with asleep bodies already excluded from the solver.
2. The pose buffer from P25 is the residency plan: the device keeps the last poses, and the CPU uploads the dirty range only.
3. CCD, queries, vehicles, bonds, and the view body stay on the CPU. PhysX's GPU path leaves CCD and queries on the CPU; this plan does the same.
4. The test, when the work exists, is `gpu_active_set_matches_the_cpu_hash` on `pile_4k` for 30 steps. A mismatch stops the feature.

Until that profile exists, the Vulkan use in this plan is the debug picture and the awake pose buffer.

## Debug scenes the binary must show

| Scene | What a person sees | Pass rule in the metrics |
| --- | --- | --- |
| `pile_4k` | A field of spheres falling onto a plane. | Finite poses. Pair tests under `4096 * 8` on the first step. |
| `asleep_grid` | A static field and a few boxes. The metrics counter `solver_bodies` falls to 0. | `solver_bodies == 0` after the sleep time. |
| `foot_slope` | A leg pair on a ramp. | Ankle penetration `>= -1e-3`, bone lengths, foot up aligned with the hit normal. |
| `car_flat` | A car that pulls away. | Chassis speed &gt; 1 m/s after 2 s of throttle. |
| `car_bump` | One side on a step. | Roll with the anti-roll bar under half the roll without it. |
| `orbit` | A ship with the origin on the hull, gravity off. | Radius error &lt; 0.05 m at `1e7` m. |
| `gravity_well` | Orbits, a slingshot, a walk around a small planet. View `gravity` draws the field, the influence spheres, and the override volumes. | Orbit radius error &lt; 0.05 m, slingshot speed gain &gt; 0.2 m/s, surface up aligned, zero-g volume near 0. |
| `ship_impact` | Four head-on lanes at 30, 120, 600, and 3 000 m/s. | `tunneled == false` and `contacted == true` on every lane. |
| `destruction_wall` | Two pre-fractured columns. A ball hits the left. The right loses the same bond from health. | Both columns break that bond, only the hit branch spawns, debris ≤ budget. |
| `ship_station` | A swept ship hits a supported wall. | No tunnel, at least one bond breaks. |

Which binary opens which scene is the observability table in the networking section. `key_r` and the `reset` command return any of these to the start pose, including bond health, gear, and the prediction ring. `--shot` is the picture an agent attaches. `--metrics` and the script report are the pass or fail an agent reads without a GPU. The session binary from the core-networking plan is where latency, loss, jitter, and the server-versus-client error are shown.

## Records each step touches

| Step | Record |
| --- | --- |
| P15 | `docs/systems/physics.md`, `docs/systems/scene.md` |
| P16 | `docs/systems/anim.md` (new), `docs/systems/README.md`, `CONTEXT.md` (foot plant) |
| P21 | `docs/systems/math.md` (one sentence) |
| P27–P32 | `docs/systems/physics.md` vehicle game use, `CONTEXT.md` (vehicle) |
| P33 | `docs/systems/fracture.md` (new), `docs/systems/README.md` |
| P36–P39 | `docs/systems/physics.md`, `docs/systems/fracture.md`, `CONTEXT.md` (bond, fracture asset) |
| P40–P41 | `docs/architecture.md`, `docs/systems/debugging.md` |
| P42–P45 | `docs/systems/physics.md` networking lines, `docs/systems/fracture.md` for P45 |
| P46 | `docs/systems/debugging.md` |
| P47–P53 | `docs/systems/physics.md` gravity game use and networking lines, `CONTEXT.md` (gravity source) at P47 |
| P54 | `docs/systems/debugging.md` |

## Coverage

| Requirement | Step |
| --- | --- |
| Layered broadphase, batch insert, refit | P4, P5, P6 |
| GJK, EPA, manifolds, warm start | P7, P8, P9 |
| Soft step, sub-steps, joints, sequential fallback | P10, P11 |
| Islands, sleep, wake, work gated by the awake set | P12, P25, P26 |
| Coloring and multithreading, deterministic against the one-thread color order | P22, P23, P24 |
| Speculative CCD, swept AABBs, two-body time of impact, fast-pair sub-steps, head-on ships | P17, P18, P19, P20, `ship_impact` |
| In-process determinism | P14, and every later hash test |
| View body, character-controller action map left as the action map | P15 |
| Foot plant: cast, two-bone IK, pelvis, slope, linked to the view body | P16, `foot_slope` |
| Wheeled vehicle: cast wheels, suspension, slip, Pacejka, torque curve, gearbox, differential, anti-roll | P27–P31, `car_flat`, `car_bump` |
| Thrusters, zero gravity, large-orbit precision | P32, `orbit` |
| Thousands of bodies, origin shift, awake-only poses, GPU solver only after a profile | P21, P25, P26, GPU gate |
| Voronoi pre-fracture, interior faces, hierarchy, bonds, hot-swap, one damage API, debris cull, optional slice | P33–P39, `destruction_wall` |
| Shared scene library, picker, reset, screenshot, metrics | P40, P41 |
| Server authority, quantized snapshots, origin on the wire | P42 |
| Owned-body prediction and rollback; other bodies stay interpolated | P43 |
| Vehicle snapshot without the contact cache | P44, `car_flat` |
| Pre-fracture seeds plus bond-break events, no mesh on the wire | P45, `destruction_wall` |
| One debug binary per system, lighting-style commands, scripts, knobs, reset | P41, P46 |
| Per-world gravity mode. Default `Uniform` is a static vector and pays nothing for wells. `Field` is opt-in and may mix that vector with wells and volumes | P47 |
| Inverse-square, clamp, sphere of influence | P47, P48 |
| Zero-g, artificial gravity, spin gravity | P50 |
| View body and feet align to local up | P51, `gravity_well` |
| Velocity Verlet, wide orbits, slingshot | P32, P52, `orbit`, `gravity_well` |
| Many sources: influence cutoff, Barnes-Hut past 32 unbounded sources | P48, P49 |
| Deterministic gravity sources on the wire | P53 |
| Gravity-field view in the physics debug binary | P54 |
