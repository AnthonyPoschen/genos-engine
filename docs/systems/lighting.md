# Lighting

The renderer lighting engine composes illumination. `Renderer::draw` passes the lights and the objects. The engine decides which lamp reaches which surface. A game does not pair a lamp with an object first.

## Goal

A player can see one lamp, a shadow where a wall blocks that lamp, and a bounce that takes the color of the hit material. A dark scene with no lamp and no bounce is black.

## Intent

The technique is radiance cascades. The working notes, the paper links, and the budget are in [Lighting notes](../lighting.md). Read that file before you change probe counts, interval length, or the merge rule.

A build does not read the previous frame. There is no lightmap and no ambient fill. The gather is four GPU passes: the lamps, then three bounces. Each bounce reads the previous pass of that build.

A direct ray stops when it hits a wall or a solid. The ray uses those shapes. It is not a physics collision step. A ray that clears the top of a wall still arrives. A bounce ray stops on the first surface in its interval.

Each face is shaded from a point just outside that face. The back of a wall stays dark when the lamp is on the other side. A brighter lamp does not pass through that wall. A lamp above an object colors the floor on every side. The vertical faces of that object stay unlit by that lamp. A lamp on one side does not color the far side.

A lit wall adds light to the floor shadow in front of that wall. The shadow is brighter near the wall than far from the wall. The floor behind that wall stays dark.

Hidden objects and objects outside the view stay in the field when `affects_light` is true. Culling removes them from the picture only.

The march is on the XZ ground plane. World probes sit past the nearer ranges. They carry material color that the nearer ranges miss.

This scene uses one near-probe spacing, 16 cm, across the floor. A miss stays on that probe and continues into the next range. Far and world ranges use more directions. They do not move the ray onto a coarser grid. The floor, the walls, and the solids sample that 16 cm field. There is no coarser surface cell between those objects. The grid stays fixed in the world. A later draw with the same lights and the same occluders reuses that field. An object that is hidden and does not affect light is left out of the gather. The paper links, including arXiv:2408.14425, are in the lighting notes.

A camera move does not rebuild the visible mesh. The draw rebuilds that mesh when the shaded scene changes or the on-screen set changes.

## Code

The GPU compute shader `shaders/light.comp` builds the field. The fragment shader `shaders/scene.frag` shades each pixel. `Renderer::draw` does not bake the lit color on the CPU.

Packed object ranges, colors, and texture ids live in one GPU buffer. An object with no texture stores texture id 0. A later frame does not upload that buffer again when the mesh is unchanged.

The gather runs only when a light or an occluder changes. A normal draw returns before the GPU fence signals. The next scene update can run while that frame is still on the GPU. A readback waits for the fence.

A probe inside a wall or a solid stores no light. A shaded point does not read a probe across a wall.

## Game use

Place lights in the scene script with `light(x, y, z, r, g, b)`. Do not call the field builder from the game loop.

A new occluder is a wall or a solid in the scene. The direct ray and the bounce ray both see it. A mesh or a particle with `affects_light` becomes a stand-in solid in `World::light_scene`.

Fire does not use that stand-in. The stationary flame is one lamp in the same cascade. A wall blocks that lamp. The flame does not light the far side of the wall. The floor beside the flame stays warm with the scene lamp off. A lit particle uses the same shade path as a face: albedo times direct plus bounce. A density puff is still optical depth and in-scatter on the view ray. The probes stay on the ground plane. Light that leaves the floor travels in the cascade until the first wall.

## Limits

The probes are not a 3D volume. A tall gap that the ground march misses can stay dark or can stay bright for the wrong reason.

Screen-space cascades are not the shading path. A second probe grid stepped the opening view down, because those objects are meters from the camera. The picture uses the one 16 cm world field. The paper's heaviest ray counts are not the budget. The budget is in the lighting notes.

Soft penumbras are not implemented. The cascade probes are 16 cm apart. The fragment shader shades each pixel. The shadow edge is the lamp ray. The lamp side of an object is not darkened by a margin around its base.

## Decisions

- [ADR 0008](../adr/0008-lighting-uses-radiance-cascades.md) selects radiance cascades. The engine keeps a finished field until a light or an occluder changes. A new build still does not read the previous frame.
- [Renderer](renderer.md) owns the draw that runs this pass.
