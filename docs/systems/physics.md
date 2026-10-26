# Physics

The physics step moves bodies. It applies gravity, a move wish, contact, and springs.

## Goal

A game can simulate a falling mass, a bounce, a hit between two masses, friction, and one spring. A capsule, a box, and a simplified mesh take part in that hit.

## Intent

Gravity is `GRAVITY`, along -Y. There is no air drag. `step` takes a world and a time step and updates that world in place. The first step may size the contact lists. A later step does not allocate.

Each body has a motion quality. `Discrete` is the default and resolves an overlap that already exists. `Speculative` grows the contact distance by `min(speed * dt, speculative margin)` before the move, and the solver stops that approach. The default margin is 0.25 m. `Swept` is reserved for a time of impact; until that solve lands, a swept body still takes the discrete move.

When `motor` is set, the step copies `wish.x` and `wish.z` into the velocity before contact. Contact then removes the part of that wish that points into a surface.

Contact restitution is the larger value on the two bodies. Friction is the larger value too. A spring pushes while the body is closer than `rest_length`. It does not pull. Rest compression on a vertical spring is `mass * -GRAVITY.y / stiffness`.

A capsule is a segment along local Y plus a radius. `half_height` is the segment half-length. A box uses `half_extents` on the local axes. `orientation` aims the capsule segment and the box. The step does not change `orientation`.

`Shape::from_triangles` builds a mesh collider before the step. A convex mesh becomes one hull of at most 12 vertices. A concave mesh is split into at most 6 boxes, so a hole in the source stays open. Contact tests those pieces. The pieces stay on the occupied volume.

## Code

The crate is `crates/physics`, package `genos-physics`.

- `Body`, `World`, `Spring`, `Shape`, `MotionQuality`, and `step` are in `src/lib.rs`.
- Contact is in `src/contact.rs`.
- Speculative contacts and the GJK gap are in `src/ccd.rs`.
- Mesh build is in `src/mesh.rs`.
- `GRAVITY` is `(0, -9.81, 0)`.
- `World.bodies` is a `Vec`. The crate does not cap the body count.
- `MAX_HULL_VERTS` is 12. `MAX_MESH_PIECES` is 6.

## Game use

Create a world with `World::new(GRAVITY)`. Insert each body with `World::insert`. Call `step(&mut world, dt)`. The world is updated in place. Read `position`, `orientation`, and `velocity` on `bodies[..count]`.

For a character, set `motor` and write the desired horizontal velocity in `wish`. Use `Body::capsule`, `Body::cuboid`, or `Body::mesh`. Build a mesh with `Shape::from_triangles` or `Body::mesh`. `contact_faces` reports the reduced face count.

For a bullet whose gap at the start of the step is inside the margin, set `motion` to `MotionQuality::Speculative`. `World.speculative_margin` is the cap, 0.25 m unless a caller changes it. `step_stats.ccd_applied` is true for that step.

## Limits

The step does not turn a body. There is no stack solver and no air drag. A large `dt` drifts from `½gt²`.

`Discrete` still tunnels when the travel is longer than the body. `Speculative` does not cover a gap larger than the margin: a ship at 3 000 m/s is that miss, and `tunnel_count` on `cargo test -p genos-physics --test metrics -- --fixture good` stays above 0 until time of impact. The margin knob is `World.speculative_margin`. The same harness is the probe: `ccd_in_step` is true only when a body asked for speculative collision.

A mesh collider is a few convex pieces, not the source triangles. Two concave meshes do not get a triangle-versus-triangle test. The piece count stops at 6. A very thin sheet can be missed by the voxel sample.

The camera calls this step from the scene crate. Lights have no collider.

## Plan

The next pipeline is [physics/plan.md](physics/plan.md). That file is a plan. It does not change this step.

Continuous collision and destruction are decided in [physics-ccd-destruction-design.md](physics-ccd-destruction-design.md) and [ADR 0016](../adr/0016-physics-ccd-destruction.md). That decision is accepted. It does not change this step.

## Networking

The step does not replicate. Gameplay networking waits ([Architecture](../architecture.md)). The contract the pipeline will meet:

- Replicated state: awake poses and the sleep bit, vehicle chassis and wheel speeds, pre-fracture seeds (or the asset hash when the file is already local), bond-break events. Gravity sources (`GM`, clamp, influence, attachment) replicate only when that world has opted into the field. A uniform-gravity world sends the static vector and no source block. Scene-script gravity volumes stay local, like walls.
- Authority: the server owns body state. The owning client sends input for the view body.
- Prediction / rollback: the owned view body only. Vehicles, other dynamics, and destruction interpolate.
- Determinism: one process, the fixed `1/60` step, single-thread `step`. Cross-platform bit identity is not required.
- Headless server: `step` needs no window and no Vulkan device.

## Decisions

- [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md) keeps this step in the repository. There is no physics middleware crate.
