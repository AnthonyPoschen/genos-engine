# Window

The window owns the Wayland seat. It presents one surface. It reports focus, size, pointer lock, and fullscreen to the frame.

## Goal

A game window opens, takes focus, and gives the runtime a surface. A widget can follow resize and focus. A game can lock the pointer while the player looks, and can release it for a pause menu.

## Intent

The frame pulls state. `Window::pump` reads the compositor, then tells subscribers what changed. When no events are queued, `Window::pump` returns at once. A listener runs inside `pump`. A listener must not call `pump`.

The window does not lock the pointer when it opens. A click does not lock the pointer. The frame calls `set_pointer_capture` when it accepts the click.

The compositor pings a mapped window (`xdg_wm_base.ping`). The answer is sent only while `Window::pump` or the seat dispatch reads the socket. Omarchy treats a missed ping as "Application not responding". The frame keeps the gap between dispatches inside 100 ms. `StallMeter` records the longest gap. The scene stores it as `EventLoopSettings.longest_stall_ms`.

A scene file is read on a worker once the window is mapped. The frame draws a small loading picture and keeps pumping. Device waits, pipeline compiles, and copies larger than a megabyte run on the frame thread while a side thread dispatches the seat. That side thread stops before the next present or `pump`. Quit destroys the window first, skips `vkDeviceWaitIdle`, and exits. The driver reclaims the device. A scene that is leaving is dropped on `genos-retire`, not on the frame thread.

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

`Frame.wheel_y` is the vertical wheel for this pump, in notches. Positive is wheel up. The horizontal axis is ignored. A continuous axis event wins for that frame. A click wheel's value120 is used when the continuous event did not arrive. A discrete notch is used when neither of those arrived. `pump` clears the wheel after it copies the value out. `FocusGate` passes the wheel only while the window has focus.

Call `lock_cursor` when look capture starts. Call `unlock_cursor` when the pause menu opens. Call `set_mode` to ask for fullscreen.

Give `window.display` and `window.surface` to the renderer. The window does not draw.

## Limits

The X11 window waits. Windows and macOS windows wait. The editor window is not built. The horizontal wheel is ignored.

A listener cannot pump the window. Fullscreen is a request. The compositor can refuse it.

The frame thread does not spend more than 100 ms between seat dispatches while the window is mapped. A load, a device wait, and a quit that block longer than that miss a ping. The side thread that answers pings must not run at the same time as a present or `Window::pump`.

## Decisions

- [ADR 0004](../adr/0004-window-owns-focus-and-the-frame-pulls-state.md) gives the Wayland seat to this window.
- [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md) forbids `winit`, SDL, and GLFW.
