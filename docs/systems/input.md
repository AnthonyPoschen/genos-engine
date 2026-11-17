# Input

The input system owns the keyboard, the mouse, and the gamepad slots. A game reads actions. A game does not read raw device bits for movement.

## Goal

A player can move and look with a keyboard, a mouse, or a gamepad. The same action names work on each platform. A released stick does not move the player.

## Intent

The model is a pull. Each frame calls `begin_frame`, then writes fresh device state, then reads the action map.

`InputCode` names and numeric values match `input-zig`. The names stay snake case. That enum is the one allowed `non_camel_case_types` exception. `key_grave` is the key left of `1` (`~). Its value is 192, the Windows OEM 3 virtual key, and Linux evdev code 41. `key_left_bracket` is `[` (219, evdev 26). `key_right_bracket` is `]` (221, evdev 27).

One keyboard, one mouse, and stable gamepad slots stay. A disconnected pad keeps its slot.

The stick deadzone is per axis. The default is `DEFAULT_STICK_DEADZONE` (`0.08`). A value inside the deadzone becomes zero. A larger tilt stays unchanged. A full tilt still reaches `1`. `InputSystem::set_stick_deadzone` sets every slot. `GamepadDevice::set_stick_deadzone` sets one stick. The connected pad rests near `0.07` on an axis, so `0.06` still let that rest through.

A binding has a sensitivity. The default is `1`. The deadzone runs first, then the sensitivity scales what is left. `2` on a half tilt reaches `1`. The result is still clamped to `-1..=1`. `InputSettings` key `mouse_sensitivity` scales `add_mouse_delta`. The default is `1`.

Linux reads `/dev/input/jsN`. The first poll opens each present device and keeps that file. A later poll reads new events from the open file. A poll of a pad whose name is already stored does not allocate: the glyph family was computed when the name changed, and the compare does not build a new string. `connect_gamepad`, `set_gamepad_stick`, `set_gamepad_button`, and `set_gamepad_trigger` are the writes a platform uses. The Linux poll is one caller of those writes. Windows calls the same writes from `XInput`.

On an Xbox pad, axis 0 and axis 1 are the left stick and axis 3 and axis 4 are the right stick. The Y axis is negated. Axis 2 and axis 5 are triggers that rest at `-1` and reach `1`; that range is stored as `0..=1`. `GamepadDevice::down` for a trigger is true when that button bit is down or the axis is at or above `activation_threshold` (`0.5`). Axis 6 and axis 7 are the d-pad. A Steam Deck has no `ABS_Z` or `ABS_RZ`. Its sticks are axes 0–3, its trackpads are axes 4–7 and are ignored, and the analog triggers are `ABS_HAT2X` and `ABS_HAT2Y` at joydev indices 8 and 9. The character map does not use the triggers.

joydev numbers the buttons that are set, from `BTN_JOYSTICK` upward and then the codes below it, so the index depends on the family. `JSIOCGBTNMAP` and `JSIOCGAXMAP`, read once when the pad opens, override that guess: select, start, home, and the stick clicks stay on `BTN_SELECT`, `BTN_START`, `BTN_MODE`, and `BTN_THUMBL` / `BTN_THUMBR` even when the pad also sets `BTN_TL2` / `BTN_TR2`. When the ioctl fails, the family table is the fallback. Xbox (`xpad`) reports the labeled X button, the west face, as `BTN_X` (`0x133`) and Y, the north face, as `BTN_Y` (`0x134`). The ioctl map uses that same pair for Xbox and Steam Deck. PlayStation and a generic pad use the Linux names, so `BTN_X` is north and `BTN_Y` is west. The xpad fallback, used when the ioctl fails, still puts the west face at index 2 and the stick clicks at 9 and 10. PlayStation (`hid-playstation`, DualShock, DualSense) reports north at index 2, the trigger buttons at 6 and 7, and the stick clicks at 11 and 12. A generic pad without those trigger buttons keeps select at 6. A Steam Deck follows `hid-steam`: index 0 is the left pad touch, 1 is the right pad touch, 2 is the quick-access button, 3 is south, 5 is west (`BTN_X`), 6 is north (`BTN_Y`), 9 and 10 are the digital triggers, and 11 is View. Pad touches and grips are not bound. Buttons past 10 are kept.

The picture family is a glyph set: keyboard and mouse, Xbox, PlayStation, Steam Deck, a generic gamepad, or touch. The name on the slot picks the pad family, once, when the pad connects or the name changes. `Steam Deck` is checked before Xbox. A Deck that reports an Xbox name gets Xbox pictures, because that is the device it is emulating. A finger that begins selects touch. `active_glyphs` is the family to draw. `active_id` is the device that selected it. `take_device_changes` returns connections and changes of family or active id since the last take. A stick that is already outside the deadzone does not select the family again. A stick selects it when it crosses out of the deadzone. A button or a trigger selects it on the press. A key or a mouse button selects keyboard and mouse on the press. A mouse delta selects it while the pointer is moving. A second pad of the same family still emits a `DeviceChange` when it becomes the active slot. Disconnecting the active pad selects another connected pad, or the keyboard.

`InputSettings` is the settings value. `to_text` and `parse` round-trip it. `parse` of `key=value` lines starts from the character-controller map, so a file that only sets `stick_deadzone` keeps WASD. `set_key` overlays one line. A sensitivity line may come before the binding line. On a name that does not exist yet, that line creates an axis action, so `camera.sensitivity` can be followed by `camera.vectors`. An enabled line stays in force when the binding line follows it. A binding's sensitivity is not a public field; the text and JSON paths clamp it to `-8..=8`. `save` and `load` read that text from a file. A file that starts with `{` is the action-map JSON. The JSON reader accepts a space after `:` and a `sensitivity` field that precedes `code`. `stick_deadzone` and `mouse_sensitivity` are keys. An action name replaces button bindings (`release=key_b mouse_right`). `move.left` replaces one side. `code*2` sets that binding's sensitivity. `look.sensitivity` sets every binding on that action. `dump` on the settings is the binding text. `InputSystem::dump` lists the devices and, after `InputSettings::apply`, the same bindings.

Replicated state: none. Bindings are local settings. Device state is not replicated.
Authority: the local runtime owns the devices.
Prediction / rollback: none. The pull is the current frame.
Determinism: action evaluation is a pure function of the device state and the map. The Linux poll is not deterministic.
Headless server: the map and the settings file load with no devices. `poll_gamepads` finds nothing and leaves the slots disconnected.

The input crate does not depend on the window or the renderer.

A touch point is one finger sample: an id, a position, and a phase (`began`, `moved`, `ended`, `cancelled`). Positions are in points (pixels), the same space the window reports, and positive Y follows that space. The window writes them with `set_touch_points`. This crate recognizes a tap, a swipe, and a pinch. A lift that travels at most `TAP_SLOP` (`16` points) is a tap. A lift that travels at least `SWIPE_MIN` (`48` points) is a swipe, stored as a unit vector in the point's coordinates. A pinch compares the two-finger span with the span at the start of the gesture, or with the last span that emitted, so a slow change still emits once it reaches a tenth. `scale` is that ratio. The bound axis adds `(scale - 1)` times the action sensitivity on Y, then clamps to `-1..=1`. The pair is the same two contact ids until one of them lifts. A span of `1` point or less is not a baseline; the first sample past that sets the baseline and does not emit. A batch that moves the fingers and ends them still emits: every sample writes its position, including `Ended`, the pinch runs while both contacts are present, and only then are the lifts classified. A finger in that pinch pair does not also count as a tap. Gestures clear in `begin_frame`. The baseline does not. A drag between the tap slop and the swipe minimum is neither. A cancelled finger drops the contact and makes no gesture. The action-map JSON writes `gesture` on the action that has one.

`name.gesture` is `tap`, `swipe`, `pinch`, or `off`. A tap sets `down`. A swipe or a pinch adds to `axis_2d`. A missing action is created with no keys. The character-controller map does not bind a gesture until a settings line does.

## Code

The crate is `crates/input`, package `genos-input`.

- `InputSystem` in `src/system.rs` owns the devices, the gamepad poll, and the glyph-set changes. `src/xinput.rs` owns the Windows pad read. `src/gamecontroller.rs` owns the macOS pad read.
- `InputCode` in `src/code.rs` owns the names and the values.
- `character_controller` in `src/action.rs` owns the first action map. `InputSettings` in `src/settings.rs` owns the settings file.
- `GamepadDevice` in `src/device.rs` owns the stick deadzone. `glyph_set_from_name` owns the picture family.
- `TouchSurface` in `src/touch.rs` owns the points and the gestures.
- `debug/input` (`genos-input`) prints the live view and serves `get_input`, `set_input`, and `get_glyphs` on `127.0.0.1:8768`.

## Game use

Call `character_controller()` for the first player. The map has these actions:

- `move` reads `WASD`, `IJKL`, and the left stick.
- `look` reads the right stick. Mouse motion stays on `MouseDevice` (`mouse.dx`, `mouse.dy`), scaled by `mouse_sensitivity`.
- `capture` reads the left mouse button.
- `release` reads Escape.

Read the axes with `axis_2d`. Read buttons with `down`. `GamepadDevice::pressed` is the edge for a button or a digital stick direction, matching the keyboard. Change the deadzone before the poll that must use the new value. Rebind with `rebind`, `rebind_2d`, or `InputSettings::set_key`, then `save`. Read `active_glyphs` after `take_device_changes` when the pictures should follow the device.

`genos-ui` calls `ui_actions()` for focus. `ui_left`, `ui_right`, `ui_up`, and `ui_down` are the arrows, the d-pad, and the left stick. `ui_activate` is Enter, Space, and the south face button. `ui_back` is Escape and the east face button. `ui_next` is Tab. Shift-Tab moves to the previous control. `key_a` stays on `move`.

## Limits

X11 device reads are not in this crate yet. Windows reads XInput inside `poll_gamepads` and writes each slot with `connect_gamepad`, `set_gamepad_stick`, `set_gamepad_button`, and `set_gamepad_trigger`. The pad name is `XInput`, so the pictures are Xbox. Slots `0..3` are the XInput user indices. macOS reads GameController the same way. Connect and disconnect notifications keep a controller in one slot until it leaves. Those notifications run on the main queue, and the poll does not, so one lock covers the slot assign, the clear, and the poll fill. The pad's vendor name is copied once, on connect, and selects the pictures. A controller without an extended gamepad does not take a slot. IOKit is not read, so a pad GameController does not publish stays disconnected. A missing slot is disconnected. Stick Y stays positive upward. The engine deadzone still applies on read. A later poll of a pad that is already connected does not allocate: `connect_gamepad` returns before it copies the name.

The Win32 and Cocoa windows write the keyboard, the pointer, and touch through `Window::write_devices`, which calls `write_window`. That call uses `set_key`, `set_mouse_button`, `add_mouse_delta`, and `set_touch_points`. The window does not own a second action map and does not recognize a tap, a swipe, or a pinch.

Every backend uses the same writes:

- `begin_frame`, then the writes, then the game reads the action map.
- `set_key` or `apply_evdev_keys` for the keyboard.
- `set_mouse_button` and `add_mouse_delta` for the pointer.
- `connect_gamepad`, `set_gamepad_stick`, `set_gamepad_button`, and `set_gamepad_trigger` for a pad. Linux reads `/dev/input/jsN` inside `poll_gamepads`. Windows reads XInput. macOS reads GameController.
- `set_touch_points` for fingers. The action map turns points into a tap, a swipe, or a pinch. Win32 pointer messages and Cocoa touches are the samples. A finger that is still down and did not move this frame is reported as `moved`.

The behavior source for the key and pointer reads is `input-zig`, listed in [Architecture](../architecture.md). The UI reads `take_device_changes` and `active_glyphs` after those writes.

`input-zig` has no touch code. Touch points are a write from the window, not an operating-system read in this crate. The touch device id is `2`. The keyboard is `0` and the mouse is `1`.

`gilrs` is not a dependency. [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md) keeps the joystick read in this crate.

`KeyBindings` names the debug keys. `builtin` is grave or F1 for the menu, F or F3 for the frame-rate plate, U or F8 for the unfilled count, and shift-U or shift-F8 for the tint. `tab_next` is `]`. `tab_prev` is `[`. `tab_focus_next` is Tab and `tab_focus_prev` is shift-Tab, for when the pointer is over the menu. The first key is the one a board without function keys uses. `apply_text` replaces chords from `action key [key...]` lines. `+key_u` means shift. `debug_keys` reads `GENOS_DEBUG_KEYS` when that path is set, then those lines win. A game calls `pressed` with the action name. It does not test `key_f1` itself.

## Decisions

- [ADR 0001](../adr/0001-rust-port-of-input-zig.md) keeps the pull model, the codes, and the action maps.
- [ADR 0004](../adr/0004-window-owns-focus-and-the-frame-pulls-state.md) puts the Wayland seat on the window. Gamepads stay on this system.
- [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md) forbids an input middleware crate.
