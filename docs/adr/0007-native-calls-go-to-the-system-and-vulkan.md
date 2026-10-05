# Native calls go to the system and to Vulkan

Engine code makes each native call to the operating system or to Vulkan. A third-party crate does not own the window, the input, the renderer, the audio, or the network. The Rust standard library stays. When the standard library has no equivalent, the helper lives in this repository. Scripting is the exception in [ADR 0009](0009-rhai-is-the-scripting-language.md).

The link line is the system library for that native call. Window and input use the same system libraries as `input-zig`. Those libraries include `user32`, `kernel32`, `xinput1_4`, `X11`, `wayland-client`, `xkbcommon`, and the macOS frameworks behind `macos_shim.m`. Linux audio uses ALSA, and Omarchy provides that device through PipeWire. Windows audio uses WASAPI, and macOS audio uses CoreAudio. Network code uses Winsock or BSD sockets.

Vulkan calls use the Vulkan loader. macOS links MoltenVK. [ADR 0003](0003-one-vulkan-renderer-moltenvk-on-macos.md) records that choice. Declarations and the Objective-C shim live in this repository. The declarations follow the `extern` style of `input-zig`.

**Rejected:** `winit`, SDL, GLFW, `wgpu`, `ash`, `vulkano`, `x11rb`, `cpal`, `rodio`, and `tokio`.
