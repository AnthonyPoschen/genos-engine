# Lighting

The renderer lighting engine composes illumination. `Renderer::draw` passes the lights and the objects. The engine decides which lamp reaches which surface. A game does not pair a lamp with an object first.

## Goal

A player can see one lamp, a shadow where a wall blocks that lamp, and a bounce that takes the color of the hit material. A dark scene with no lamp and no bounce is black.

## Intent

The technique is radiance cascades. The working notes, the paper links, and the budget are in [Lighting notes](../lighting.md). Read that file before you change probe counts, interval length, or the merge rule.

A build does not read the previous frame. There is no lightmap and no ambient fill. The gather writes one world-cache band, then two screen passes. The first screen pass stores the lamp on each hit. The second pass stores the lamp that leaves a wide wall. A small solid also stores the screen gather. The pixel reads that screen grid and adds its own lamp ray. The world cascade fills a miss.

A miss merges with the world cache. An average of nearby probes does not stand in for a bounce the gather does not run.

A direct ray stops when it hits a wall or a solid. The ray uses those shapes. It is not a physics collision step. A ray that clears the top of a wall still arrives. A bounce ray stops on the first surface in its interval.

Each face is shaded from a point just outside that face. The back of a wall stays dark when the lamp is on the other side. A brighter lamp does not pass through that wall. A lamp above an object colors the floor on every side. The vertical faces of that object stay unlit by that lamp. A lamp on one side does not color the far side.

A lit wall adds its bounce to the floor in front of that wall through the interval merge. The floor behind that wall stays dark.

Hidden objects and objects outside the view stay in the field when `affects_light` is true. Culling removes them from the picture only.

The march is on the XZ ground plane. World probes sit past the nearer ranges. They carry material color that the nearer ranges miss.

The near probes are a world lattice around the eye. One probe covers the budget of a 16 by 16 pixel tile, with a cap of 80 by 45. The spacing is 0.5 m. Each probe traces 4 m with 16 directions. The last 0.5 m blends into the world cache. A miss reads that cache, which keeps a direction for light behind the camera. A turn leaves the lattice where it was built.

The floor, the walls, and the solids read that same field. An upward face reads probes outside its footprint. A later draw reuses the field until a lamp moves by about a meter, or a lamp color, an occluder, or the light-affecting medium changes. Screen rectangles do not rebuild it. An object that is hidden and does not affect light is left out of the gather. The paper links, including arXiv:2408.14425, are in the lighting notes.

A camera move does not rebuild the visible mesh. The draw rebuilds that mesh when the shaded scene changes or the on-screen set changes.

## Code

The GPU compute shader `shaders/light.comp` builds the field. The fragment shader `shaders/scene.frag` shades each pixel. `Renderer::draw` does not bake the lit color on the CPU.

Packed object ranges, colors, and texture ids live in one GPU buffer. An object with no texture stores texture id 0. A later frame does not upload that buffer again when the mesh is unchanged.

The picture field is a world lattice, one probe per 16 pixels of budget, with 16 directions and a 4 m interval. It rebuilds when a lamp or an occluder changes, or when the grid size changes. A turn does not rebuild it. The pixel reads that lattice by world position. A point outside it reads the world cache. A readback fills every world band before the picture.

A coarse world grid covers the floor at 2.5 m. A lamp or occluder change marks that cache dirty. Each later frame writes one band. A still camera with a clean cache submits no world work. A screen miss reads the cache, including light behind the camera.

A probe inside a wall or a solid stores a distance below zero and is left out of the blend. A hit stores the distance to that hit. The blend skips a probe behind the shaded face. It also skips a probe whose hit is much closer than the shaded point. The blend skips a probe when the segment to it crosses a wall.

## Game use

Place a point lamp with `light(x, y, z, r, g, b)`. Place a directional light with `sun(x, y, z, r, g, b)`. The sun vector is the direction the rays travel. `sun(0, -1, 1, 1, 1, 1)` points down at 45 degrees toward +Z. A color of 1, 1, 1 is the unit lamp. The sun uses the brightness of that lamp at 7 m. Do not call the field builder from the game loop.

A new occluder is a wall or a solid in the scene. The direct ray and the bounce ray both see it. A mesh or a particle with `affects_light` becomes a stand-in solid in `World::light_scene`.

Fire does not use that stand-in. The stationary flame is one lamp in the same cascade. A wall blocks that lamp. The flame does not light the far side of the wall. The floor beside the flame stays warm with the scene lamp off. A lit particle uses the same shade path as a face: albedo times reflectance times direct plus bounce. A density puff is still optical depth and in-scatter on the view ray. The probes stay on the ground plane. Light that leaves the floor travels in the cascade until the first wall.

## Limits

The probes are not a 3D volume. A tall gap that the ground march misses can stay dark or can stay bright for the wrong reason.

The final gather is a screen probe grid. The world cascade fills a miss. The paper's heaviest ray counts are not the budget. The budget is in the lighting notes.

The soft edge of a shadow is the coarser angle step of the next cascade, merged only where β is 1. There is no painted halo around a light square. The fragment shader shades each pixel. The direct shadow edge is the lamp ray. The lamp side of an object is not darkened by a margin around its base.

## Decisions

- [ADR 0008](../adr/0008-lighting-uses-radiance-cascades.md) selects radiance cascades. The engine keeps a finished field until the screen key changes, or a lamp, an occluder, or the light-affecting medium changes. A turn does not rebuild the field. The near probes stay on the world lattice. A new build still does not read the previous frame.
- [Renderer](renderer.md) owns the draw that runs this pass.
