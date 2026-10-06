# Math

The math library stores positions, directions, and rotations. Movement and the physics step use these values.

## Goal

A game can place a point, aim a direction, and turn that direction. The camera and each placed object store those values.

## Intent

Components are `f32`. Y is up. The ground is the XZ plane. Matrices are column-major. `A * B` applies `B` first.

Quaternion slerp takes the short arc when the dot product is negative. A zero vector normalizes to zero. The values are `Copy`. The operations do not allocate.

## Code

The crate is `crates/math`, package `genos-math`.

- `Vec2`, `Vec3`, and `Vec4` are in `src/vec.rs`.
- `Mat3` and `Mat4` are in `src/mat.rs`.
- `Quat` is in `src/quat.rs`.

## Game use

Build a position with `Vec3::new`. Add a scaled direction to move that position. Build a turn with `Quat::from_axis_angle`. Rotate a direction with `Quat::rotate`. `Mat4::from_quat` and `Mat3::from_quat` build that same turn as a matrix.

The camera stores `Camera.position` as a `Vec3`. `look_direction` returns the view direction. `update` adds a scaled ground direction.

## Limits

There is no `f64` path. This crate does not build the Vulkan projection. The camera owns that matrix. Lighting probes and mesh vertices stay raw floats.

## Decisions

- [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md) keeps engine math in this repository.
