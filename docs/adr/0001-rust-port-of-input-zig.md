# Rust replaces the Zig input library inside this repository

`input-zig` already polls the keyboard, the mouse, and gamepads on Windows, macOS, and Linux. Genos Engine is a Rust workspace. The `genos-input` crate is a Rust port of that library. The port keeps the pull model, the input codes, the action map, and the JSON binding file. Platform calls follow the Zig sources that were tested on each system.

**Rejected:** Link the Zig library into Rust. Invent a second input model.
