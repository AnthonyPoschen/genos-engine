# Architecture

Genos Engine has two programs. The runtime runs a game on Linux, Windows, and macOS. The editor makes games on Linux. The decisions are in `docs/adr/`.

## Native calls

Engine code makes each native call to the operating system or to Vulkan. A third-party crate does not own the window, the input, the renderer, the audio, or the network. The Rust standard library stays. See [ADR 0007](adr/0007-native-calls-go-to-the-system-and-vulkan.md).

The first proof uses the window, the input, Vulkan, Rhai, and radiance cascades. Audio and network follow the same rule when they arrive.

## First proof

The first program is `examples/camera`. The execution goal is [MVP scene](goals/mvp-scene.md). See [ADR 0010](adr/0010-first-proof-is-the-lit-scripted-scene.md).

Linux on Wayland is the first machine for this proof. Windows and macOS run the same scene when those machines are available. The X11 window waits until an X11 session is a target.

On Wayland, the camera moves only while the window has keyboard focus. On X11, Windows, and macOS, the keyboard poll is global. That split already exists in `input-zig`.

## Frame

`examples/camera` owns the frame. When a second program needs the same loop, add a shared runtime crate.

When the scene starts, load `scene.rhai` once.

Each frame does these steps:

1. Pump the window.
2. Update the keyboard, the mouse, and the gamepad slots.
3. Move the camera from `move` and from look.
4. Build the radiance field.
5. Draw the scene with the camera matrix.
6. Present the surface.

## Crates

```text
Cargo.toml
crates/input/       package genos-input
crates/window/      package genos-window
crates/render/      package genos-render
examples/camera/    package genos-camera
```

`genos-input` does not depend on the window or the renderer. The window exposes plain focus events. On Wayland, input maps those events to input codes. On X11, Windows, and macOS, input polls the system. The renderer takes a surface from the window. The renderer does not open a window.

A later `crates/editor` package can depend on these crates. These crates do not depend on the editor. See [ADR 0002](adr/0002-runtime-and-editor-share-one-repository.md).

## Input port

`../input-zig` is the behavior source. `genos-input` is the component that owns that behavior. Keep one keyboard, one mouse, stable gamepad slots, input codes, action maps, and JSON bindings. The camera reads an action map. It does not keep its own key bits. See [ADR 0001](adr/0001-rust-port-of-input-zig.md).

Copy the platform calls from these files:

- `src/platform/linux.zig` for X11 and `/dev/input/jsN`.
- `src/debug_input_wayland.zig` for the Wayland seat.
- `src/platform/windows.zig` for virtual keys and XInput.
- `src/platform/macos.zig` and `src/platform/macos_shim.m` for CoreGraphics, the event tap, GameController, and IOKit.

The Wayland keyboard path in `linux.zig` is empty. The working path is the debug window. The engine window takes that role. See [ADR 0004](adr/0004-window-owns-focus-and-the-frame-pulls-state.md).

On Wayland, the window locks and hides the pointer when capture starts, and it destroys that lock when capture ends. Look then uses relative pointer motion. Opening the window does not lock the pointer.

## Coordinates

The ground is the XZ plane. Y is up. Yaw 0 looks along -Z, and positive yaw turns toward +X. The viewport origin is the top-left `(0, 0)`. The bottom-right is `(1, 1)`.

## Renderer

`genos-render` uses Vulkan on every platform. On macOS, the build links MoltenVK. The window creates a Metal layer for `VK_EXT_metal_surface`. See [ADR 0003](adr/0003-one-vulkan-renderer-moltenvk-on-macos.md).

The first scene uses one swapchain, one depth buffer, and one view-projection matrix. The done picture is the lit scene in [MVP scene](goals/mvp-scene.md).

## Lighting

Lighting uses radiance cascades. Spacing follows the floor size. Near ranges keep more probe positions and fewer directions. Far ranges do the reverse. World probes sit in the world, past those ranges, and carry material-colored light into empty rays. The field is built again every frame. See [Lighting](lighting.md) and [ADR 0008](adr/0008-lighting-uses-radiance-cascades.md).

## Reference

Breaking Point is an old engine. Read it for math and camera ideas. The note is [Breaking Point](references/breaking-point.md). Leave its OpenGL and Win32 code behind.

## Scripting

Games and mods use Rhai. A script calls registered engine functions. The first script is `examples/camera/scene.rhai`. See [ADR 0009](adr/0009-rhai-is-the-scripting-language.md).

## Out of this proof

The editor, more scenes, audio, and networking wait. Fit for Omarchy belongs to the editor. See [ADR 0006](adr/0006-editor-is-linux-first.md).
