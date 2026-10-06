# Scripting

Rhai is the language for games and mods. A script calls engine functions. A script does not call the operating system or Vulkan.

## Goal

An author can place a room, and later a mod can change that room, without a change to the crates.

## Intent

The `rhai` crate is the one scripting exception to the native-call rule. The engine still owns the window, the input, the renderer, and the light field.

A script starts after the host registers the functions. A name that is not registered is not available to the script.

The first host loads a scene. The functions are `floor`, `wall`, `solid`, `light`, and `sun`. Ground positions use X and depth Z. A point light uses X, height Y, and depth Z. A sun uses the direction the rays travel.

## Code

The host is `crates/scene/src/script.rs`. The first script is `examples/camera/scene.rhai`. `load_path` and `load_str` return a `Scene`.

## Game use

Put a new room in a `.rhai` file. Load it once with `load_path` before the frame loop. Pass the `Scene` to `World::from_scene`.

Add a host function in `script.rs` before a script calls a new name. Keep the function on engine types. Do not give the script a Vulkan handle or a window pointer.

## Limits

There is no mod folder, no hot reload, and no gameplay script yet. The host can place geometry and lights. It cannot spawn a mesh asset or bind an input action.

## Decisions

- [ADR 0009](../adr/0009-rhai-is-the-scripting-language.md) selects Rhai and names the crate exception.
- [Scene](scene.md) owns the camera that walks through the loaded room.
