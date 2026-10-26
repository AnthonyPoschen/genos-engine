# Continuous collision and destruction

Status: **proposed**. ADR: [0016](../adr/0016-physics-ccd-destruction.md). Source: the physics plan of 12 Oct 2026, [`physics/plan.md`](physics/plan.md). Terms for lighting stay in [`docs/lighting/GLOSSARY.md`](../lighting/GLOSSARY.md). The physics words in the glossary below are proposed. The commit that first publishes one in code adds it to `CONTEXT.md`.

This design does not change `step`. The crate today integrates `velocity * dt` and resolves an overlap that already exists (`crates/physics/src/lib.rs`, `crates/physics/src/contact.rs`). Orientation does not integrate. A mesh collider is at most six boxes or one hull of 12 vertices (`MAX_MESH_PIECES`, `MAX_HULL_VERTS`).

## Goals

- Two convex ships on a head-on course at 30, 120, 600, and 3 000 m/s each take one `1/60` s step and stay in contact instead of exchanging places.
- A modest bullet, travel under the speculative margin, stops in that same step without a time-of-impact loop.
- An authored breakable starts as one body. The cook stores a Voronoi chunk hierarchy and a bond graph. At runtime the step swaps in the child bodies of the chunk that broke.
- A bond breaks from contact impulse and from a game event through one function. Health reaching 0 is that event.
- A dedicated server and a client that already has the `.gfract` file agree on the chunks and on which bonds are broken.
- A break updates GI through the local change records of [ADR 0013](../adr/0013-lighting-invalidation-is-local.md). The cook's per-chunk cards and SDFs are reused.

## Non-goals

- Replacing the current discrete step for bodies that stay `Discrete`. The room camera, the stack tests, and `free_fall_matches_half_gt_squared_and_gt` keep their results.
- A GPU solver, cloth, fluids, or a deforming mesh.
- Runtime fracture that builds a new pattern from the impact point. One optional plane slice exists for a game that asks. `step` does not call it.
- Predicting destruction on the client. The owned view body remains the only predicted body in the physics plan.
- Cross-platform bit identity of `f32`. The gate is a same-process hash, then quantized poses on the wire.
- Editing the lighting schedule, the bake, or `step` in this change.

## Solver

The step has to grow a swept pair and a bond break without a second physics library.

