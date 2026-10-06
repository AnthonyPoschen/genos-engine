# Lighting

The renderer lighting engine composes illumination. `Renderer::draw` passes the lights and the objects. The engine decides which lamp reaches which surface. A game does not pair a lamp with an object first.

## Goal

A player can see one lamp, a shadow where a wall blocks that lamp, and a bounce that takes the color of the hit material. A dark scene with no lamp and no bounce is black.

## Intent

The technique is radiance cascades. The working notes, the paper links, and the budget are in [Lighting notes](../lighting.md). Read that file before you change probe counts, interval length, or the merge rule.

A build does not read the previous frame. There is no lightmap and no ambient fill.

A direct ray stops when it hits a wall or a solid. The ray uses those shapes. It is not a physics collision step. A ray that clears the top of a wall still arrives. A bounce ray stops on the first surface in its interval.

Each face is shaded from a point just outside that face. The back of a wall stays dark when the lamp is on the other side.

Hidden objects and objects outside the view stay in the field when `affects_light` is true. Culling removes them from the picture only.

The march is on the XZ ground plane. World probes sit past the nearer ranges. They carry material color that the nearer ranges miss.

This scene uses one near-probe spacing, 16 cm, across the floor. Far and world ranges are coarser, and they fill rays the near range misses. The floor, the walls, and the solids sample that 16 cm field. There is no coarser surface cell between those objects. The grid stays fixed in the world. A later draw with the same lights and the same occluders reuses that field. An object that is hidden and does not affect light is left out of the gather. The paper links, including arXiv:2408.14425, are in the lighting notes.

A camera move does not rebuild the visible mesh. The draw rebuilds that mesh when the shaded scene changes or the on-screen set changes.

## Code

The pass is `Lighting` in `crates/render/src/lighting.rs`. `Renderer::draw` reuses the baked mesh when the camera moves and the same objects stay on screen. The gather runs only when a light or an occluder changes.

The field math is `build`, `illuminate`, and `sample` in `crates/render/src/field.rs`. The tests in `crates/render/tests/cascades.rs` and `crates/render/tests/lighting_engine.rs` pin that math.

`lit_surface` in `crates/render/src/mesh.rs` combines the direct ray and the bounced sample.

## Game use

Place lights in the scene script with `light(x, y, z, r, g, b)`. Do not call `build` from the game loop. `Renderer::draw` calls it.

A new occluder is a wall or a solid in the scene. The direct ray and the bounce ray both see it. A mesh or a particle with `affects_light` becomes a stand-in solid in `World::light_scene`.

## Limits

The probes are not a 3D volume. A tall gap that the ground march misses can stay dark or can stay bright for the wrong reason.

Screen-space cascades are not the shading path. A second probe grid stepped the opening view down, because those objects are meters from the camera. The picture uses the one 16 cm world field. The paper's heaviest ray counts are not the budget. The budget is in the lighting notes.

Soft penumbras are not implemented. The floor, the walls, and the solids use 16 cm cells across this scene. A cast shadow on the floor is cut along the straight projection of the occluder, so the edge is not a staircase of those cells. The lamp side of an object is not darkened by a margin around its base.

## Decisions

- [ADR 0008](../adr/0008-lighting-uses-radiance-cascades.md) selects radiance cascades. The engine keeps a finished field until a light or an occluder changes. A new build still does not read the previous frame.
- [Renderer](renderer.md) owns the draw that runs this pass.
