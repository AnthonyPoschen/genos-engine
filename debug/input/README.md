# debug/input

`genos-input`, the input debug binary. It runs headless: no window and no
draw. A person reads the text view. An agent drives it with a script or MCP.

```
cargo run -p genos-input-debug -- --frames 1
cargo run -p genos-input-debug -- --script view.txt
cargo run -p genos-input-debug -- --rebind release=key_b --mcp
```

`--mcp` serves `get_input`, `set_input`, and `get_glyphs` on
`127.0.0.1:8768`. Lighting uses 8766 and physics uses 8767. `POST /cmd/<name>`
is the same command.

The view lists the glyph set, each action's axis and whether it is down, touch
points, gestures, connected pads, and the settings text.

## Script

`key CODE down|up`, `button CODE down|up`, `mouse DX DY`,
`pad SLOT connect NAME`, `pad SLOT disconnect`, `stick SLOT left|right X Y`,
`touch ID X Y began|moved|ended|cancelled`, `rebind KEY=VALUE`, `frame`.
`frame` prints one view and starts the next frame.

## Window backends

Wayland already writes keys and the pointer through the window. Windows,
macOS, and X11 device reads are the windowing work (agent bc-da635bf1), not
this binary. When those backends land they call the same writes:

- `InputSystem::begin_frame` once, then the writes, then the game reads actions.
- `set_key` or `apply_evdev_keys` for the keyboard.
- `set_mouse_button` and `add_mouse_delta` for the pointer.
- `connect_gamepad`, `set_gamepad_stick`, `set_gamepad_button`, and
  `set_gamepad_trigger` for a pad. Linux already does this inside
  `poll_gamepads` from `/dev/input/jsN`.
- `set_touch_points` with one point per finger. The action map recognizes a
  tap, a swipe, and a pinch. The window does not.

`take_device_changes` and `active_glyphs` are what the UI reads after those
writes. A backend does not keep a second action map.
