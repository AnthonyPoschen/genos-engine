# Input

The input system owns the keyboard, the mouse, and the gamepad slots. A game reads actions. A game does not read raw device bits for movement.

## Goal

A player can move and look with a keyboard, a mouse, or a gamepad. The same action names work on each platform. A released stick does not move the player.

## Intent

The model is a pull. Each frame calls `begin_frame`, then writes fresh device state, then reads the action map.

`InputCode` names and numeric values match `input-zig`. The names stay snake case. That enum is the one allowed `non_camel_case_types` exception.

One keyboard, one mouse, and stable gamepad slots stay. A disconnected pad keeps its slot.

The stick deadzone is per axis. The default is `DEFAULT_STICK_DEADZONE` (`0.08`). A value inside the deadzone becomes zero. A larger tilt stays unchanged. A full tilt still reaches `1`. `InputSystem::set_stick_deadzone` sets every slot. `GamepadDevice::set_stick_deadzone` sets one stick. The connected pad rests near `0.07` on an axis, so `0.06` still let that rest through.

Linux reads `/dev/input/jsN`. The first poll opens each present device and keeps that file. A later poll reads new events from the open file.

Axis 0 and axis 1 are the left stick. Axis 3 and axis 4 are the right stick. The Y axis is negated. Axis 2 and axis 5 are triggers. The character map does not use the triggers.

The input crate does not depend on the window or the renderer.

## Code

The crate is `crates/input`, package `genos-input`.

- `InputSystem` in `src/system.rs` owns the devices and the gamepad poll.
- `InputCode` in `src/code.rs` owns the names and the values.
- `character_controller` in `src/action.rs` owns the first action map.
- `GamepadDevice` in `src/device.rs` owns the stick deadzone.

## Game use

Call `character_controller()` for the first player. The map has these actions:

- `move` reads `WASD`, `IJKL`, and the left stick.
- `look` reads mouse motion and the right stick.
- `capture` reads the left mouse button.
- `release` reads Escape.

Read the axes with `axis_2d`. Read buttons with `down`. Change the deadzone before the poll that must use the new value.

## Limits

Windows, macOS, and X11 device reads are not in this crate yet. The behavior source for those reads is `input-zig`, listed in [Architecture](../architecture.md).

The action map does not load a player-authored JSON file at runtime yet. `to_json` and the parser exist for the map shape.

## Decisions

- [ADR 0001](../adr/0001-rust-port-of-input-zig.md) keeps the pull model, the codes, and the action maps.
- [ADR 0004](../adr/0004-window-owns-focus-and-the-frame-pulls-state.md) puts the Wayland seat on the window. Gamepads stay on this system.
- [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md) forbids an input middleware crate.
