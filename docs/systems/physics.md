# Physics

The physics step moves bodies. It applies gravity, a move wish, contact, and springs.

## Goal

A game can simulate a falling mass, a bounce, a hit between two masses, friction, and one spring. A capsule, a box, and a simplified mesh take part in that hit.

## Intent

Gravity is `GRAVITY`, along -Y. There is no air drag. `step` takes a world and a time step and returns the next world. The call does not allocate.

When `motor` is set, the step copies `wish.x` and `wish.z` into the velocity before contact. Contact then removes the part of that wish that points into a surface.

Contact restitution is the larger value on the two bodies. Friction is the larger value too. A spring pushes while the body is closer than `rest_length`. It does not pull. Rest compression on a vertical spring is `mass * -GRAVITY.y / stiffness`.

A capsule is a segment along local Y plus a radius. `half_height` is the segment half-length. A box uses `half_extents` on the local axes. `orientation` aims the capsule segment and the box. The step does not change `orientation`.

`Shape::from_triangles` builds a mesh collider before the step. A convex mesh becomes one hull of at most 12 vertices. A concave mesh is split into at most 6 boxes, so a hole in the source stays open. Contact tests those pieces. The pieces stay on the occupied volume.

## Code

The crate is `crates/physics`, package `genos-physics`.

- `Body`, `World`, `Spring`, `Shape`, and `step` are in `src/lib.rs`.
- Contact is in `src/contact.rs`.
- Mesh build is in `src/mesh.rs`.
- `GRAVITY` is `(0, -9.81, 0)`.
- `MAX_BODIES` is 32.
- `MAX_HULL_VERTS` is 12. `MAX_MESH_PIECES` is 6.

## Game use

Create a world with `World::new(GRAVITY)`. Insert each body with `World::insert`. Call `step(&world, dt)` and keep the returned world. Read `position`, `orientation`, and `velocity` on `bodies[..count]`.

For a character, set `motor` and write the desired horizontal velocity in `wish`. Use `Body::capsule`, `Body::cuboid`, or `Body::mesh`. Build a mesh with `Shape::from_triangles` or `Body::mesh`. `contact_faces` reports the reduced face count.

## Limits

The step does not turn a body. There is no stack solver, no air drag, and no continuous collision. A large `dt` drifts from `½gt²`.

A mesh collider is a few convex pieces, not the source triangles. Two concave meshes do not get a triangle-versus-triangle test. The piece count stops at 6. A very thin sheet can be missed by the voxel sample.

Lights have no collider.

## Decisions

- [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md) keeps this step in the repository. There is no physics middleware crate.
