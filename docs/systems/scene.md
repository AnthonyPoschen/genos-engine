# Scene

The scene owns the camera and the scripted layout. The ground is a plane. The view rides one capsule on that plane.

## Goal

A player can walk through a scripted room and look around. The picture matches the coordinate rules. The script places the floor, the walls, the solids, and the lights. The view stops on those colliders. A solid can also move through fixed positions on a loop. A wall or another solid can stop that move.

## Intent

The ground is the XZ plane. Y is up. Yaw `0` looks along -Z. Positive yaw turns toward +X. Positive strafe is +X.

The eye rests near `CAMERA_HEIGHT` (`1.7`). The capsule stands on the floor. Its top is the eye. With no move input, the eye stays within `0.05` m of `1.7` m.

Move input writes a desired horizontal velocity on that capsule. The velocity is relative to yaw and ignores pitch. [Physics](physics.md) integrates the wish. A wall or a solid removes the part of the wish that points into it. The eye position comes from the capsule after the step.

Look still changes yaw and pitch on the camera. The room keeps one capsule for the view.

A codimation binds one solid to two or more fixed positions and one easing. The frame step moves that solid from one position to the next. The physics step resolves the move. Contact still runs. A clear segment ends within `0.05` m of that segment's end position. After the last position, the next segment returns to the first position.

A later time is still on the path. A wall or another solid keeps the moving solid on the near side. The scene position and the collider stay on the same volume. A solid with no codimation does not move because of this system. The camera wish is unchanged.

The viewport origin is the top-left `(0, 0)`. The bottom-right is `(1, 1)`. Vulkan NDC +Y is the lower half. A floor point in front of the camera and below the eye has NDC y greater than `0`. A point to the camera's right has NDC x greater than `0`.

`Camera::opening` is the first pose: `x = -6`, `z = 6`, yaw = π/4. `attach_scene` builds the room colliders and places the capsule under the eye.

The running camera serves that scene on a loopback MCP endpoint. An agent can read the floor, the walls, the solids, the lights, and the camera. An agent can change a stored field, add an object, or remove an object. The picture and the colliders use that same scene. The script file stays unchanged.

The first script is `examples/camera/scene.rhai`. `floor` and `wall` take ground X and depth Z. `light` takes X, height Y, and depth Z. `solid` takes a shape name, ground X, depth Z, size, and a color.

The floor, each wall, and each square solid are boxes. The circle solid is a cylinder mesh reduced to a convex hull. Lights have no collider.

The scene crate does not call Vulkan. It keeps the lights and the objects. The renderer lighting engine decides which light reaches which surface. See [Lighting](lighting.md).

## Code

The crate is `crates/scene`, package `genos-scene`.

- `Camera`, `attach_scene`, `codimate`, `update`, `look_direction`, and `view_proj` are in `src/camera.rs`.
- `Codimation` and `Easing` are in `src/codimation.rs`. `Easing` is `Linear`, `EaseIn`, `EaseOut`, or `EaseInOut`.
- `Camera.position` is the eye. `Camera.physics` is the world. `Camera.view` is the capsule index.
- `look_direction` returns the view direction.
- `Scene`, `Floor`, `Wall`, `Solid`, and `Light` are in `src/types.rs`.
- Floor, wall, solid, and light positions are `Vec3` values. Colors stay raw floats.
- `load_path` and `load_str` are in `src/script.rs`.
- The agent endpoint is `crates/mcp`, package `genos-mcp`.
- `Host` keeps the scene and the camera that the frame draws.
- `Server` binds `127.0.0.1` and writes one discovery record.
- The record path is `$XDG_RUNTIME_DIR/genos/mcp/<pid>.json`.
- When `XDG_RUNTIME_DIR` is empty, the path is `/tmp/genos-mcp-<uid>/<pid>.json`.
- The resource URI is `genos://scene`.
- Handles are `floor`, `wall:<id>`, `solid:<id>`, and `light:<id>`.

- `Wall.base` is the height of the underside. 0 stands on the floor. The script call `raised_wall(x, z, width, depth, height, base)` places a lintel, a sill, a beam or a roof slab.
- `Solid.yaw` turns a square solid about +Y in radians, the sense of `Quat::from_axis_angle` about +Y. The collider, the picture and the light use it. A circle ignores it.

## Game use

Load a script with `load_path` once, before the loop. Call `Camera::attach_scene` with that scene. Build a render `World` from the same `Scene`.

Each frame, read `move` and `look` from the input system. Measure the seconds since the previous frame. The first frame has no previous frame. Pass a finite positive fallback for that frame.

Call `update` with the scene, those actions, and that interval. Call the particle advance with the same interval. Do not pass a fixed `1/60` s step as the frame interval.

`update` writes the wish in units per second. The interval scales that wish. The physics solver runs at 1/60 s. It runs when the saved time reaches that step. A shorter remainder waits for the next frame. The camera loop caps one frame at 0.25 s.

A non-positive or non-finite interval does not move the camera. It does not change stick look. It does not step physics. It does not advance a codimation. Mouse look stays a pointer delta. Pass the camera to `Renderer::draw`.

Call `Camera::codimate` after `attach_scene`. Pass the solid index and a `Codimation`. The solid starts on the first position. Each slice submits that path's displacement to the physics step. The resolved position is written back onto the solid. `set_codimation_running(solid, false)` holds the solid at its current point. The path time stays. `true` resumes from that point.

The camera starts the endpoint after it loads the script. An agent discovers the URL from the record for that process. A record for a dead process is not a live endpoint.

Read `genos://scene` for the live document. Call `read_scene` for that same document. Call `set_object`, `spawn_object`, `remove_object`, or `set_camera` to edit it.

A geometry edit rebuilds the room colliders. A color change, an absorption change, or a light edit does not rebuild those colliders.

Subscribe to `genos://scene`.

One frame publishes one update when the scene or the camera pose changes. The endpoint sends `notifications/resources/updated`.

A new room is a new script. Register a new function on the Rhai host before a script can call it. See [Scripting](scripting.md).

## Limits

The capsule does not jump or crouch. The step does not turn it. There is one room script. There is no scene graph. The scene types have no entity id. The agent endpoint assigns a handle, and that handle stays until removal.

The endpoint listens on `127.0.0.1` only. The endpoint stops when that camera process exits. An edit is not written to the script.

A codimation translates one solid. It does not rotate or scale that solid. The script cannot author a path. A slice longer than the collider can still pass through it. The picture places a wall or a square solid from `position.x`, `position.z`, and `height`. The picture does not follow `position.y`.

This crate does not march light. The ground-plane march is in the renderer.

The physics world holds `MAX_BODIES` (32) bodies. `attach_scene` with more than 30 walls and solids drops the rest, and the view capsule with them, so a large level cannot be walked with colliders yet. The stress example flies instead.

## Decisions

- [ADR 0010](../adr/0010-first-proof-is-the-lit-scripted-scene.md) replaces the unlit camera proof. The view now uses a physics capsule. That ADR rejected a physics body for the first proof.
- [ADR 0009](../adr/0009-rhai-is-the-scripting-language.md) selects Rhai for the layout script.
- [ADR 0005](../adr/0005-first-proof-is-a-camera-over-shapes.md) is the earlier unlit proof. Do not restore it as the goal.
