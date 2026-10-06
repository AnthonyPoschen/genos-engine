# Scene

The scene owns the camera and the scripted layout. The ground is a plane. The camera walks on that plane.

## Goal

A player can walk through a scripted room and look around. The picture matches the coordinate rules. The script places the floor, the walls, the solids, and the lights.

## Intent

The ground is the XZ plane. Y is up. Yaw `0` looks along -Z. Positive yaw turns toward +X. Positive strafe is +X.

The camera height stays at `CAMERA_HEIGHT` (`1.7`). The camera walks through solids. [Physics](physics.md) is a separate step. This camera does not call it.

The viewport origin is the top-left `(0, 0)`. The bottom-right is `(1, 1)`. Vulkan NDC +Y is the lower half. A floor point in front of the camera and below the eye has NDC y greater than `0`. A point to the camera's right has NDC x greater than `0`.

`Camera::opening` is the first pose: `x = -6`, `z = 6`, yaw = π/4.

The first script is `examples/camera/scene.rhai`. `floor` and `wall` take ground X and depth Z. `light` takes X, height Y, and depth Z. `solid` takes a shape name, ground X, depth Z, size, and a color.

The scene crate does not call Vulkan. It keeps the lights and the objects. The renderer lighting engine decides which light reaches which surface. See [Lighting](lighting.md).

## Code

The crate is `crates/scene`, package `genos-scene`.

- `Camera`, `update`, `look_direction`, and `view_proj` are in `src/camera.rs`.
- `Camera.position` is a `Vec3`. `look_direction` returns the view direction.
- `Scene`, `Floor`, `Wall`, `Solid`, and `Light` are in `src/types.rs`.
- Floor, wall, solid, and light positions are `Vec3` values. Colors stay raw floats.
- `load_path` and `load_str` are in `src/script.rs`.

## Game use

Load a script with `load_path` once, before the loop. Build a `World` from that `Scene` in the renderer.

Each frame, read `move` and `look` from the input system. Call `update` with those actions. `update` adds a scaled ground direction from `look_direction`. Pass the camera to `Renderer::draw`.

A new room is a new script. Register a new function on the Rhai host before a script can call it. See [Scripting](scripting.md).

## Limits

The camera does not collide with walls. There is one room script. There is no scene graph and no entity id.

This crate does not march light. The ground-plane march is in the renderer.

## Decisions

- [ADR 0010](../adr/0010-first-proof-is-the-lit-scripted-scene.md) replaces the unlit camera proof.
- [ADR 0009](../adr/0009-rhai-is-the-scripting-language.md) selects Rhai for the layout script.
- [ADR 0005](../adr/0005-first-proof-is-a-camera-over-shapes.md) is the earlier unlit proof. Do not restore it as the goal.
