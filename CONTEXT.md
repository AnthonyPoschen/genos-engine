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

**App lifecycle**:
The owner of shutdown. The event loop stops producers, drains the GPU by polling fences, destroys the device, unmaps the window, and then exits. Opening the device returns a permit that this owner consumes. Dropping the device without that owner panics and does not wait idle.
_Avoid_: quit path, teardown helper

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

**Scene handle**:
A stable name for one floor, wall, solid, or light in the live scene. The floor name is `floor`.
_Avoid_: Entity id

**Agent endpoint**:
The loopback MCP session on the running camera. An agent reads and edits the live scene there.
_Avoid_: Editor, game network

**Ground plane**:
The horizontal XZ plane. Up is positive Y. Yaw 0 looks along -Z. Positive yaw turns toward +X.
_Avoid_: Floor

**Character controller**:
The input map that moves the camera on the ground plane and turns the view.
_Avoid_: Physics body, pawn

**Position**:
A point in the world. Y is up. The camera and each placed object store one.
_Avoid_: Coordinate, transform

**Direction**:
A vector that aims a view or a movement.
_Avoid_: Heading, bearing

**Body**:
A simulated mass. It has a position, an orientation, a linear velocity, an angular velocity, a motion quality, and a shape.
_Avoid_: Character controller, pawn

**Angular velocity**:
How fast a body turns, in world space, radians per second. The step integrates it into orientation. Rotational energy is `½ ωᵀ I ω`.
_Avoid_: Spin rate, torque

**Motion quality**:
How far one body's collision looks ahead in a single physics step. Discrete uses the current bounds. Speculative uses a margin. Swept is the time-of-impact path.
_Avoid_: CCD mode, collision flag

**Speculative contact**:
A contact generated inside `min(speed * dt, speculative margin)` so the solver can stop a body before it overlaps.
_Avoid_: Sweep, raycast

**Time of impact**:
The moment a swept pair's gap closes. The step advances that pair to the moment, solves the contact, and repeats.
_Avoid_: Sweep, raycast

**Conservative advancement**:
An upper bound on how fast a gap can close: `gap / (|v_a − v_b| + |ω_a| r_a + |ω_b| r_b)`. The step uses it with no scale factor.
_Avoid_: Closest-point velocity, time of impact scale

**Physics step**:
One update that moves bodies by gravity, contact, and springs. A speculative body can stop inside its margin. A swept pair advances to the time of impact. A broken bond swaps in chunk bodies from a fracture asset.
_Avoid_: Lighting ray, cascade

**Bond**:
One link in a breakable: two chunks, or a chunk and the world, plus the shared area and the health left.
_Avoid_: Joint, spring

**Breakable**:
One authored object that simulates as a single body until a bond breaks, then swaps in chunk bodies from its fracture asset.
_Avoid_: Mesh collider, destructible mesh

**Chunk**:
One piece of a fracture asset. It is not a body until a hot-swap spawns it.
_Avoid_: Fragment, shard

**Hot-swap**:
Replacing the intact stand-in for a broken branch with that branch's child bodies in the step that breaks the bond.
_Avoid_: Runtime fracture, rebake

**Debris**:
A spawned chunk under the debris volume. It sleeps quickly, it does not collide with other debris, and it can be removed when the debris budget is full.
_Avoid_: Particle, gib

**Fracture asset**:
The cooked `.gfract` file: the chunk hierarchy and the bond graph. The physics step reads it. The step does not cook it.
_Avoid_: Mesh, texture

**Codimation**:
A looping move of one solid through fixed positions.
_Avoid_: Animation clip, tween

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
The light that reaches a surface from an emitter or from another surface. GI lighting terms are defined in [docs/lighting/GLOSSARY.md](docs/lighting/GLOSSARY.md), which is authoritative for lighting names. The radiance-field, radiance-cascade and world-probe entries below are GI v1 terms only.
_Avoid_: Shader, material

**Radiance field** (historical, GI v1):
The light traveling through the scene, split into distance ranges.
_Avoid_: Lightmap, g-buffer

**Radiance cascade** (historical, GI v1, removed 2026-10-12):
One distance range of the radiance field. A near cascade stores more positions, and a far cascade stores more directions.
_Avoid_: Cascade shadow map, lightmap

**World probe** (historical, GI v1; not the GI world radiance cache):
A coarse probe in the world, past the nearer cascades. It carries material-colored light those ranges miss.
_Avoid_: Light probe, reflection probe, lightmap

**Script**:
Rhai code that a game or a mod runs inside the runtime. A script calls engine functions, and it does not call the operating system or Vulkan.
_Avoid_: Python, Lua

**Mod**:
A set of scripts and data that changes a game without a change to the engine.
_Avoid_: Plugin, native library

**System record**:
The living note for one engine system. It states the goal, the intent, the code, the game use, and the limits. It changes when those facts change.
_Avoid_: ADR, spec

**UI element**:
A rectangle in a UI tree. It has a size rule and a look.
_Avoid_: Widget, control

**Screen root**:
A UI element placed on the viewport. It stays when the camera moves.
_Avoid_: HUD, canvas

**World root**:
A UI element placed on a world point. Its screen rectangle follows that point.
_Avoid_: Billboard, sprite

**Asset**:
Data a game holds after a load. An image, a mesh, PCM samples, and a font are assets.
_Avoid_: Resource, content

**Byte source**:
The supplier of bytes for one load. A file path and a memory block are byte sources.
_Avoid_: Stream, backend

**Texture map**:
One image with frame rectangles and named clips for 2D animation.
_Avoid_: Sprite sheet, atlas

**Loading**:
The system that turns byte sources into assets. Loading does not play audio and does not upload to a GPU.
_Avoid_: Asset manager, content pipeline
