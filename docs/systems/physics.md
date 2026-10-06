# Physics

The physics step moves bodies. It applies gravity, contact, and springs.

## Goal

A game can simulate a falling mass, a bounce, a hit between two masses, friction, and one spring.

## Intent

Gravity is `GRAVITY`, along -Y. There is no air drag. `step` takes a world and a time step and returns the next world. The call does not allocate.

Contact restitution is the larger value on the two bodies. Friction is the larger value too. A spring pushes while the body is closer than `rest_length`. It does not pull. Rest compression on a vertical spring is `mass * -GRAVITY.y / stiffness`.

## Code

The crate is `crates/physics`, package `genos-physics`.

- `Body`, `World`, `Spring`, `Shape`, and `step` are in `src/lib.rs`.
- `GRAVITY` is `(0, -9.81, 0)`.
- `MAX_BODIES` is 32.

## Game use

Create a world with `World::new(GRAVITY)`. Insert each body with `World::insert`. Call `step(&world, dt)` and keep the returned world. Read `position`, `orientation`, and `velocity` on `bodies[..count]`.

## Limits

The step does not turn a body. Orientation is stored, and the step leaves it unchanged. Shapes are a sphere and a static plane. One spring is the only joint.

There is no stack solver and no air drag. A large `dt` drifts from `½gt²`. The camera does not call this step. The camera still walks through walls.

## Decisions

- [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md) keeps this step in the repository.
