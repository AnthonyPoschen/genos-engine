# The window owns focus, and the frame pulls state

`input-zig` polls the keyboard and the mouse on X11, Windows, and macOS. On Wayland, those two updates are empty. The tested Wayland path is the focused `xdg` window in `debug_input_wayland.zig`. That window reads the seat and writes device state.

The engine window owns that seat on Wayland. Each frame pumps the window, updates input, and then reads the result. Gamepads stay on the device path. XInput, `/dev/input/jsN`, and GameController with the IOKit fallback do not use the window. [ADR 0007](0007-native-calls-go-to-the-system-and-vulkan.md) extends this rule to audio, network, and other native calls.

**Rejected:** `winit` or SDL as the owner of the window and the input. A second Wayland connection beside the engine window.
