# Window

The window owns the Wayland seat. It presents one surface. It reports focus, size, pointer lock, and fullscreen to the frame.

## Goal

A game window opens, takes focus, and gives the runtime a surface. A widget can follow resize and focus. A game can lock the pointer while the player looks, and can release it for a pause menu.

## Intent

The frame pulls state. `Window::pump` reads the compositor, then tells subscribers what changed. A listener runs inside `pump`. A listener must not call `pump`.

The window does not lock the pointer when it opens. A click does not lock the pointer. The frame calls `set_pointer_capture` when it accepts the click.

`lock_cursor` hides the cursor and confines it. `unlock_cursor` shows the cursor and lets it leave. Escape is a game action. The window does not bind Escape itself.

`set_mode` asks for `WindowMode::Fullscreen` or `WindowMode::Windowed`. `WindowEvent::ModeChanged` arrives on a later pump, after the compositor applies the mode.

`Window::open` is the player window. The class is `genos`. The title is `Genos`. The compositor can focus it and tile it.

`Window::open_proof` is the check window. The class is `genos-camera`. The title is `Genos Engine`. The Hyprland rule for that class floats the window and refuses focus. Checks pass `--proof` so they use this window. `make run` does not pass `--proof`.

On Wayland, the keyboard and the mouse act while the window has focus. Gamepads do not use the window.

## Code

The crate is `crates/window`, package `genos-window`.

- `Window`, `Frame`, and `FocusGate` are in `src/lib.rs`.
- `WindowEvent` and `WindowMode` are in `src/events.rs`.
- The Wayland seat, the pointer lock, and fullscreen are in `src/wayland_shim.c`.

## Game use

Open with `Window::open`. Subscribe with `Window::on` before the loop, or later. A late subscriber receives the current size, and receives focus, lock, or fullscreen when those are active.

The events are:

- `Resized { width, height }`
- `Focused` and `Unfocused`
- `CursorLocked` and `CursorUnlocked`
- `ModeChanged(WindowMode)`

`Frame` reports `pointer_x` and `pointer_y` in window pixels. The origin is the top-left.

Call `lock_cursor` when look capture starts. Call `unlock_cursor` when the pause menu opens. Call `set_mode` to ask for fullscreen.

Give `window.display` and `window.surface` to the renderer. The window does not draw.

## Limits

The X11 window waits. Windows and macOS windows wait. The editor window is not built.

A listener cannot pump the window. Fullscreen is a request. The compositor can refuse it.

## Decisions

- [ADR 0004](../adr/0004-window-owns-focus-and-the-frame-pulls-state.md) gives the Wayland seat to this window.
- [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md) forbids `winit`, SDL, and GLFW.
