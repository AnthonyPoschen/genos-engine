# Math

The math library stores positions, directions, and rotations. Movement and the physics step use these values. It also holds the participating-medium closed forms that fog calls.

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
- Fresnel, Snell, GGX, Beer–Lambert, mirror and portal maps, flat-water transmittance, and the Fresnel mirror-view budget are in `src/optics.rs`. The GLSL mirror is `crates/render/shaders/reflections.glsl`.
- Participating-medium closed forms are in `src/media.rs`: transmittance, height-fog density, Henyey-Greenstein, the homogeneous in-scatter integral, and height-fog optical depth along a ray. `crates/render/shaders/media.glsl` is the GLSL mirror. `fog_along` and `height_fog_along` call the `f32` wrappers.

## Game use

Build a position with `Vec3::new`. Add a scaled direction to move that position. Build a turn with `Quat::from_axis_angle`. Rotate a direction with `Quat::rotate`. `Mat4::from_quat` and `Mat3::from_quat` build that same turn as a matrix.

The camera stores `Camera.position` as a `Vec3`. `look_direction` returns the view direction. `update` adds a scaled ground direction.

Fog calls `transmittance_f32` and `homogeneous_inscatter_f32` from `fog_along`, and `height_optical_depth_f32` from `height_fog_along`. `phase_weight` calls `phase_hg_f32`.

## Limits

Positions, directions, and rotations stay `f32`. This crate does not build the Vulkan projection. The camera owns that matrix. Lighting probes and mesh vertices stay raw floats.

The medium closed forms are `f64` so the grade matches the analytic expression. The renderer calls the `f32` wrappers. A horizontal height-fog ray is `|cos θ| < 1e-4` (`HEIGHT_FOG_HORIZONTAL_COS`).

## Networking

The closed forms are pure. Vectors are pure. Nothing here is replicated.

- Replicated state: none.
- Authority: none. Every caller evaluates the same function.
- Prediction / rollback: none.
- Determinism: the value is fixed by the arguments.
- Headless server: the same functions. No grid and no picture.

## Decisions

- [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md) keeps engine math in this repository.