| | Rapier (Dimforge) | Jolt (Rouwe) | **In-repo solver** |
|---|---|---|---|
| Where it runs | Rust crate `rapier3d` | C++ library through a binding | `crates/physics`, called by `step` |
| CCD it ships | Soft CCD (a predictive margin) and nonlinear CCD with `max_ccd_substeps` ([CCD](https://rapier.rs/docs/user_guides/rust/rigid_body_ccd/)) | `EMotionQuality::LinearCast`: sweep, then time stealing ([docs](https://jrouwe.github.io/JoltPhysicsDocs/5.3.0/)) | `Speculative` plus conservative advancement on `Swept` pairs |
| Destruction | None in the solver | None in the solver | Bond graph on the same islands |
| Determinism | Its own body order and `f32` math | Its own job order | Body order and contact order are ours, hashed in-process |
| ADR 0007 | Pulls Parry and nalgebra and owns the step | The step would live in a third-party library | The step stays in this repository |

**Pick: the in-repo solver.** Rapier is the closest Rust library and already splits a cheap margin from a swept sub-step. Jolt is the reference for `BodyId`, object layers, and a motion-quality enum, and its linear cast is the shipped form of "sweep, then stop". Both are rejected as dependencies by [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md) and by the physics plan. The contact code we have (`solve_contact`, one normal impulse, friction, position correction) remains the sequential-impulse fallback. The soft constraint (frequency and damping) from Erin Catto, "Soft Constraints", GDC 2011 ([notes](https://box2d.org/files/ErinCatto_SoftConstraints_GDC2011.pdf)) is the solver the physics plan's P10 adds. This design uses that solver. It does not add a second one.

## Continuous collision

### Alternatives

1. **Speculative contacts only.** PhysX inflates the contact offset by the motion and lets the discrete solver stop the body (`PxRigidBodyFlag::eENABLE_SPECULATIVE_CCD`, [PhysX advanced collision](https://nvidia-omniverse.github.io/PhysX/physx/latest/docs/AdvancedCollisionDetection.html)). Bepu v2 does the same: the margin is the speed, clamped by a minimum and a maximum, and the default maximum is unbounded ([Bepu CCD](https://docs.bepuphysics.com/ContinuousCollisionDetection.html)). One extra margin, no root find. At 3 000 m/s the uncapped margin is 50 m, so the ships collide in empty space. A cap small enough to look right (0.25 m) does not cover a ship.
2. **Time of impact, then sub-step that pair.** Conservative advancement (Brian Mirtich, *Impulse-based Dynamic Simulation of Rigid Body Systems*, PhD thesis, UC Berkeley, 1996; the game account is Erin Catto, "Continuous Collision", GDC 2013, [slides](https://box2d.org/files/ErinCatto_ContinuousCollision_GDC2013.pdf)) walks the closest-point distance against the relative velocity of those points, including rotation. Bullet uses that walk for convex versus convex. Rapier's nonlinear CCD and Bepu's `Continuous` mode are the same family: find a time, advance, repeat. Jolt's linear cast is the cheaper sibling and, by its own note, misses a fast spin because it sweeps the start rotation.
3. **Sub-step the whole world** until every body's travel is under its radius. Box2D moves the simulation to the first time of impact ([simulation](https://github.com/erincatto/box2d/blob/main/docs/simulation.md)). Correct, and one fast pair re-simulates every awake body. The room does not need that.

**Pick: (1) for bullets, (2) for ships, and pair-local slices when a pair outruns its radius.** (3) is the cost we refuse. This is the physics plan's P17–P20.

`MotionQuality::Discrete` uses the current bounds. `Speculative` is the bullet. `Swept` is the ship. A body has one quality. The default is `Discrete`, so a scene that never sets a quality does no swept work.

### Data

```text
MotionQuality   Discrete | Speculative | Swept
speculative margin   default 0.25 m (Medium and High). Low uses 0.05 m. Potato does not read it.
ToiEvent        pair, time (s), normal, point, advances, failed
pair_slices     how many slices that pair took
body_slices     how many slices that body took
toi_failed      count of pairs that made no progress
toi_deferred    Swept pairs treated as Speculative because the preset pair cap was full
```

Swept bounds are the start AABB expanded by linear travel over the step and by a bounding-sphere arc `radius * angle` for angular travel. Discrete bodies contribute the current AABB. The broadphase pairs those bounds. Planes stay unbounded.

### Step, inside one 1/60 s tick

Sub-steps divide the tick. They do not rebuild the broadphase. The global sub-step count stays the solver's (4 when the physics plan's soft step has landed). CCD does not add a minimum step count for an asleep world.

1. Forces, then the broadphase on swept bounds.
2. `Discrete` and `Speculative` bodies take the global sub-steps. A speculative contact distance is `min(speed * dt_sub, speculative margin)`. The solver stops the approach. No time-of-impact loop.
3. Each `Swept` pair, and only that pair: GJK distance, closing speed from the relative velocity of the closest points (both linear velocities and both angular velocities). Time of impact is distance divided by closing speed when the points are approaching. Advance `0.8` of the remaining time, and at least `1e-4` s, solve the contact, and repeat until the tick is consumed or 16 advances have run. Other bodies stay on the global sub-step. No progress sets `toi_failed` for that pair and leaves the last safe pose.
4. If a pair's relative travel over the tick exceeds the smaller bounding radius, that pair integrates in slices whose travel is at most that radius. The rest of the world keeps the global sub-step count.
5. A `Swept` pair past the preset's pair cap is integrated as `Speculative` for that tick and increments `toi_deferred`.

A missed root counts as a tunnel in `tunnel_count`. The debug binary still shows `toi_failed` on its own.

## Destruction

### Alternatives

1. **Offline pre-fracture and a bond graph.** The usual game method, and the one Müller, Chentanez, and Kim criticise for ignoring the impact point (SIGGRAPH 2013, [paper](https://matthias-research.github.io/pages/publications/fractureSG2013.pdf), DOI `10.1145/2461912.2461934`). NVIDIA Blast and Unreal Chaos ship it: a chunk hierarchy, a connection for each shared face, a support link to the world, and a health value that removes the connection ([Blast](https://developer.nvidia.com/blast), [Chaos Destruction](https://dev.epicgames.com/documentation/en-us/unreal-engine/chaos-destruction-overview)). Both peers can load one file. The spawn message is a content hash when the client already has the file.
2. **Runtime patterns at the impact.** The same Müller paper represents the mesh as a volumetric approximate convex decomposition and applies a user pattern where the hit landed, then rebuilds islands. The chunks depend on the hit. A content hash of one `.gfract` cannot name them, and the server would ship either the hit plus a non-deterministic clip or the generated vertices.
3. **Finite elements.** James F. O'Brien and Jessica K. Hodgins, "Graphical Modeling and Animation of Brittle Fracture", SIGGRAPH 1999, and the game-time version by Eric G. Parker and James F. O'Brien, SCA 2009. The crack follows the stress. The server tick would carry a tetrahedral mesh, and the contact code would no longer be a convex test.

**Pick: (1).** The server has to replay one asset. The cook writes the pattern once. The step only swaps and simulates. Müller's support-island idea is the part we keep (a chunk that can still walk bonds to a support stays static). The impact-aligned pattern is the part we leave. A game that needs a cut calls `slice_chunk` with a plane. That call is replicated as the plane, and both peers clip. `allow_slice` defaults to false, and `step` never calls it.

The cell itself is a Voronoi cell: the source hull clipped by perpendicular-bisector planes in increasing seed index. That is the physics plan's P33. It is a simpler pattern than Müller's approximate convex decomposition, and it is computed offline. Seed order is the cell order. There is no hashmap iteration and no `atan2` on this path.

### Data

`.gfract` version 1, little-endian `f32`, as the physics plan and the [asset plan](assets/plan.md) already name it.

```text
Chunk     parent, first_child, child_count, convex pieces, volume
Bond      chunk A, chunk B or WORLD (u32::MAX), area, centroid, normal, health, health_rest
Breakable intact BodyId, asset hash, chunks, bonds, spawned bodies, allow_slice, debris_budget
BondBreak breakable, bond, amount_milli, tick
```

`health = health_per_area * area` at cook time. A support chunk also has a bond whose other end is `WORLD`. Level 0 is the coarse seeds. Children sit on a fixed grid inside the parent cell. The asset stores the parent and the child range.

The SHA-256 of the version-1 payload is the sim content hash. SDF cards and texture blocks stay on the render subset, so a BC7 cook and an ASTC cook share one physics hash.

### Step, after the contact solve

1. A breakable starts as one dynamic or static body. Chunks are not bodies.
2. The solver sums the normal impulse of a contact whose two bodies are chunks that share a bond, and calls `apply_damage(breakable, bond, impulse)`.
3. A game call uses the same function. Health reaching 0 is `apply_damage` of the remaining health. The amount is damage in milli-units on the wire. The receiver does not care which caller crossed zero.
4. At health `<= 0`, remove the bond and its fixed joint, wake both islands, and `activate` the chunks that edge held.
5. `activate` removes that chunk's body and spawns its children with the parent's linear and angular velocity. Each remaining child bond becomes a fixed joint. A spawned chunk that can still walk bonds to a support stays static. A component with no support becomes dynamic. Chunks outside the damaged neighbour set stay unspawned. Unbroken coarse chunks stay the parent body.
6. A spawned chunk whose volume is under `debris_volume` (default `0.02` m³) is broadphase layer 2 (debris). It sleeps at `0.1` s and `0.4` m/s. Debris does not pair with debris. Past `debris_budget`, remove the smallest asleep debris. Awake debris stays. Removing asleep debris does not wake the parent island.

## CPU milliseconds per preset

These are ceilings for the physics step on the owner's machine. They are not a measurement. This change records no times. The harness metric stays `NOT-IMPLEMENTED` until the workload exists and the owner's PC records it. Lavapipe and a cloud VM do not supply the number. A ceiling is a cap: an asleep island adds no narrowphase and no solver work, and the frame does not fill the cap on purpose.

One time-of-impact advance is budgeted at 3 µs (one closest-point query plus one contact solve). One awake debris body is budgeted at 10 µs of solver work. Those unit costs are the arithmetic behind the ceilings, not a profile.

| Preset | What the step may do | Pair cap | Advances per pair | Debris budget | Speculative margin | Ceiling |
|---|---|---|---|---|---|---|
| Potato | Discrete only. An intact breakable is one body. | 0 | 0 | 0 | unused | 0.2 ms |
| Low | Speculative bullets. No time-of-impact loop. | 0 | 0 | 0 | 0.05 m | 0.5 ms |
| Medium | Two swept pairs, one broken column. | 2 | 8 | 8 | 0.25 m | 1.5 ms |
| High | `ship_impact` (four lanes) and `destruction_wall`. | 8 | 16 | 32 | 0.25 m | 3 ms |

High arithmetic: `8 × 16 × 3 µs = 0.384 ms` of advances, plus `32 × 10 µs = 0.320 ms` of debris. The 3 ms ceiling covers that plus the broadphase, the manifolds, and the islands. Medium is `2 × 8 × 3 µs` plus eight debris bodies inside 1.5 ms. A `Swept` pair past the pair cap increments `toi_deferred` and uses the speculative margin for that tick.

`ship_impact` on High is the workload for `step_ms_high`: four lanes, speeds 30, 120, 600, and 3 000 m/s, one step of `1/60` s. Each ship is a box, half-extents `(2, 1, 1)` m, centres 40 m apart. At 3 000 m/s each ship travels 50 m, which is longer than the 4 m hull.

## Success metrics

Each metric has one threshold. The comparator is inclusive. A non-finite reading fails. The live scene prints `NOT-IMPLEMENTED` until the step can produce the reading. A missing feature does not get a stand-in number.

`energy_drift = (E_end - E_start) / max(|E_start|, 1)`. `E` is kinetic energy plus `m * 9.81 * position.y`. Bonds store no elastic energy. A gain above the threshold fails. A loss passes, because a contact may be inelastic.

`penetration_m` is the minimum signed separation on the ship lanes, in metres. Negative is an overlap.

`bonds_broken` is the minimum broken-bond count of the two `destruction_wall` columns.

`contacts_stored` is 1 when every ship lane stored a contact impulse, and 0 otherwise.

`floor_penetration_m` is the deepest distance a chunk centre sits below the floor past its own half-extent. 0 means none.

`tunnel_count` counts a lane whose centres exchanged order along the closing axis, a lane whose separation went past −0.01 m, or a lane with `toi_failed`.

`determinism_hash_match` is 1 when a second run in the same process hashes equal, and 0 otherwise. The hash is the physics plan's state hash, over the same spawns, the same `.gfract` bytes, and `step` at `1/60`.

| Metric | Threshold | Scene |
|---|---|---|
| `tunnel_count` | `<=0` | `ship_impact` |
| `penetration_m` | `>=-0.01` | `ship_impact` |
| `energy_drift` | `<=0.01` | `ship_impact` |
| `determinism_hash_match` | `>=1` | both |
| `contacts_stored` | `>=1` | `ship_impact` |
| `bonds_broken` | `>=1` | `destruction_wall` |
| `debris_alive` | `<=8` | `destruction_wall` |
| `floor_penetration_m` | `<=0` | `destruction_wall` |
| `toi_deferred` | `<=0` | `ship_impact` on High |
| `step_ms_potato` | `<=0.2` | Potato workload |
| `step_ms_low` | `<=0.5` | Low workload |
| `step_ms_medium` | `<=1.5` | Medium workload |
| `step_ms_high` | `<=3` | High workload |

## Test plan

Two scenes, both deterministic, both with a programmatic pass and fail.

`ship_impact` is four lanes of the swept boxes above, one `1/60` s step (the catalog runs five steps and requires the same gates). Pass: `tunnel_count`, `penetration_m`, `contacts_stored`, `toi_deferred`, `energy_drift`, and `determinism_hash_match` inside the table. The spinning companion from the physics plan (box half-extent 1 m, angular velocity 40 rad/s about Z, 30 m/s into a wall 2 m away) is a second check in the same scene file: the box bounds stay on the approach side of the wall.

`destruction_wall` is two columns of the coarse-and-fine asset. The left column is struck by a sphere. The right column takes `apply_damage` of the matching bond at frame 30, with no extra body. `debris_budget` is 8. After 90 steps: `bonds_broken`, `debris_alive`, and `floor_penetration_m` inside the table, and fine chunks exist only on the struck coarse chunk of each column. The two columns do not have to hash equal. The sphere is extra mass. The bond id that breaks is the same bond in both columns.

The scenes will live in `debug/scenes/ship_impact.rhai` and `debug/scenes/destruction_wall.rhai` when `debug/physics` exists. The script calls `step`, reads `physics_metrics`, and uses `check` / `require`. Exit code 1 when a check fails. `--headless --scene NAME --frames N --metrics PATH` is the same gate without a window. `--out` sets the directory. The default is `./genos-debug/physics/`. `GENOS_PHYSICS_METRICS_OUT` is the mirror and applies only when `--out` was omitted.

### Harness that exists now

CCD and the bond graph are not in `step`. The harness is test code, `crates/physics/tests/metrics.rs`, and it does not add a solver.

```text
cargo test -p genos-physics --test metrics
cargo test -p genos-physics --test metrics -- --fixture good
cargo test -p genos-physics --test metrics -- --fixture bad
cargo test -p genos-physics --test metrics -- --fixture live
```

`--fixture` defaults to `proof`. `GENOS_PHYSICS_METRICS_FIXTURE` is the mirror and applies only when `--fixture` was omitted. The argument wins when both are set.

Each run prints one line per metric: `name value threshold status`. It writes `./genos-debug/physics/<fixture>.json`. `cargo test` starts in the package directory; the default still writes to the repository `genos-debug/physics`. `--out` and `GENOS_PHYSICS_METRICS_OUT` stay relative to the process directory. Exit 0 when no line is `FAIL`. Exit 1 when a line is `FAIL`. Exit 2 on a bad argument or a bad script. `NOT-IMPLEMENTED` is not a failure.

`good.rhai` feeds a reading inside every threshold. `bad.rhai` feeds a reading outside every threshold. `live.rhai` marks every metric pending. `proof` runs all three and exits 0 only when good exits 0, bad exits 1, and live exits 0 with every line `NOT-IMPLEMENTED`. The scripts cannot change a threshold. The table above is the gate. Phase 0 reads `reading`, `pending`, and full-line `//` comments from those files. It does not embed the `rhai` crate. The debug binary's Rhai host, the one [ADR 0009](../adr/0009-rhai-is-the-scripting-language.md) allows, runs the same calls when the scenes move.

The debug menu, when the physics binary exists, has no help text. A knob's label is the word itself (`speculative margin`, `debris budget`, `bond health`). The menu does not explain the word.

## GI and the asset pipeline

The cook, not the physics step, builds the chunks ([asset plan](assets/plan.md)). Each chunk is a mesh with its own bake key, so the SDF and the cards exist per chunk before the first break. The intact body draws the source mesh and uses the source bake. A spawned chunk draws that chunk's mesh and uses that chunk's bake key. The hot-swap does not cook and does not rebake.

The sim subset is the `.gfract` payload: chunk bounds, convex pieces, the hierarchy, and the bonds. The render subset is the meshlets, the texture blocks, and the bake keys. The headless server loads the sim subset and creates no Vulkan device. A client loads both.

Lighting already has the words this needs. A spawned chunk is a mover. Its cache entries are **object-local entries**. What it invalidates is a **change record**: the old bounds and the new bounds. No new lighting term is added.

On the hot-swap tick the client emits, in `(breakable, chunk)` order:

- one change record for the removed body, old bounds to empty;
- one change record per spawned child, empty to the child bounds.

An awake chunk emits a change record on a later tick only when its bounds move. The sleep tick emits one record and then nothing while the chunk stays asleep. A culled debris chunk emits one removal. The surface cache drops that transform's object-local entries when it compacts.

Interior faces are in the chunk mesh. Their cards stay in the bake and are not resident while the parent is intact, because the chunk is not drawn. The first frame the chunk is visible takes the ordinary pre-fill. The spawn cap of ADR 0013 still applies. Entries that do not fit wait. The pixel reads a finished coarser value. There is no full-frame GI rebuild on a break.

The server does not build change records. The client builds them after it has applied `BondBreak` events and then the poses for that tick. Destruction is not predicted, so a rollback of the view body does not re-break a bond.

## Networking

Gameplay networking still waits. These lines are the contract.

- **Replicated state:** the `.gfract` content hash when the client has the file, otherwise the seeds, child counts, interior material, support flags, and spawn pose. Then `BondBreak { breakable, bond, amount_milli, tick }` in tick order. Awake chunk poses use the same quantized body record as any dynamic body. A slice, when allowed, replicates the plane. Asleep debris and the generated vertices stay local.
- **Authority:** the server runs `step` and is the only side that decides a bond broke. Clients apply the server's `BondBreak` through `apply_damage`. They do not accept one another's breaks.
- **Prediction / rollback:** none for a breakable or a chunk. The owned view body stays the only predicted body. A client that sees a chunk pose before it has spawned that chunk applies `break_baseline` first.
- **Determinism:** one process, `step` on one thread, fixed `1/60` s, the same `.gfract` bytes, seed order, and bond order. `step_parallel` may run on the server only after it hash-matches `step`. The wire compares quantized poses and bond ids.
- **Headless server:** `step` and the sim subset. No window, no Vulkan device, no GI, no change records.

## Phased rollout

| Phase | What lands | Gate |
|---|---|---|
| 0 | This design and the metrics harness | `proof` exits 0. Live lines are `NOT-IMPLEMENTED`. |
| 1 | Swept bounds and speculative contacts | The bullet case in the physics plan. Ship lanes stay `NOT-IMPLEMENTED`. |
| 2 | Time of impact, pair slices, `ship_impact` | `tunnel_count`, `penetration_m`, `contacts_stored`, `toi_deferred`, `determinism_hash_match` become readings. |
| 3 | `genos-fracture` and `.gfract` in the cook | Round-trip bytes. `step` still does not break. |
| 4 | Hot-swap, `apply_damage`, debris | `destruction_wall` readings: `bonds_broken`, `debris_alive`, `floor_penetration_m`. |
| 5 | Change records from chunk bounds, per-chunk bake keys already cooked | A break does not switch on the global stillness hash. Spawn stays inside the ADR 0013 cap. |
| 6 | `BondBreak` on the session, when core networking exists | `bond_breaks_replicate_without_the_mesh`. |

Each phase is its own commit. A phase that misses its metric stays out.

## Open questions

- The scale from a gameplay health point to `amount` on `apply_damage` is unset. Impulse and health have to share one number. The milli-unit on the wire is agreed. The conversion constant is not.
- Culling the smallest asleep debris can remove a chunk the camera can see. The budget requires the cull. Whether a chunk inside the view waits until it is off screen is open. The metric `debris_alive <= 8` still holds either way.
- Sixteen advances are the cap at 3 000 m/s. If `tunnel_count` fails on that lane, the cap moves up in that commit. The cap does not grow silently, and the step does not fall through to a tunnel.
- A swept ship that is also a breakable: the intact body is `Swept`. After the swap, a child whose travel still exceeds its radius stays `Swept` for that tick, and a slow child returns to `Discrete`. That rule is the proposal for `ship_station`. It is not implemented.
- Change-record order is events, then poses, then records. That wants a confirmation in the core-networking plan when that file names the snapshot order. If that file picks a different order, it wins.

## Glossary

One meaning each. Lighting names used above (`change record`, `object-local entry`, `mover`, `pre-fill`, `work cap`) keep the lighting glossary's meaning.

**Motion quality.** How far one body's collision looks ahead in a single physics step: the current bounds, a speculative margin, or a time of impact.

**Speculative contact.** A contact generated inside `min(speed * dt, speculative margin)` so the solver can stop a body before it overlaps, with no time-of-impact solve.

**Time of impact.** The earliest time in the step at which two convex bodies touch.

**Conservative advancement.** Advance a pair by a fraction of the remaining time of impact, solve that contact, and repeat. The fraction stays below 1.

**Bond.** One link in a breakable: two chunks, or a chunk and the world, plus the shared area and the health left.

**Bond graph.** All of the bonds of one fracture asset. Removing a bond disconnects those chunks.

**Breakable.** One authored object that simulates as a single body until a bond breaks, and then swaps in chunk bodies from its fracture asset.

**Chunk.** One piece of a fracture asset. It is not a body until a hot-swap spawns it.

**Hot-swap.** Replacing one chunk's body with that chunk's child bodies in the step that breaks the bond.

**Fracture asset.** The cooked `.gfract` file: the chunk hierarchy, the interior faces, and the bond graph.

**Bond break.** The event that a bond's health reached 0. The cause is a contact impulse or a game value such as health. The event is the same.

**Debris.** A spawned chunk under the debris volume. It sleeps quickly, it does not collide with other debris, and it can be removed when the debris budget is full.

## Departures

- Müller, Chentanez, and Kim, SIGGRAPH 2013, sections on impact patterns and island rebuilds. The paper applies a fracture pattern at the hit and rebuilds connectivity. This design stores one Voronoi pattern in `.gfract` and spawns the precomputed children. Support is kept: a chunk with a bond path to `WORLD` stays static. `slice_chunk` is a game call, and `step` does not slice.
- The Voronoi clip is bisector planes in seed order (physics plan P33). The paper's decomposition is a volumetric approximate convex decomposition, built to accept a runtime pattern. We do not build that decomposition.
- Bepu's default maximum speculative margin is unbounded. Ours is 0.25 m on Medium and High, and 0.05 m on Low. Past that cap the body is `Swept`, not a larger margin.
- Catto, GDC 2013, on conservative advancement needing many iterations when shapes are close. We stop at 16 advances, advance 0.8 of the remaining time, require at least `1e-4` s of progress, and set `toi_failed` plus a tunnel. We do not iterate until the root converges.
- Jolt `LinearCast` sweeps the starting rotation and steals the rest of the tick. We include angular velocity in the closing speed, we sub-step the pair through the rest of the tick, and other bodies keep the global sub-step.
- Box2D sub-steps the simulation to the first impact. We sub-step the fast pair.
- Blast and Chaos ship the bond graph inside their own solvers. The graph is the idea we copy. The SDK stays out ([ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md)).
- O'Brien-style fracture releases elastic energy at the crack. A bond here stores health only. `energy_drift` treats a gain as a solver fault.
- ADR 0013 is followed: a chunk is a mover, and the record is the old and new bounds. A break does not mark the whole cache dirty.
