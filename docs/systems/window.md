# Window

The window owns the Wayland seat. It presents one surface. It reports focus, size, pointer lock, and fullscreen to the frame.

## Goal

A game window opens, takes focus, and gives the runtime a surface. A widget can follow resize and focus. A game can lock the pointer while the player looks, and can release it for a pause menu.

## Intent

The frame pulls state. `Window::pump` reads the compositor, then tells subscribers what changed. When no events are queued, `Window::pump` returns at once. A listener runs inside `pump`. A listener must not call `pump`.

The window does not lock the pointer when it opens. A click does not lock the pointer. The frame calls `set_pointer_capture` when it accepts the click.

The compositor pings a mapped window (`xdg_wm_base.ping`). The answer is sent only while `Window::pump` or the seat dispatch reads the socket. Omarchy treats a missed ping as "Application not responding". The frame keeps the gap between dispatches inside 100 ms. `StallMeter` records the longest gap. The scene stores it as `EventLoopSettings.longest_stall_ms`.

A scene file is read on a worker once the window is mapped. The frame draws a small loading picture and keeps pumping. Device waits, pipeline compiles, and copies larger than a megabyte run on the frame thread while a side thread dispatches the seat. That side thread stops before the next present or `pump`. Quit runs the app lifecycle below. A scene that is leaving is dropped on `genos-retire`, not on the frame thread. The current scene is not dropped on the way out.

`lock_cursor` hides the cursor and confines it. `unlock_cursor` shows the cursor and lets it leave. Escape is a game action. The window does not bind Escape itself.

`set_mode` asks for `WindowMode::Fullscreen` or `WindowMode::Windowed`. `WindowEvent::ModeChanged` arrives on a later pump, after the compositor applies the mode.

`Window::open` is the player window. The class is `genos`. The title is `Genos`. The compositor can focus it and tile it.

`Window::open_proof` is the check window. The class is `genos-camera`. The title is `Genos Engine`. The Hyprland rule for that class floats the window and refuses focus. Checks pass `--proof` so they use this window. `make run` does not pass `--proof`.

`Window::open_proof_on` and `Window::open_named_on` take a Wayland display name. `None` connects to the session compositor. A verdict script sets the child `WAYLAND_DISPLAY` to the requested display. It passes `--display` only when that binary's help lists the flag. An older binary places the window from the environment. An empty name is refused.

On Wayland, the keyboard and the mouse act while the window has focus. Gamepads do not use the window.

## Code

The crate is `crates/window`, package `genos-window`.

- `Window`, `Frame`, and `FocusGate` are in `src/lib.rs`.
- `WindowEvent` and `WindowMode` are in `src/events.rs`.
- The app lifecycle is in `src/lifecycle.rs`.
- The Wayland seat, the pointer lock, and fullscreen are in `src/wayland_shim.c`.

## App lifecycle

`Lifecycle` owns the window and the device. Safe code cannot move either out. Shutdown is a state machine on the event thread: stop producers, drain the GPU, destroy resources, unmap, then exit. `Window::pump` runs before each step, so `xdg_wm_base.ping` is answered while the window is mapped.

Producers are polled. A script thread is joined only after it has finished. After two seconds it is detached, because joining a script blocked on the frame thread would deadlock. The MCP listener is dropped in that same step. Its accept thread is idle, so the join is short.

The GPU drain does not call `vkDeviceWaitIdle`. One empty submit on the graphics queue, and one on the compute queue when that queue is distinct, is ordered after the last present. The machine polls those fences, and the lighting world fence when a world submit is in flight. Each poll is one pump apart. Frame fences are not polled: a reset without a submit leaves them unsignaled. Destruction is three steps (yield, lighting resources, the rest of the device) with a pump between them. The window is unmapped only after the device is gone. `genos_window::exit` flushes stdout and is the only `process::exit`.

The longest gap between those pumps is printed as `lifecycle_quit_gap_ms=`. `lifecycle_drain_forced=1` means a fence was still unsignaled after 30 seconds and destruction continued. The budget is 100 ms. `--lifecycle-block-ms N` waits N milliseconds inside the drain, with no pump inside the wait, so the quit-gap test can prove it catches a stall. `GENOS_LIFECYCLE_BLOCK_MS` is the mirror and applies only when the flag was omitted. It is a test hook, not a player setting.

Failure modes: a gap at or above 100 ms means teardown blocked the event thread (a device wait, a thread join, or a destroy step). Exit 134 and `corrupted size vs. prev_size` mean the device was left for the driver's exit handler while other threads were still alive. The probes are the two stdout lines, the process exit code, and `crates/window/tests/lifecycle_quit.rs`.

## Networking

- Replicated state: none. The window, the seat, and the shutdown phase stay in the process that opened the surface.
- Authority: that process. The compositor is the display server, not a game server.
- Prediction / rollback: none. Pointer lock and mode are requests. The compositor confirms them on a later pump.
- Determinism: shutdown polls fences. It does not wait an unbounded device idle. The quit gap is the longest dispatch interval.
- Headless server: a gameplay server opens no window and does not run this lifecycle. `--headless` and `--proof` still map a window on a local compositor.

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
