# MVP scene

This document is the execution goal for the first proof. The program is `examples/camera`. The scene file is `examples/camera/scene.rhai`. [ADR 0010](../adr/0010-first-proof-is-the-lit-scripted-scene.md) records the choice.

Done means every check under "Done" passes on Linux.

## Picture

The window shows one scene.

- The floor is white.
- Two walls meet at a corner.
- Three solids sit in the scene. One is red, one is blue, and one is green.
- Each solid is a square or a circle.
- The script places at least one light.
- Color from a colored solid appears on the white floor or on a wall, beyond that solid.
- Radiance cascades build that bounced light again every frame.

The ground is the XZ plane. Y is up, and the camera height stays on Y. X is across the screen. Z is depth. Yaw 0 looks along -Z. Positive yaw turns toward +X. Strafe positive moves toward +X.

In the viewport, the top-left is `(0, 0)` and the bottom-right is `(1, 1)`. Vulkan NDC `y > 0` is the lower half of that viewport. A floor point in front of the camera and below the eye lands in that lower half. A point to the camera's right has NDC `x > 0`.

Cascade rays march on the ground plane. Spacing and range length follow the floor size, so the same builder fits any scene. A nearer range keeps more positions and fewer directions than a farther range. World probes sit on a coarse world grid past the floor and fill rays the nearer ranges miss. A hit takes the color of that material. Probe colors are blended, and each floor vertex samples that blend. The field is built again every frame from the current scene. The notes are in [Lighting](../lighting.md).

## Input

`genos-input` is the input component. The camera reads it. The camera does not keep key state of its own.

The component follows `input-zig`.

- `InputSystem` owns one keyboard, one mouse, and the gamepad slots.
- Each frame, platform code writes those devices. The caller then reads the devices or an action map.
- `InputCode` uses the same names and the same numeric values as `input-zig`.
- `ActionMap` binds those codes to named actions. `move` is one 2D binding. The map loads and saves JSON.
- Platform code writes devices only. It does not know the camera.
- On Wayland, the focused engine window supplies keyboard and mouse state. Gamepad state comes from `/dev/input/jsN`.
- The character controller is that action map. It is not a second input path.

## Controls

The input map is a character controller, and it drives the camera.

- The `move` action binds `W`, `A`, `S`, `D`, and the left stick.
- `move` travels on the ground plane relative to the camera yaw.
- Mouse movement and the right stick change yaw and pitch.
- Pitch stops before the camera looks straight up or straight down.
- The first click locks and hides the Wayland pointer. The window does not lock the pointer when it opens.
- `Escape` destroys that lock and shows the pointer again.
- The window close control exits the program.

The camera does not collide with the floor or the walls. This proof has no physics engine.

On Wayland, the keyboard and the mouse act while the window has focus.

## Script

`scene.rhai` places the floor, the walls, the three solids, and the lights. The runtime registers the functions that script calls. The script runs when the scene loads.

`floor`, `wall`, and `solid` take ground `x` and depth `z`. `light(x, y, z, r, g, b)` takes height as `y`.

After a script edit, start the program again. The positions change.

## Out of this goal

The editor, physics, collision, world probes, audio, network, and a second scene wait.

## Done

On Linux, run `examples/camera`. Confirm each check. Checks 8 and 9 need a connected gamepad.

1. The white floor, the corner, and the red, blue, and green solids are visible.
2. Hold `W`. The camera moves forward along its yaw and stays at the same height.
3. Hold `A` and `D`. The camera strafes on that same plane.
4. Hold `S`. The camera moves back on that same plane.
5. Move the mouse. The view yaws and pitches.
6. Keep pitching. The view stops short of straight up and straight down.
7. Press `Escape`. The pointer is free.
8. Connect a gamepad. The left stick moves the camera.
9. Move the right stick. The view turns.
10. Find a tint from a colored solid on the white floor or on a wall, outside that solid.
11. Change a position in `scene.rhai`. Start the program again. That object sits in the new place.
12. Resize the window. The picture returns.
13. Close the window. The program exits.
14. The input system reports one keyboard, one mouse, and stable gamepad slots. Slot 0 stays available while that pad is disconnected.
15. The `move` action reads `W`, `A`, `S`, `D`, and the left stick from that system.
16. The character-controller map round-trips through JSON.
