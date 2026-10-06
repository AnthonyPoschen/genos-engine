# Renderer

The renderer owns the Vulkan draw. It turns a world and a camera into one presented frame. Lighting runs inside that draw.

## Goal

A game can submit fixed shapes today, and can submit meshes, particles, and shader draws as those features grow. Objects outside the view are not rasterized. Hidden objects are not rasterized. Both still feed the light field when `affects_light` is true.

## Intent

One Vulkan path serves Linux, Windows, and macOS. macOS uses MoltenVK. The renderer loads `libvulkan.so.1` directly. It does not use `ash`, `wgpu`, or `vulkano`.

`Renderer::draw` passes the lights and the objects to the lighting engine, then uploads the visible triangles, then presents. The caller does not build the field and does not pair lamps with objects. The engine keeps the field while the lights and the occluders stay the same. The engine keeps the visible mesh while the same objects stay on screen.

`World::from_scene` makes one object for the floor, one for each wall, and one for each solid. Extra objects are meshes, particles, or shaders.

`DrawKind` is the set of draws:

- `Fixed` is the floor, a wall, or a solid from the scene.
- `Mesh` is a triangle list plus a column-major pose. Replace the pose each frame to move the mesh.
- `Particles` are points. Each point draws a small vertical quad.
- `Shader` is `ShaderSpace::Mesh` or `ShaderSpace::Screen`. A mesh-space shader can carry a `Displacement` height map. The map builds a grid. A screen-space shader is recorded and is not drawn over the scene yet.

`hidden` skips raster. `in_view` skips raster when the bounds miss the camera frustum. `affects_light` set to false also removes that object from the cascade scene. The floor stays in the cascade scene because the march runs on it.

Each box face and each cylinder side is lit from a point just outside that face. One sample at the center of a wall lights the back face. Do not return to that sample.

The floor, the walls, and the solids use 16 cm cells across this scene. A cast shadow on the floor follows the straight projection of the occluder through the lamp. There is no coarser cell between the objects.

## Code

The crate is `crates/render`, package `genos-render`.

- `Renderer` is in `src/gpu.rs`.
- `World`, `Object`, `DrawKind`, and `in_view` are in `src/world.rs`.
- Triangle build is in `src/mesh.rs`.
- `Lighting` is in `src/lighting.rs`. The field math is in `src/field.rs`.

## Game use

```text
let world = World::from_scene(scene);
renderer.draw(&world, &camera, false)?;
renderer.draw_with_overlay(&world, &camera, &rects, false)?;
```

Each `ScreenRect` is a pixel rectangle. The draw places those rectangles after the world. The overlay does not test depth. The overlay does not write depth.

Set `object.hidden = true` to hide an object and keep its light. Push a `DrawKind::Mesh` and replace `pose` to animate. Push a `DrawKind::Particles` to draw points. Push a `DrawKind::Shader` with a `Displacement` to build a grid from a height map.

Open the renderer with the window display and the window surface. Call `resize` when `WindowEvent::Resized` arrives.

## Limits

There is no skinned clip player. A mesh animates only when the game writes a new pose.

There is no particle simulator. The game supplies the points.

A screen-space shader does not run a custom program yet. Custom SPIR-V is not loaded.

The draw is one forward pass. There is one swapchain and one depth buffer.

## Decisions

- [ADR 0003](../adr/0003-one-vulkan-renderer-moltenvk-on-macos.md) keeps one Vulkan renderer.
- [ADR 0007](../adr/0007-native-calls-go-to-the-system-and-vulkan.md) forbids a renderer framework.
- [Lighting](lighting.md) owns the light pass that this draw calls.
