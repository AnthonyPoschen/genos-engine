# Genos Engine

Genos Engine is the game runtime and the Linux editor in this repository. The hosting product stays in the `genos` repository.

## Language

**Runtime**:
The program that runs a game on a device.
_Avoid_: Engine, player, host

**Editor**:
The Linux program an author uses to make a game.
_Avoid_: Editor pod, IDE

**Author**:
The person who makes a game in the editor.
_Avoid_: User, developer, customer

**Player**:
The person who plays a game in the runtime.
_Avoid_: User, client

**Game**:
A product the runtime runs.
_Avoid_: Title, server, slot

**Platform**:
Linux, Windows, or macOS as a place the runtime runs.
_Avoid_: Target, backend

**Display server**:
X11 or Wayland on Linux.
_Avoid_: Backend

**Window**:
The operating-system object that shows a surface and can take focus.
_Avoid_: Viewport, canvas

**Surface**:
The renderable area a window presents.
_Avoid_: Swapchain, framebuffer, screen

**Focus**:
The state in which a window receives the keyboard and the pointer from the display server.
_Avoid_: Active, selected

**Input system**:
The owner of the keyboard, the mouse, and the gamepad slots.
_Avoid_: Input manager, event bus

**Device**:
The keyboard, the mouse, or one gamepad slot.
_Avoid_: Controller, peripheral

**Gamepad slot**:
A stable player slot for one gamepad. Slot 0 stays player one while that gamepad is disconnected.
_Avoid_: Joystick index, controller id

**Input code**:
The shared token for a key, a mouse button, or a gamepad control.
_Avoid_: Keycode, scancode, virtual key

**Action**:
A named intent in a game, such as jump or move.
_Avoid_: Event, command, binding

**Action map**:
The set of bindings from input codes to actions for one player.
_Avoid_: Input map, control scheme

**Binding**:
The input codes that fire one action.
_Avoid_: Mapping, shortcut

**Camera**:
The view a game uses to look at the world.
_Avoid_: Viewport, spectator

**Ground plane**:
The horizontal XZ plane. Up is positive Y. Yaw 0 looks along -Z. Positive yaw turns toward +X.
_Avoid_: Floor

**Character controller**:
The input map that moves the camera on the ground plane and turns the view.
_Avoid_: Physics body, pawn

**Omarchy**:
The Linux desktop the editor must fit first.
_Avoid_: Hyprland

**Native call**:
A call from the runtime or the editor straight to an operating-system API or to Vulkan.
_Avoid_: Middleware, toolkit, framework

**Breaking Point**:
The 2013 C++ engine in `AnthonyPoschen/Breaking-Point`. It is a reference for math and camera ideas.
_Avoid_: Genos Engine, the runtime

**Lighting**:
The light that reaches a surface from an emitter or from another surface.
_Avoid_: Shader, material

**Radiance field**:
The light traveling through the scene, split into distance ranges.
_Avoid_: Lightmap, g-buffer

**Radiance cascade**:
One distance range of the radiance field. A near cascade stores more positions, and a far cascade stores more directions.
_Avoid_: Cascade shadow map, lightmap

**World probe**:
A coarse probe in the world, past the nearer cascades. It carries material-colored light those ranges miss.
_Avoid_: Light probe, reflection probe, lightmap

**Script**:
Rhai code that a game or a mod runs inside the runtime. A script calls engine functions, and it does not call the operating system or Vulkan.
_Avoid_: Python, Lua

**Mod**:
A set of scripts and data that changes a game without a change to the engine.
_Avoid_: Plugin, native library
